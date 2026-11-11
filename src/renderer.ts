import {
  CELL_STRIDE,
  CLUSTER_FLAG,
  type RowView,
  type SizedInfo,
} from "./terminal.js";
import {
  ImageCache,
  Z_UNDER_BACKGROUND,
  browserImageDecoder,
  parsePlacements,
  type DrawableImage,
  type ImageDecoder,
  type ImagePlacement,
  type ImageSource,
} from "./images.js";

/** スクロールバーの幅の既定（CSS ピクセル）。xterm.js 6.0.0 と同じ。`TerminalOptions.scrollbarWidth` で変えられる。 */
export const SCROLLBAR_WIDTH = 14;

/**
 * 設定から使うスクロールバーの幅（CSS ピクセル）を決める。
 * 未指定・負の数・数でない値は既定（`SCROLLBAR_WIDTH`）、`scrollback` が 0 のときは 0。
 */
export function resolveScrollbarWidth(options: {
  scrollback?: number;
  scrollbarWidth?: unknown;
}): number {
  if (options.scrollback === 0) return 0;
  const w = options.scrollbarWidth;
  return typeof w === "number" && Number.isFinite(w) && w >= 0
    ? w
    : SCROLLBAR_WIDTH;
}

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

/**
 * セルの幅（デバイスピクセル）。字の幅の実測値を切り上げる。丸めると、たとえば
 * 13.2px が 13px になって字が詰まる。実測できないとき（0 や NaN）は文字の大きさの 0.6 倍。
 */
export function cellWidthFor(measured: number, fontSize: number): number {
  const w = Math.ceil(measured - 1e-6) || Math.round(fontSize * 0.6);
  return Math.max(1, w);
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
  /** `ansi` は先頭の 16 色か、256 色すべて。 */
  constructor(
    ansi: readonly string[],
    readonly foreground: string,
    readonly background: string,
    readonly cursor: string,
    /** 選択範囲の色。半透明でよい（CSS の色）。 */
    readonly selection: string = "#ffffff4d",
    /** ブロックカーソルの下の字の色。省略したら背景の色。 */
    readonly cursorText: string = background,
  ) {
    this.colors = [...ansi];
    for (let i = ansi.length; i < 256; i++) this.colors.push(extendedPaletteColor(i));
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

/** 描画が `Terminal` から読むもの。画像を描くには、`ImageSource` の 3 つと `readPlacements` も要る。 */
export interface RenderSource extends Partial<ImageSource> {
  /** 列の数。 */
  readonly cols: number;
  /** 行の数。 */
  readonly rows: number;
  /** スクロールバックの行数。 */
  readonly scrollbackLength: number;
  /** カーソルの状態。 */
  readonly cursor: {
    x: number;
    y: number;
    visible: boolean;
    blinking: boolean;
    shape: "block" | "underline" | "bar";
  };
  /** フォーカスがあるか。 */
  readonly isFocused: boolean;
  /** 画面の一番下から何行さかのぼって見ているか。 */
  readonly viewportOffset: number;
  /** 変わった行を返し、印を消す。 */
  takeDirtyRows(): number[];
  /** 表示位置を考えた、いま見えている `y` 行目の選択範囲 `[開始の列, 終わりの列)`。選択が無ければ `undefined`。 */
  selectionSpan?(y: number): [number, number] | undefined;
  /** いま見えている `y` 行目の URL の区間 `[開始の列, 終わりの列)`。波線を引く。 */
  urlSpans?(y: number): [number, number][];
  /** 表示位置を考えた、いま見えている `y` 行目。 */
  readViewportRow(y: number): RowView;
  /** 画像の配置の並び（`WasmTerminal.placements` と同じ）。 */
  readPlacements?(): ArrayLike<number>;
  /** 追加のカーソルの `[x, y, 形]` の並び（`WasmTerminal.extra_cursors` と同じ）。 */
  readExtraCursors?(): ArrayLike<number>;
  /** 追加のカーソルの色の語 `[字の色, カーソルの色]`（`WasmTerminal.extra_cursor_colors` と同じ）。 */
  readExtraCursorColors?(): ArrayLike<number>;
}

/** 描画の設定。 */
export interface RenderOptions {
  /** CSS の `font-family`。 */
  fontFamily: string;
  /** CSS ピクセル。 */
  fontSize: number;
  /** 字の高さに掛ける倍率。 */
  lineHeight: number;
  /** 色の表。 */
  palette: Palette;
}

/** `CanvasRenderer` が使う Canvas 2D の道具。テストでは入れ替える。 */
export interface RenderEnv {
  /** 画面の `devicePixelRatio`。 */
  devicePixelRatio(): number;
  /** 次のフレームで `cb` を呼ぶよう頼む（`requestAnimationFrame`）。 */
  requestFrame(cb: () => void): number;
  /** `requestFrame` を取り消す。 */
  cancelFrame(id: number): void;
  /** 画像を読み解く道具。省くとブラウザーの機能を使う。 */
  imageDecoder?: ImageDecoder;
  /** いまの時刻（ミリ秒）。アニメーションのコマを進めるのに使う。省くと `performance.now()`。 */
  now?(): number;
}

/** ブラウザーの `devicePixelRatio` と `requestAnimationFrame` を使う、既定の `RenderEnv`。 */
export const browserEnv: RenderEnv = {
  devicePixelRatio: () => globalThis.devicePixelRatio || 1,
  requestFrame: (cb) => requestAnimationFrame(cb),
  cancelFrame: (id) => cancelAnimationFrame(id),
};

/** 描く画像と、その読み解き済みの中身。 */
interface PlacedBitmap {
  p: ImagePlacement;
  image: DrawableImage;
}

/** 画像を重ねる層。背景色の下、字の下（背景色の上）、字の上。 */
interface ImageLayers {
  under: PlacedBitmap[];
  below: PlacedBitmap[];
  above: PlacedBitmap[];
}

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
  /** いま描く追加のカーソル。行ごと。 */
  private extra = new Map<number, { x: number; shape: number }[]>();
  /** 追加のカーソルの色の語 `[字の色, カーソルの色]`。 */
  private extraColors: ArrayLike<number> = [0, 0];
  /** 直前の描画で追加のカーソルがあった行。消えたときに描き直す。 */
  private lastExtraRows = new Set<number>();
  private blinkOn = true;
  private blinkTimer: ReturnType<typeof setInterval> | undefined;
  private disposed = false;
  private cellSizeListeners = new Set<(size: CellSize) => void>();
  /** 直前の描画で、スクロールバックを見ていたか。 */
  private lastOffset = 0;
  private imageCache?: ImageCache;
  /** 今のフレームで描く画像（読み解き済みのもの）。層ごと、`z` と ID の順。 */
  private layers: ImageLayers = { under: [], below: [], above: [] };
  /** 直前のフレームで描いた画像の印。変わったら全部を描き直す。 */
  private lastImageKey = "";
  /** 直前のフレームで、回している画像があったか（あれば次のフレームも描く）。 */
  private animating = false;

  constructor(
    private canvas: HTMLCanvasElement,
    private source: RenderSource,
    private options: RenderOptions,
    private env: RenderEnv = browserEnv,
  ) {
    // alpha: false の canvas は、macOS の WebKit で字が太く描かれる（見立て。実機では未確認）。背景は行ごとに
    // 不透明に塗るので、透明を許す canvas でも背景の見た目は変わらない。
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("coneko-term: canvas 2d context is unavailable");
    this.ctx = ctx;
    this.measure();
  }

  /** セルの大きさ（CSS ピクセル）。 */
  get cellSize(): CellSize {
    return { width: this.cellW / this.dpr, height: this.cellH / this.dpr };
  }

  /** デバイスピクセルでのセルの大きさ。 */
  get deviceCellSize(): CellSize {
    return { width: this.cellW, height: this.cellH };
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

  private font(bold: boolean, italic: boolean, scale = 1): string {
    const { fontFamily, fontSize } = this.options;
    return `${italic ? "italic " : ""}${bold ? "bold " : ""}${fontSize * this.dpr * scale}px ${fontFamily}`;
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
    const width = cellWidthFor(sample.width / 32, size);
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

  /** 行 `ys`（見えている画面の行番号）を次のフレームで描き直す。 */
  invalidateRows(ys: Iterable<number>): void {
    for (const y of ys) this.extraRows.add(y);
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
    // 追加のカーソルは、主のカーソルが見えなくても、同じ点滅に従う
    const want =
      cursor.blinking && (cursor.visible || this.extra.size > 0) && focused;
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
    const offset = src.viewportOffset;
    this.collectExtraCursors(offset);
    this.updateBlink();
    const cursor = src.cursor;
    const showCursor = this.cursorShown(offset);
    const rows = new Set<number>();
    let full = this.needsFull || offset !== this.lastOffset;
    // 画像の置き場所や中身が変わったら、全部を描き直す
    this.animating = false;
    if (this.collectImages(offset)) full = true;
    // 過去の行を見ているときは、書き込みでずれる行を全部描き直す
    if (offset > 0 && dirty.length > 0) full = true;
    if (!full) {
      for (const y of dirty) rows.add(y);
      for (const y of this.extraRows) rows.add(y);
      if (this.lastCursor) rows.add(this.lastCursor.y);
      if (showCursor || this.lastCursor) rows.add(cursor.y);
      // 追加のカーソルは、点滅と主の形に従うので、いる行を毎回描く
      for (const y of this.extra.keys()) rows.add(y);
      for (const y of this.lastExtraRows) rows.add(y);
    }
    this.extraRows.clear();
    this.needsFull = false;
    this.lastOffset = offset;
    this.lastCursor = showCursor ? { x: cursor.x, y: cursor.y } : undefined;
    this.lastExtraRows = new Set(this.extra.keys());
    if (full) {
      for (let y = 0; y < src.rows; y++) this.drawRow(y, offset);
    } else {
      for (const y of [...rows].sort((a, b) => a - b)) {
        if (y < src.rows) this.drawRow(y, offset);
      }
    }
    // アニメーションを回している間は、止まるまで毎フレーム時刻を確かめる
    if (this.animating) this.requestRender();
  }

  /**
   * 見えている画像の配置を集めて層に分ける。前のフレームから変わったら `true`。
   * 読み解きが終わっていない画像は、終わるまで描かない（終わったら描き直す）。
   */
  private collectImages(offset: number): boolean {
    const src = this.source;
    const flat = src.readPlacements?.();
    const layers: ImageLayers = { under: [], below: [], above: [] };
    let key = "";
    if (flat && flat.length > 0) {
      const cache = (this.imageCache ??= new ImageCache(
        src as ImageSource,
        this.env.imageDecoder ?? browserImageDecoder,
        () => this.invalidate(),
      ));
      cache.sync();
      const placed = parsePlacements(flat)
        .filter((p) => p.row + offset < src.rows && p.row + offset + p.rows > 0)
        .sort((a, b) => a.z - b.z || a.imageId - b.imageId);
      const ticked = new Set<number>();
      const now = (this.env.now ?? (() => performance.now()))();
      for (const p of placed) {
        if (!ticked.has(p.imageId)) {
          ticked.add(p.imageId);
          if (cache.tick(p.imageId, now)) this.animating = true;
        }
        const image = cache.get(p.imageId);
        if (!image) continue;
        const layer =
          p.z < Z_UNDER_BACKGROUND ? layers.under : p.z < 0 ? layers.below : layers.above;
        layer.push({ p, image });
        key += `${p.imageId}:${p.row}:${p.col}:${p.srcX}:${p.srcY}:${p.srcW}:${p.srcH}:${p.offsetX}:${p.offsetY}:${p.cols}:${p.rows}:${p.z}:${p.native}:${p.destW}:${p.destH}@${cache.version(p.imageId)};`;
      }
    } else {
      this.imageCache?.sync();
    }
    this.layers = layers;
    const changed = key !== this.lastImageKey;
    this.lastImageKey = key;
    return changed;
  }

  /** 行 `y` にかかる画像を、`top` から始まる 1 行ぶんだけ描く。 */
  private drawImages(list: PlacedBitmap[], y: number, offset: number): void {
    if (list.length === 0) return;
    const ctx = this.ctx;
    const top = y * this.cellH;
    const bottom = top + this.cellH;
    const limit = this.source.cols * this.cellW;
    ctx.globalAlpha = 1;
    for (const { p, image } of list) {
      const srcX = p.srcX;
      const srcY = p.srcY;
      const srcW = p.srcW > 0 ? p.srcW : image.width - srcX;
      const srcH = p.srcH > 0 ? p.srcH : image.height - srcY;
      if (srcW <= 0 || srcH <= 0) continue;
      // 隣の行と継ぎ目が出ないよう、置く場所は整数のデバイスピクセルにする
      const dx = Math.round(p.col * this.cellW + p.offsetX);
      const dy = Math.round((p.row + offset) * this.cellH + p.offsetY);
      const dw = Math.round(
        p.destW > 0 ? p.destW : p.native ? srcW : p.cols * this.cellW,
      );
      const dh = Math.round(
        p.destH > 0 ? p.destH : p.native ? srcH : p.rows * this.cellH,
      );
      if (dw <= 0 || dh <= 0) continue;
      const x0 = Math.max(dx, 0);
      const x1 = Math.min(dx + dw, limit);
      const y0 = Math.max(dy, top);
      const y1 = Math.min(dy + dh, bottom);
      if (x1 <= x0 || y1 <= y0) continue;
      ctx.drawImage(
        image as unknown as CanvasImageSource,
        srcX + ((x0 - dx) * srcW) / dw,
        srcY + ((y0 - dy) * srcH) / dh,
        ((x1 - x0) * srcW) / dw,
        ((y1 - y0) * srcH) / dh,
        x0,
        y0,
        x1 - x0,
        y1 - y0,
      );
    }
  }

  /** 追加のカーソルを、いま見える行ごとに集める。さかのぼって見ているときは出さない。 */
  private collectExtraCursors(offset: number): void {
    const src = this.source;
    this.extra.clear();
    this.extraColors = src.readExtraCursorColors?.() ?? [0, 0];
    const flat = offset === 0 ? src.readExtraCursors?.() : undefined;
    if (!flat) return;
    for (let i = 0; i + 2 < flat.length; i += 3) {
      const x = flat[i];
      const y = flat[i + 1];
      if (x >= src.cols || y >= src.rows) continue;
      const list = this.extra.get(y);
      if (list) list.push({ x, shape: flat[i + 2] });
      else this.extra.set(y, [{ x, shape: flat[i + 2] }]);
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
    // 追加のカーソル。主のカーソルの点滅に従う（主が見えなくても出る）
    const extras = blinkVisible ? this.extra.get(y) : undefined;
    const solidExtra = new Set<number>();
    if (extras && src.isFocused) {
      for (const e of extras) {
        if (this.extraShape(e.shape) === "block") solidExtra.add(e.x);
      }
    }

    ctx.globalAlpha = 1;
    ctx.fillStyle = palette.background;
    ctx.fillRect(0, top, cols * cellW, cellH);

    // 下から順に: 背景色の下の画像、背景色、字の下の画像、選択範囲、字、字の上の画像
    this.drawImages(this.layers.under, y, offset);
    const n = Math.min(row.length, cols);
    const colors = this.resolveRowColors(
      row.cells,
      n,
      solidBlock ? cursor.x : -1,
      solidExtra,
    );
    this.fillBackgroundRuns(row.cells, colors, n, top);
    this.drawImages(this.layers.below, y, offset);
    this.fillSelection(y, top);
    this.drawGlyphsAndLines(row, colors, n, top, this.source.urlSpans?.(y));
    ctx.globalAlpha = 1;
    this.drawImages(this.layers.above, y, offset);

    if (cursorOn && blinkVisible) this.drawCursorShape(cursor, row.cells, top);
    if (extras) {
      for (const e of extras) {
        if (e.x >= cols || solidExtra.has(e.x)) continue;
        this.drawExtraCursorShape(e, row.cells, colors, top);
      }
    }
  }

  /** 追加のカーソルの形（主のカーソルに従うものは、いまの主の形にする）。 */
  private extraShape(shape: number): "block" | "underline" | "bar" {
    return (
      (["block", "underline", "bar"] as const)[shape] ??
      this.source.cursor.shape
    );
  }

  /** 追加のカーソルのブロックの、字の色と背景色。`fg`・`bg` はそのセルの色。 */
  private extraBlockColors(fg: string, bg: string): [string, string] {
    const { palette } = this.options;
    const textWord = this.extraColors[0];
    const cursorWord = this.extraColors[1];
    const cursorKind = cursorWord >>> 24;
    const textKind = textWord >>> 24;
    // カーソルの色が反転のときは、字の色の指定を使わない
    if (cursorKind === 3) return [bg, fg];
    const cursorBg =
      cursorKind === 0 ? palette.cursor : palette.resolve(cursorWord, true);
    let text = palette.cursorText;
    if (textKind === 3) text = bg;
    else if (textKind !== 0) text = palette.resolve(textWord, false);
    return [text, cursorBg];
  }

  /** 追加のカーソルのうち、ブロックで塗らないもの（中抜き・下線・縦棒）を描く。 */
  private drawExtraCursorShape(
    e: { x: number; shape: number },
    cells: Uint32Array,
    colors: { fg: string }[],
    top: number,
  ): void {
    const ctx = this.ctx;
    const { palette } = this.options;
    const word = this.extraColors[1];
    const kind = word >>> 24;
    const color =
      kind === 0
        ? palette.cursor
        : kind === 3
          ? (colors[e.x]?.fg ?? palette.cursor)
          : palette.resolve(word, true);
    const width = Math.max(1, cells[e.x * CELL_STRIDE + 1] || 1);
    const left = e.x * this.cellW;
    const w = width * this.cellW;
    const t = this.lineThickness();
    const shape = this.extraShape(e.shape);
    ctx.fillStyle = color;
    ctx.strokeStyle = color;
    if (!this.source.isFocused) {
      // 中抜きのブロック
      ctx.lineWidth = t;
      ctx.strokeRect(left + t / 2, top + t / 2, w - t, this.cellH - t);
    } else if (shape === "underline") {
      ctx.fillRect(left, top + this.cellH - t, w, t);
    } else if (shape === "bar") {
      ctx.fillRect(left, top, t, this.cellH);
    }
  }

  /**
   * 行のセルごとの前景色・背景色・属性を決める（反転を適用し、太字なら明るい色にする）。
   * `blockX` 列は、ブロックカーソルの色にする（なければ -1）。`extraBlocks` の列は、
   * 追加のカーソルのブロックの色にする。
   */
  private resolveRowColors(
    cells: Uint32Array,
    n: number,
    blockX: number,
    extraBlocks: ReadonlySet<number> = new Set(),
  ): { fg: string; bg: string; flags: number }[] {
    const { palette } = this.options;
    const colors: { fg: string; bg: string; flags: number }[] = new Array(n);
    for (let x = 0; x < n; x++) {
      const o = x * CELL_STRIDE;
      const flags = cells[o + 4];
      let fg = palette.resolve(cells[o + 2], false, (flags & 1) !== 0);
      let bg = palette.resolve(cells[o + 3], true);
      if (flags & 16) [fg, bg] = [bg, fg];
      if (x === blockX) {
        bg = palette.cursor;
        fg = palette.cursorText;
      } else if (extraBlocks.has(x)) {
        [fg, bg] = this.extraBlockColors(fg, bg);
      }
      colors[x] = { fg, bg, flags };
    }
    return colors;
  }

  /** 背景を塗る。同じ色が続くところは 1 回で塗る。 */
  private fillBackgroundRuns(
    cells: Uint32Array,
    colors: { bg: string }[],
    n: number,
    top: number,
  ): void {
    const ctx = this.ctx;
    const defaultBg = this.options.palette.background;
    let runStart = 0;
    let runColor: string | undefined;
    const flush = (end: number) => {
      if (runColor !== undefined && runColor !== defaultBg) {
        ctx.fillStyle = runColor;
        ctx.fillRect(
          runStart * this.cellW,
          top,
          (end - runStart) * this.cellW,
          this.cellH,
        );
      }
    };
    for (let x = 0; x < n; x++) {
      // 全角の右半分（幅 0）は、左半分と一緒に塗る
      if (cells[x * CELL_STRIDE + 1] === 0) continue;
      const bg = colors[x].bg;
      if (bg !== runColor) {
        flush(x);
        runColor = bg;
        runStart = x;
      }
    }
    flush(n);
  }

  /** 選択範囲を、背景の上に半透明で重ねる。字はこの上に描く。 */
  private fillSelection(y: number, top: number): void {
    const span = this.source.selectionSpan?.(y);
    if (!span) return;
    const [c0, c1] = span;
    const end = Math.min(c1, this.source.cols);
    if (end <= c0) return;
    const ctx = this.ctx;
    ctx.fillStyle = this.options.palette.selection;
    ctx.fillRect(c0 * this.cellW, top, (end - c0) * this.cellW, this.cellH);
  }

  /** 字と、下線・取り消し線を描く。 */
  private drawGlyphsAndLines(
    row: RowView,
    colors: { fg: string; flags: number }[],
    n: number,
    top: number,
    urls?: [number, number][],
  ): void {
    const ctx = this.ctx;
    const cells = row.cells;
    let lastFont = "";
    for (let x = 0; x < n; x++) {
      const o = x * CELL_STRIDE;
      const width = cells[o + 1];
      if (width === 0) continue;
      const { fg, flags } = colors[x];
      if (flags & 32) continue; // 不可視
      const mark = cells[o + 6];
      // 複数のセルにまたがる字は、この行にかかる四角の一番左のセルで、四角ごと描く
      if ((mark & 0xffff) !== 0 && ((mark >>> 16) & 0xff) === 0) {
        ctx.globalAlpha = flags & 2 ? 0.5 : 1;
        this.drawSized(row.sized((mark & 0xffff) - 1), mark >>> 24, x, top, fg);
      }
      const cp = cells[o];
      let ul = (flags >>> 8) & 7;
      // URL には波線を引く（下線の指定があるセルはそのまま）
      if (ul === 0 && urls?.some(([a, b]) => x >= a && x < b)) ul = 3;
      const strike = (flags & 64) !== 0;
      const hasText = cp !== 0 && cp !== 0x20;
      if (!hasText && ul === 0 && !strike) continue;
      const left = x * this.cellW;
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
        // Unicode placeholder の字は描かない（画像が代わりに描かれる）
        if (text.codePointAt(0) !== 0x10eeee) {
          ctx.fillText(text, left, top + this.baseline);
        }
      }
      const spanW = width * this.cellW;
      if (ul !== 0) {
        const word = cells[o + 5];
        const color =
          word >>> 24 === 0 ? fg : this.options.palette.resolve(word, false);
        this.drawUnderline(ul, color, left, top, spanW);
      }
      if (strike) {
        ctx.fillStyle = fg;
        ctx.fillRect(
          left,
          top + this.baseline - Math.round(this.baseline * 0.35),
          spanW,
          this.lineThickness(),
        );
      }
    }
  }

  /**
   * 複数のセルにまたがる字（OSC 66）を描く。`dy` は、いまの行が四角の上から何行目か。
   * 描くのは四角のうち、いまの行にかかる横長の帯だけ（行ごとに描き直すため）。
   * 分数の大きさのときだけ、`v`・`h` の寄せ方で四角の中に置く。
   */
  private drawSized(
    info: SizedInfo,
    dy: number,
    x: number,
    top: number,
    fill: string,
  ): void {
    const frac =
      info.denominator > info.numerator ? info.numerator / info.denominator : 1;
    const k = info.scale * frac;
    if (k <= 0) return;
    const ctx = this.ctx;
    const left = x * this.cellW;
    const blockW = info.cols * this.cellW;
    const blockH = info.scale * this.cellH;
    const blockTop = top - dy * this.cellH;
    ctx.save();
    ctx.beginPath();
    ctx.rect(left, top, blockW, this.cellH);
    ctx.clip();
    ctx.font = this.font(false, false, k);
    ctx.fillStyle = fill;
    const align = (a: number, space: number, size: number): number =>
      a === 1 ? space - size : a === 2 ? (space - size) / 2 : 0;
    const textW = ctx.measureText(info.text).width;
    const x0 =
      left + (frac < 1 ? align(info.horizontal, blockW, textW) : 0);
    const y0 =
      blockTop + (frac < 1 ? align(info.vertical, blockH, this.cellH * k) : 0);
    ctx.fillText(info.text, x0, y0 + this.baseline * k);
    ctx.restore();
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
    this.imageCache?.dispose();
    this.imageCache = undefined;
  }
}

/** スクロールバー。DOM で作る。見た目は xterm.js 6.0.0（VS Code のスクロールバー）に近づける。 */
export class Scrollbar {
  readonly track: HTMLElement;
  private slider: HTMLElement;
  private cleanups: (() => void)[] = [];
  private dragging = false;
  /** いまのドラッグを終える関数。 */
  private endDrag?: () => void;

  constructor(
    host: HTMLElement,
    private api: {
      /** 幅（CSS ピクセル）。0 なら出さない。 */
      width: number;
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
      width: `${api.width}px`,
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
      this.endDrag = undefined;
      this.dragging = false;
      slider.style.background = "rgba(121, 121, 121, 0.4)";
      doc.removeEventListener("pointermove", move);
      doc.removeEventListener("pointerup", up);
      doc.removeEventListener("pointercancel", up);
    };
    doc.addEventListener("pointermove", move);
    doc.addEventListener("pointerup", up);
    doc.addEventListener("pointercancel", up);
    this.endDrag = up;
  }

  /** 高さ（CSS ピクセル）と中身に合わせて、つまみの位置と大きさを決める。 */
  update(heightPx: number, enabled: boolean): void {
    const total = this.api.total();
    const rows = this.api.rows();
    const scrollable = total - rows;
    if (!enabled || this.api.width <= 0 || scrollable <= 0 || heightPx <= 0) {
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

  /** 後始末。DOM から外し、ドラッグ中なら終える。 */
  dispose(): void {
    this.endDrag?.();
    for (const c of this.cleanups) c();
    this.cleanups.length = 0;
    this.track.remove();
  }
}
