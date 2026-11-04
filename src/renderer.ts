import { CELL_STRIDE, CLUSTER_FLAG, type RowView } from "./terminal.js";

/** スクロールバーの幅（CSS ピクセル）。xterm.js 6.0.0 と同じ。 */
export const SCROLLBAR_WIDTH = 14;

/** カーソルが点滅する間隔（ミリ秒）。xterm.js 6.0.0 と同じ。 */
const BLINK_MS = 600;

/** `fit` の計算に使う値。長さはすべて CSS ピクセル。 */
export interface FitInput {
  /** 欄の中身の幅（padding を引いたもの）。 */
  width: number;
  /** 欄の中身の高さ（padding を引いたもの）。 */
  height: number;
  /** セルの幅。 */
  cellWidth: number;
  /** セルの高さ。 */
  cellHeight: number;
  /** スクロールバーの幅。`scrollback` が 0 なら 0。 */
  scrollbarWidth: number;
}

/**
 * 欄の大きさから列と行の数を出す。xterm.js 6.0.0 の FitAddon（`proposeDimensions`）と同じ式:
 * 幅からスクロールバーの幅を引いてセルの幅で割り、切り捨てる。列は 2 以上、行は 1 以上。
 * セルの大きさが分からない（0 以下）、または欄が見えていない（幅か高さが 0 以下）ときは
 * `undefined`。
 */
export function proposeDimensions(
  input: FitInput,
): { cols: number; rows: number } | undefined {
  const { width, height, cellWidth, cellHeight, scrollbarWidth } = input;
  if (!(cellWidth > 0) || !(cellHeight > 0)) return undefined;
  if (!(width > 0) || !(height > 0)) return undefined;
  return {
    cols: Math.max(2, Math.floor((width - scrollbarWidth) / cellWidth)),
    rows: Math.max(1, Math.floor(height / cellHeight)),
  };
}

/** 256 色のうち、16 から 255 の色（6x6x6 の立方体と 24 段階の灰色）。 */
function extendedPaletteColor(index: number): string {
  let r: number, g: number, b: number;
  if (index < 232) {
    const i = index - 16;
    const level = (n: number) => (n === 0 ? 0 : 55 + n * 40);
    r = level(Math.floor(i / 36));
    g = level(Math.floor(i / 6) % 6);
    b = level(i % 6);
  } else {
    r = g = b = 8 + (index - 232) * 10;
  }
  const hex = (n: number) => n.toString(16).padStart(2, "0");
  return `#${hex(r)}${hex(g)}${hex(b)}`;
}

/** 色の語（`RowView.cells` の色の並び）から作る、描画に使う色の表。 */
export class Palette {
  private colors: string[];
  constructor(
    ansi: readonly string[],
    readonly foreground: string,
    readonly background: string,
    readonly cursor: string,
  ) {
    this.colors = [...ansi];
    for (let i = 16; i < 256; i++) this.colors.push(extendedPaletteColor(i));
  }

  /** パレットの `index` 番の色。 */
  get(index: number): string {
    return this.colors[index] ?? this.foreground;
  }

  /**
   * 色の語を CSS の色にする。上位 8 ビットが種類（0 既定、1 パレット、2 RGB）。
   * `bold` のときは、パレットの 0 から 7 を明るい色（8 から 15）にする（xterm.js の既定）。
   */
  resolve(word: number, isBackground: boolean, bold = false): string {
    const kind = word >>> 24;
    const value = word & 0xff_ffff;
    if (kind === 1) return this.get(bold && value < 8 ? value + 8 : value);
    if (kind === 2) {
      return `#${value.toString(16).padStart(6, "0")}`;
    }
    return isBackground ? this.background : this.foreground;
  }
}

/** 描画が `Terminal` から読むもの。 */
export interface RenderSource {
  readonly cols: number;
  readonly rows: number;
  readonly scrollbackLength: number;
  readonly cursor: {
    x: number;
    y: number;
    visible: boolean;
    blinking: boolean;
    shape: "block" | "underline" | "bar";
  };
  readonly isFocused: boolean;
  /** 画面の一番下から何行さかのぼって見ているか。 */
  readonly viewportOffset: number;
  takeDirtyRows(): number[];
  readViewportRow(y: number): RowView;
}

/** 描画の設定。 */
export interface RenderOptions {
  fontFamily: string;
  /** CSS ピクセル。 */
  fontSize: number;
  /** 字の高さに掛ける倍率。 */
  lineHeight: number;
  palette: Palette;
}

/** `CanvasRenderer` が使う Canvas 2D の道具。テストでは入れ替える。 */
export interface RenderEnv {
  devicePixelRatio(): number;
  requestFrame(cb: () => void): number;
  cancelFrame(id: number): void;
}

export const browserEnv: RenderEnv = {
  devicePixelRatio: () => globalThis.devicePixelRatio || 1,
  requestFrame: (cb) => requestAnimationFrame(cb),
  cancelFrame: (id) => cancelAnimationFrame(id),
};

/** セルの大きさ。単位は CSS ピクセル。 */
export interface CellSize {
  width: number;
  height: number;
}

/**
 * Canvas 2D でターミナルの画面を描く。
 *
 * - 座標はすべてデバイスピクセルの整数で扱い、canvas の実際の大きさを
 *   `devicePixelRatio` 倍にする。にじまない。
 * - 描くのは `requestAnimationFrame` でまとめる。変わった行（と、カーソルが動いた行）だけを描く。
 *   表示位置が動いたり、大きさ・フォント・色が変わったときは全部を描く。
 */
export class CanvasRenderer {
  private ctx: CanvasRenderingContext2D;
  private dpr = 1;
  /** デバイスピクセルでのセルの大きさ。 */
  private cellW = 1;
  private cellH = 1;
  /** セルの上端から、ベースラインまでの距離（デバイスピクセル）。 */
  private baseline = 1;
  private descent = 0;
  private frame: number | undefined;
  private needsFull = true;
  private extraRows = new Set<number>();
  private lastCursor: { x: number; y: number } | undefined;
  private blinkOn = true;
  private blinkTimer: ReturnType<typeof setInterval> | undefined;
  private disposed = false;
  private cellSizeListeners = new Set<(size: CellSize) => void>();
  /** 直前の描画で、スクロールバックを見ていたか。 */
  private lastOffset = 0;

  constructor(
    private canvas: HTMLCanvasElement,
    private source: RenderSource,
    private options: RenderOptions,
    private env: RenderEnv = browserEnv,
  ) {
    const ctx = canvas.getContext("2d", { alpha: false });
    if (!ctx) throw new Error("coneko-term: canvas 2d context is unavailable");
    this.ctx = ctx;
    this.measure();
  }

  /** セルの大きさ（CSS ピクセル）。 */
  get cellSize(): CellSize {
    return { width: this.cellW / this.dpr, height: this.cellH / this.dpr };
  }

  /** セルの大きさが変わった（フォントの読み込みや `devicePixelRatio` の変化のあと）。 */
  onCellSizeChange(listener: (size: CellSize) => void): { dispose(): void } {
    this.cellSizeListeners.add(listener);
    return { dispose: () => void this.cellSizeListeners.delete(listener) };
  }

  /** 設定を入れ替えて測り直す。 */
  setOptions(options: RenderOptions): void {
    this.options = options;
    this.measure();
  }

  private font(bold: boolean, italic: boolean): string {
    const { fontFamily, fontSize } = this.options;
    return `${italic ? "italic " : ""}${bold ? "bold " : ""}${fontSize * this.dpr}px ${fontFamily}`;
  }

  /**
   * 字の幅と高さを測る。フォントを読み終える前に測ると代わりのフォントの値になるので、
   * 読み終えたら呼び直す（`Terminal.open` がそうする）。
   */
  measure(): void {
    const ctx = this.ctx;
    this.dpr = this.env.devicePixelRatio();
    const size = this.options.fontSize * this.dpr;
    ctx.font = this.font(false, false);
    ctx.textBaseline = "alphabetic";
    const sample = ctx.measureText("W".repeat(32));
    const m = ctx.measureText("Mg");
    const width = Math.max(1, Math.round(sample.width / 32) || Math.round(size * 0.6));
    const ascent = m.fontBoundingBoxAscent ?? m.actualBoundingBoxAscent ?? size * 0.8;
    const descent = m.fontBoundingBoxDescent ?? m.actualBoundingBoxDescent ?? size * 0.2;
    const charH = Math.max(1, Math.ceil(ascent + descent));
    const height = Math.max(1, Math.floor(charH * this.options.lineHeight));
    const changed = width !== this.cellW || height !== this.cellH;
    this.cellW = width;
    this.cellH = height;
    this.descent = Math.ceil(descent);
    // 字の高さより行が高いぶんは、上下に半分ずつ余白にする
    this.baseline = Math.floor((height - charH) / 2) + Math.ceil(ascent);
    this.applySize();
    this.needsFull = true;
    this.requestRender();
    if (changed) {
      const size = this.cellSize;
      for (const l of [...this.cellSizeListeners]) l(size);
    }
  }

  /** 列と行の数に合わせて canvas の大きさを決める。 */
  private applySize(): void {
    const c = this.canvas;
    const w = this.source.cols * this.cellW;
    const h = this.source.rows * this.cellH;
    if (c.width !== w) c.width = w;
    if (c.height !== h) c.height = h;
    c.style.width = `${w / this.dpr}px`;
    c.style.height = `${h / this.dpr}px`;
    // canvas の大きさを変えると、2D の状態（font など）が初期に戻る
    this.ctx.textBaseline = "alphabetic";
  }

  /** 列と行の数が変わった。 */
  resize(): void {
    this.applySize();
    this.needsFull = true;
    this.requestRender();
  }

  /** 全部を描き直す。 */
  invalidate(): void {
    this.needsFull = true;
    this.requestRender();
  }

  /** 次のフレームで描く。何回呼んでも 1 回にまとめる。 */
  requestRender(): void {
    if (this.frame !== undefined || this.disposed) return;
    this.frame = this.env.requestFrame(() => {
      this.frame = undefined;
      this.render();
    });
  }

  /** 画面に何か起きた（書き込みなど）。カーソルの点滅を最初に戻して描く。 */
  touch(): void {
    this.blinkOn = true;
    if (this.blinkTimer !== undefined) this.restartBlink();
    this.requestRender();
  }

  private restartBlink(): void {
    this.stopBlink();
    this.blinkTimer = setInterval(() => {
      this.blinkOn = !this.blinkOn;
      this.requestRender();
    }, BLINK_MS);
  }

  private stopBlink(): void {
    if (this.blinkTimer !== undefined) clearInterval(this.blinkTimer);
    this.blinkTimer = undefined;
  }

  private updateBlink(): void {
    const { cursor, isFocused: focused } = this.source;
    const want = cursor.blinking && cursor.visible && focused;
    if (want && this.blinkTimer === undefined) this.restartBlink();
    if (!want && this.blinkTimer !== undefined) {
      this.stopBlink();
      this.blinkOn = true;
    }
  }

  /** いま描く。通常は `requestRender` で足りる。 */
  render(): void {
    if (this.disposed) return;
    const src = this.source;
    const dirty = src.takeDirtyRows();
    this.updateBlink();
    const offset = src.viewportOffset;
    const cursor = src.cursor;
    const showCursor = this.cursorShown(offset);
    const rows = new Set<number>();
    let full = this.needsFull || offset !== this.lastOffset;
    // 過去の行を見ているときは、書き込みでずれる行を全部描き直す
    if (offset > 0 && dirty.length > 0) full = true;
    if (!full) {
      for (const y of dirty) rows.add(y);
      for (const y of this.extraRows) rows.add(y);
      if (this.lastCursor) rows.add(this.lastCursor.y);
      if (showCursor || this.lastCursor) rows.add(cursor.y);
    }
    this.extraRows.clear();
    this.needsFull = false;
    this.lastOffset = offset;
    this.lastCursor = showCursor ? { x: cursor.x, y: cursor.y } : undefined;
    if (full) {
      for (let y = 0; y < src.rows; y++) this.drawRow(y, offset);
    } else {
      for (const y of [...rows].sort((a, b) => a - b)) {
        if (y < src.rows) this.drawRow(y, offset);
      }
    }
  }

  private cursorShown(offset: number): boolean {
    const c = this.source.cursor;
    return c.visible && offset === 0;
  }

  private drawRow(y: number, offset: number): void {
    const ctx = this.ctx;
    const { palette } = this.options;
    const src = this.source;
    const row = src.readViewportRow(y);
    const cells = row.cells;
    const cellW = this.cellW;
    const cellH = this.cellH;
    const top = y * cellH;
    const cols = src.cols;

    const cursor = src.cursor;
    const cursorOn =
      offset === 0 && cursor.visible && cursor.y === y && cursor.x < cols;
    const blinkVisible = !cursor.blinking || !src.isFocused || this.blinkOn;
    // フォーカスが無いときは、点滅せず中抜きのブロックにする
    const solidBlock =
      cursorOn && cursor.shape === "block" && src.isFocused && blinkVisible;

    ctx.globalAlpha = 1;
    ctx.fillStyle = palette.background;
    ctx.fillRect(0, top, cols * cellW, cellH);

    const n = Math.min(row.length, cols);
    // 背景。同じ色が続くところは 1 回で塗る
    let runStart = 0;
    let runColor: string | undefined;
    const flush = (end: number) => {
      if (runColor !== undefined && runColor !== palette.background) {
        ctx.fillStyle = runColor;
        ctx.fillRect(runStart * cellW, top, (end - runStart) * cellW, cellH);
      }
    };
    const colors: { fg: string; bg: string; flags: number }[] = new Array(n);
    for (let x = 0; x < n; x++) {
      const o = x * CELL_STRIDE;
      const flags = cells[o + 4];
      const bold = (flags & 1) !== 0;
      let fg = palette.resolve(cells[o + 2], false, bold);
      let bg = palette.resolve(cells[o + 3], true);
      if (flags & 16) [fg, bg] = [bg, fg];
      if (solidBlock && x === cursor.x) {
        bg = palette.cursor;
        fg = palette.background;
      }
      colors[x] = { fg, bg, flags };
      if (cells[o + 1] === 0) continue; // 全角の右半分は、左半分と一緒に塗る
      if (bg !== runColor) {
        flush(x);
        runColor = bg;
        runStart = x;
      }
    }
    flush(n);
    // 全角の右半分（幅 0）が runColor を変えないので、塗りは左半分から続く。
    // 右半分だけ色が違うことは起きない（同じ属性で書く）。

    let lastFont = "";
    for (let x = 0; x < n; x++) {
      const o = x * CELL_STRIDE;
      const width = cells[o + 1];
      if (width === 0) continue;
      const { fg, flags } = colors[x];
      if (flags & 32) continue; // 不可視
      const cp = cells[o];
      const ul = (flags >>> 8) & 7;
      const strike = (flags & 64) !== 0;
      const hasText = cp !== 0 && cp !== 0x20;
      if (!hasText && ul === 0 && !strike) continue;
      const left = x * cellW;
      ctx.globalAlpha = flags & 2 ? 0.5 : 1;
      if (hasText) {
        const font = this.font((flags & 1) !== 0, (flags & 4) !== 0);
        if (font !== lastFont) {
          ctx.font = font;
          lastFont = font;
        }
        ctx.fillStyle = fg;
        const text =
          cp & CLUSTER_FLAG
            ? row.cluster(cp & ~CLUSTER_FLAG)
            : String.fromCodePoint(cp);
        ctx.fillText(text, left, top + this.baseline);
      }
      const spanW = width * cellW;
      if (ul !== 0) {
        const word = cells[o + 5];
        const color = word >>> 24 === 0 ? fg : palette.resolve(word, false);
        this.drawUnderline(ul, color, left, top, spanW);
      }
      if (strike) {
        ctx.fillStyle = fg;
        const t = this.lineThickness();
        ctx.fillRect(left, top + this.baseline - Math.round(this.baseline * 0.35), spanW, t);
      }
    }
    ctx.globalAlpha = 1;

    if (cursorOn && blinkVisible) this.drawCursorShape(cursor, cells, top);
  }

  private lineThickness(): number {
    return Math.max(1, Math.round(this.dpr));
  }

  private drawUnderline(
    style: number,
    color: string,
    left: number,
    top: number,
    width: number,
  ): void {
    const ctx = this.ctx;
    const t = this.lineThickness();
    ctx.fillStyle = color;
    const bottom = top + this.cellH;
    // ベースラインの少し下。セルに収まるようにする
    const y = Math.min(bottom - t, top + this.baseline + Math.max(1, Math.round(this.descent / 3)));
    switch (style) {
      case 1:
        ctx.fillRect(left, y, width, t);
        break;
      case 2: {
        const y1 = Math.max(top, y - t - 1);
        ctx.fillRect(left, y1, width, t);
        ctx.fillRect(left, Math.min(bottom - t, y1 + t + 1), width, t);
        break;
      }
      case 3: {
        // 波線: 1 セルで 1 周期
        ctx.strokeStyle = color;
        ctx.lineWidth = t;
        const amp = Math.max(1, Math.min(this.descent / 2, (bottom - y) / 2));
        const mid = Math.min(bottom - t - amp, y);
        ctx.beginPath();
        const step = Math.max(2, width / 8);
        for (let px = 0; px <= width; px += step) {
          const py = mid + amp * Math.sin((px / this.cellW) * Math.PI * 2);
          if (px === 0) ctx.moveTo(left + px, py);
          else ctx.lineTo(left + px, py);
        }
        ctx.stroke();
        break;
      }
      case 4: {
        // 点線
        const dot = t;
        for (let px = 0; px < width; px += dot * 2) {
          ctx.fillRect(left + px, y, Math.min(dot, width - px), t);
        }
        break;
      }
      case 5: {
        // 破線
        const dash = Math.max(2, Math.round(this.cellW / 2));
        for (let px = 0; px < width; px += dash * 2) {
          ctx.fillRect(left + px, y, Math.min(dash, width - px), t);
        }
        break;
      }
      default:
        ctx.fillRect(left, y, width, t);
    }
  }

  /** ブロックで塗る場合は字ごと `drawRow` が塗るので、ここは中抜きと下線・縦棒。 */
  private drawCursorShape(
    cursor: RenderSource["cursor"],
    cells: Uint32Array,
    top: number,
  ): void {
    const ctx = this.ctx;
    const focused = this.source.isFocused;
    const color = this.options.palette.cursor;
    const x = cursor.x;
    const width = Math.max(1, cells[x * CELL_STRIDE + 1] || 1);
    const left = x * this.cellW;
    const w = width * this.cellW;
    const t = this.lineThickness();
    ctx.fillStyle = color;
    ctx.strokeStyle = color;
    if (!focused) {
      // 中抜きのブロック
      ctx.lineWidth = t;
      ctx.strokeRect(left + t / 2, top + t / 2, w - t, this.cellH - t);
    } else if (cursor.shape === "underline") {
      ctx.fillRect(left, top + this.cellH - t, w, t);
    } else if (cursor.shape === "bar") {
      ctx.fillRect(left, top, t, this.cellH);
    }
    // フォーカスがあるブロックは drawRow で塗り済み
  }

  /** 後始末。 */
  dispose(): void {
    this.disposed = true;
    this.stopBlink();
    if (this.frame !== undefined) this.env.cancelFrame(this.frame);
    this.frame = undefined;
    this.cellSizeListeners.clear();
  }
}

/** スクロールバー。DOM で作る。見た目は xterm.js 6.0.0（VS Code のスクロールバー）に近づける。 */
export class Scrollbar {
  readonly track: HTMLElement;
  private slider: HTMLElement;
  private cleanups: (() => void)[] = [];
  private dragging = false;

  constructor(
    host: HTMLElement,
    private api: {
      /** 全部の行の数（スクロールバック + 画面）。 */
      total(): number;
      /** 画面の行の数。 */
      rows(): number;
      /** 一番上に見えている行（0 が一番古い）。 */
      viewportY(): number;
      scrollToLine(line: number): void;
      scrollLines(n: number): void;
    },
  ) {
    const doc = host.ownerDocument;
    this.track = doc.createElement("div");
    this.track.className = "coneko-term-scrollbar";
    Object.assign(this.track.style, {
      position: "absolute",
      top: "0",
      right: "0",
      width: `${SCROLLBAR_WIDTH}px`,
      display: "none",
      userSelect: "none",
    });
    this.slider = doc.createElement("div");
    this.slider.className = "coneko-term-scrollbar-slider";
    Object.assign(this.slider.style, {
      position: "absolute",
      left: "0",
      width: "100%",
      background: "rgba(121, 121, 121, 0.4)",
    });
    this.track.appendChild(this.slider);
    host.appendChild(this.track);
    this.bind();
  }

  private bind(): void {
    const slider = this.slider;
    const track = this.track;
    const setBg = (v: string) => (slider.style.background = v);
    const onEnter = () => !this.dragging && setBg("rgba(100, 100, 100, 0.7)");
    const onLeave = () => !this.dragging && setBg("rgba(121, 121, 121, 0.4)");
    slider.addEventListener("pointerenter", onEnter);
    slider.addEventListener("pointerleave", onLeave);
    const onDown = (e: PointerEvent) => {
      if (e.button !== 0) return;
      e.preventDefault();
      if (e.target === slider) {
        this.startDrag(e);
      } else {
        // トラックを押したら、その向きに 1 画面ぶん動く
        const rect = slider.getBoundingClientRect();
        this.api.scrollLines(e.clientY < rect.top ? -this.api.rows() : this.api.rows());
      }
    };
    track.addEventListener("pointerdown", onDown);
    this.cleanups.push(() => {
      slider.removeEventListener("pointerenter", onEnter);
      slider.removeEventListener("pointerleave", onLeave);
      track.removeEventListener("pointerdown", onDown);
    });
  }

  private startDrag(e: PointerEvent): void {
    const slider = this.slider;
    this.dragging = true;
    slider.style.background = "rgba(121, 121, 121, 0.6)";
    const startY = e.clientY;
    const startLine = this.api.viewportY();
    const trackH = this.track.clientHeight;
    const sliderH = slider.offsetHeight;
    const maxLine = this.api.total() - this.api.rows();
    const range = Math.max(1, trackH - sliderH);
    const doc = this.track.ownerDocument;
    const move = (ev: PointerEvent) => {
      const dy = ev.clientY - startY;
      this.api.scrollToLine(Math.round(startLine + (dy / range) * maxLine));
    };
    const up = () => {
      this.dragging = false;
      slider.style.background = "rgba(121, 121, 121, 0.4)";
      doc.removeEventListener("pointermove", move);
      doc.removeEventListener("pointerup", up);
      doc.removeEventListener("pointercancel", up);
    };
    doc.addEventListener("pointermove", move);
    doc.addEventListener("pointerup", up);
    doc.addEventListener("pointercancel", up);
    this.cleanups.push(up);
  }

  /** 高さ（CSS ピクセル）と中身に合わせて、つまみの位置と大きさを決める。 */
  update(heightPx: number, enabled: boolean): void {
    const total = this.api.total();
    const rows = this.api.rows();
    const scrollable = total - rows;
    if (!enabled || scrollable <= 0 || heightPx <= 0) {
      this.track.style.display = "none";
      return;
    }
    this.track.style.display = "block";
    this.track.style.height = `${heightPx}px`;
    const sliderH = Math.min(heightPx, Math.max(20, Math.round((heightPx * rows) / total)));
    const pos = Math.round(((heightPx - sliderH) * this.api.viewportY()) / scrollable);
    this.slider.style.height = `${sliderH}px`;
    this.slider.style.top = `${pos}px`;
  }

  dispose(): void {
    for (const c of this.cleanups) c();
    this.cleanups.length = 0;
    this.track.remove();
  }
}
