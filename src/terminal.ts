import { WasmTerminal } from "./wasm/coneko_term_wasm.js";
import { getMemory } from "./memory.js";

/** セル 1 つが使う `u32` の数。`RowView.cells` の並びは README を参照。 */
export const CELL_STRIDE = 6;

/** セルの字（並びの 0 番目）にこのビットが立っていたら、残りのビットが `RowView.cluster` の番号。 */
export const CLUSTER_FLAG = 0x8000_0000;

/** 後始末ができるもの。 */
export interface Disposable {
  dispose(): void;
}

/** 色。`#rgb` か `#rrggbb`。 */
export interface Theme {
  /** 文字の色（OSC 10 の問い合わせに返す）。 */
  foreground?: string;
  /** 背景の色（OSC 11）。 */
  background?: string;
  /** カーソルの色（OSC 12）。 */
  cursor?: string;
  /** 選択範囲の背景。描画（#8）が使う。 */
  selectionBackground?: string;
  black?: string;
  red?: string;
  green?: string;
  yellow?: string;
  blue?: string;
  magenta?: string;
  cyan?: string;
  white?: string;
  brightBlack?: string;
  brightRed?: string;
  brightGreen?: string;
  brightYellow?: string;
  brightBlue?: string;
  brightMagenta?: string;
  brightCyan?: string;
  brightWhite?: string;
}

/** `Terminal` を作るときの設定。 */
export interface TerminalOptions {
  /** 列の数。既定は 80。 */
  cols?: number;
  /** 行の数。既定は 24。 */
  rows?: number;
  /** 描画（#8）が使う。 */
  fontFamily?: string;
  /** 描画（#8）が使う。 */
  fontSize?: number;
  /** 描画（#8）が使う。 */
  lineHeight?: number;
  /** カーソルを点滅させるか。`CSI Ps SP q` で変わる。既定は false。 */
  cursorBlink?: boolean;
  /** スクロールバックの行数。既定は 1000。 */
  scrollback?: number;
  /** テーマ。 */
  theme?: Theme;
}

/** カーソルの状態。 */
export interface CursorState {
  x: number;
  y: number;
  visible: boolean;
  blinking: boolean;
  shape: "block" | "underline" | "bar";
}

/**
 * 1 行ぶんのセル。`cells` は WASM のメモリの上の見え方で、次に `readRow` などを呼ぶか
 * 書き込みで画面が変わるまでしか使えない。セルの並びは `CELL_STRIDE` 個ずつ:
 * 字、幅、前景色、背景色、属性と下線の形、下線の色。
 */
export interface RowView {
  /** セルの数。 */
  readonly length: number;
  /** `length * CELL_STRIDE` 個の値。 */
  readonly cells: Uint32Array;
  /** 次の行へ折り返されているか。 */
  readonly wrapped: boolean;
  /** `CLUSTER_FLAG` つきの字の番号から、書記素を引く。 */
  cluster(index: number): string;
}

const ANSI_NAMES = [
  "black",
  "red",
  "green",
  "yellow",
  "blue",
  "magenta",
  "cyan",
  "white",
  "brightBlack",
  "brightRed",
  "brightGreen",
  "brightYellow",
  "brightBlue",
  "brightMagenta",
  "brightCyan",
  "brightWhite",
] as const;

// xterm.js 6.0.0 の既定のテーマ（Rust の本体の既定と同じ）
const DEFAULT_THEME: Required<Omit<Theme, "selectionBackground">> = {
  foreground: "#ffffff",
  background: "#000000",
  cursor: "#ffffff",
  black: "#2e3436",
  red: "#cc0000",
  green: "#4e9a06",
  yellow: "#c4a000",
  blue: "#3465a4",
  magenta: "#75507b",
  cyan: "#06989a",
  white: "#d3d7cf",
  brightBlack: "#555753",
  brightRed: "#ef2929",
  brightGreen: "#8ae234",
  brightYellow: "#fce94f",
  brightBlue: "#729fcf",
  brightMagenta: "#ad7fa8",
  brightCyan: "#34e2e2",
  brightWhite: "#eeeeec",
};

function parseColor(value: string): [number, number, number] {
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(value.trim());
  if (!m) {
    throw new Error(`coneko-term: unsupported color "${value}" (use #rgb or #rrggbb)`);
  }
  let h = m[1];
  if (h.length === 3) h = [...h].map((c) => c + c).join("");
  return [
    parseInt(h.slice(0, 2), 16),
    parseInt(h.slice(2, 4), 16),
    parseInt(h.slice(4, 6), 16),
  ];
}

/** 1 回に処理する最大のバイト数。 */
const SLICE_BYTES = 64 * 1024;
/** 1 回の処理で使う時間（ミリ秒）。これを超えたらブラウザーに制御を返す。 */
const SLICE_MS = 8;

interface WriteJob {
  bytes: Uint8Array;
  offset: number;
  done?: () => void;
}

const sleep0 = () => new Promise<void>((r) => setTimeout(r, 0));

/**
 * ターミナル。VT のバイト列を `write` で受け、画面の中身を持つ。DOM を使わないので
 * Node.js でも動く（`open` を呼ぶまで DOM に触らない）。描画は #8 が足す。
 *
 * 使う前に `init()` を呼ぶこと。
 */
export class Terminal {
  readonly options: Readonly<TerminalOptions>;
  private wasm: WasmTerminal;

  /** `dispose` のあとに触ると、分かりやすい例外にする。 */
  private get core(): WasmTerminal {
    if (this.disposed) throw new Error("coneko-term: terminal is disposed");
    return this.wasm;
  }
  private queue: WriteJob[] = [];
  private pumping = false;
  private disposed = false;
  private dataListeners = new Set<(data: Uint8Array) => void>();
  private titleListeners = new Set<(title: string) => void>();
  private bellListeners = new Set<() => void>();
  private oscHandlers = new Map<number, Set<(data: string) => void>>();
  private element?: HTMLElement;
  private cleanups: (() => void)[] = [];
  private decoder = new TextDecoder();
  private encoder = new TextEncoder();

  constructor(options: TerminalOptions = {}) {
    getMemory();
    this.options = { ...options };
    const cols = Math.max(1, options.cols ?? 80);
    const rows = Math.max(1, options.rows ?? 24);
    this.wasm = new WasmTerminal(rows, cols, options.scrollback ?? 1000);
    this.core.set_cursor_blink(options.cursorBlink ?? false);
    this.applyTheme(options.theme ?? {});
  }

  private applyTheme(theme: Theme): void {
    const t = { ...DEFAULT_THEME, ...stripUndefined(theme) };
    const ansi = new Uint8Array(48);
    ANSI_NAMES.forEach((name, i) => ansi.set(parseColor(t[name]), i * 3));
    this.core.set_theme(
      Uint8Array.from(parseColor(t.foreground)),
      Uint8Array.from(parseColor(t.background)),
      Uint8Array.from(parseColor(t.cursor)),
      ansi,
    );
  }

  /** 列の数。 */
  get cols(): number {
    return this.core.cols();
  }

  /** 行の数。 */
  get rows(): number {
    return this.core.rows();
  }

  /** スクロールバックに入っている行の数。 */
  get scrollbackLength(): number {
    return this.core.scrollback_len();
  }

  /** カーソルの状態。 */
  get cursor(): CursorState {
    const [x, y, visible, blinking, shape] = this.core.cursor();
    return {
      x,
      y,
      visible: visible === 1,
      blinking: blinking === 1,
      shape: (["block", "underline", "bar"] as const)[shape],
    };
  }

  /** 代替画面にいるか。 */
  get isAlternateScreen(): boolean {
    return this.core.is_alt_screen();
  }

  /** bracketed paste が有効か（#9 が貼り付けで使う）。 */
  get bracketedPaste(): boolean {
    return this.core.bracketed_paste();
  }

  /** アプリケーションカーソルキーか（#9 が使う）。 */
  get applicationCursorKeys(): boolean {
    return this.core.application_cursor_keys();
  }

  /**
   * データを書く。文字列は UTF-8 にする。大きなデータは小分けにして処理し、間でブラウザーに
   * 制御を返す。処理は呼んだ順に行い、このデータを処理し終えたら `done` を呼ぶ。
   * `write` から戻った時点では、まだ画面に反映されていない。
   */
  write(data: string | Uint8Array, done?: () => void): void {
    if (this.disposed) return;
    // 非同期に処理するので、呼ぶ側がバッファーを使い回しても壊れないようにコピーする
    const bytes =
      typeof data === "string" ? this.encoder.encode(data) : data.slice();
    this.queue.push({ bytes, offset: 0, done });
    if (!this.pumping) {
      this.pumping = true;
      queueMicrotask(() => void this.pump());
    }
  }

  private async pump(): Promise<void> {
    try {
      let sliceStart = performance.now();
      while (!this.disposed && this.queue.length > 0) {
        const job = this.queue[0];
        const end = Math.min(job.offset + SLICE_BYTES, job.bytes.length);
        this.core.write(job.bytes.subarray(job.offset, end));
        job.offset = end;
        // 聞き手や done の例外で残りの処理を止めない。投げ直して報告する
        this.guard(() => this.flush());
        if (this.disposed) break;
        if (job.offset >= job.bytes.length) {
          this.queue.shift();
          if (job.done) this.guard(job.done);
        }
        if (performance.now() - sliceStart >= SLICE_MS && this.queue.length > 0) {
          await sleep0();
          sliceStart = performance.now();
        }
      }
    } finally {
      this.pumping = false;
    }
  }

  private guard(f: () => void): void {
    try {
      f();
    } catch (e) {
      queueMicrotask(() => {
        throw e;
      });
    }
  }

  /** PTY へ送るバイト列とイベントを、聞き手に渡す。 */
  private flush(): void {
    for (;;) {
      const ev = this.core.next_event();
      if (!ev) break;
      try {
      const data = ev.data;
      switch (ev.kind) {
        case 0:
          for (const l of [...this.bellListeners]) l();
          break;
        case 1: {
          const title = this.decoder.decode(data);
          for (const l of [...this.titleListeners]) l(title);
          break;
        }
        case 3: {
          const text = this.decoder.decode(data);
          for (const l of [...(this.oscHandlers.get(ev.ident) ?? [])]) l(text);
          break;
        }
      }
      } finally {
        ev.free();
      }
    }
    const out = this.core.take_output();
    if (out.length > 0) {
      for (const l of [...this.dataListeners]) l(out);
    }
  }

  /**
   * キーボードやマウスなどの入力を、返事と同じ出口（`onData`）から出す。
   * 文字列は UTF-8 にする。
   */
  input(data: string | Uint8Array): void {
    if (this.disposed) return;
    this.core.queue_output(
      typeof data === "string" ? this.encoder.encode(data) : data,
    );
    this.flush();
  }

  /**
   * PTY へ送るバイト列の出口。問い合わせへの返事（DSR など）と、`input` に渡したものが出る。
   * 渡す `Uint8Array` は聞き手のもの（コピー済み）。
   */
  onData(listener: (data: Uint8Array) => void): Disposable {
    return addListener(this.dataListeners, listener);
  }

  /** 題名が変わった（OSC 0、OSC 2）。 */
  onTitleChange(listener: (title: string) => void): Disposable {
    return addListener(this.titleListeners, listener);
  }

  /** BEL が来た。 */
  onBell(listener: () => void): Disposable {
    return addListener(this.bellListeners, listener);
  }

  /**
   * OSC の番号 `ident` のハンドラーを登録する（xterm.js の `parser.registerOscHandler` に当たる）。
   * `data` は `OSC ident ;` のあとの文字列（UTF-8）。登録した番号は、ターミナルの内蔵の処理の
   * 代わりにハンドラーが受ける。同じ番号に複数登録できる。
   */
  registerOscHandler(
    ident: number,
    handler: (data: string) => void,
  ): Disposable {
    let set = this.oscHandlers.get(ident);
    if (!set) {
      set = new Set();
      this.oscHandlers.set(ident, set);
      this.core.register_osc(ident);
    }
    set.add(handler);
    return {
      dispose: () => {
        const s = this.oscHandlers.get(ident);
        if (!s?.delete(handler) || s.size > 0) return;
        this.oscHandlers.delete(ident);
        if (!this.disposed) this.core.unregister_osc(ident);
      },
    };
  }

  /** 初期状態に戻す。書き込み待ちのデータは捨てる（`done` は呼ばない）。 */
  reset(): void {
    this.queue.length = 0;
    this.core.reset();
  }

  /** 大きさを変える。 */
  resize(cols: number, rows: number): void {
    this.core.resize(Math.max(1, rows), Math.max(1, cols));
  }

  /**
   * 欄の大きさに合わせて大きさを変える（FitAddon の `fit()` に当たる）。
   * 描画（#8）ができるまでは何もせず、いまの大きさを返す。
   */
  fit(): { cols: number; rows: number } {
    return { cols: this.cols, rows: this.rows };
  }

  /** ウィンドウの代わりに、フォーカスを得た・失ったことを知らせる。 */
  setFocused(focused: boolean): void {
    this.core.set_focus(focused);
    this.flush();
  }

  /**
   * 欄の要素に付ける。いまは描かず、フォーカスを受ける要素（`tabindex` つき）を作るだけ。
   * 描画は #8 が足す。
   */
  open(host: HTMLElement): void {
    if (this.element) throw new Error("coneko-term: already opened");
    const el = host.ownerDocument.createElement("div");
    el.className = "coneko-term";
    el.tabIndex = 0;
    const onFocus = () => this.setFocused(true);
    const onBlur = () => this.setFocused(false);
    el.addEventListener("focus", onFocus);
    el.addEventListener("blur", onBlur);
    this.cleanups.push(() => {
      el.removeEventListener("focus", onFocus);
      el.removeEventListener("blur", onBlur);
      el.remove();
    });
    host.appendChild(el);
    this.element = el;
  }

  /** `open` で作った要素。`open` の前は `undefined`。 */
  get hostElement(): HTMLElement | undefined {
    return this.element;
  }

  /** フォーカスを当てる。`open` の前は何もしない。 */
  focus(): void {
    this.element?.focus();
  }

  /** 変わった行（見えている画面の行番号）を返し、「変わった」の印を消す。 */
  takeDirtyRows(): number[] {
    return Array.from(this.core.take_dirty_rows());
  }

  /** 見えている画面の `y` 行目のセル。 */
  readRow(y: number): RowView {
    return this.view(this.core.read_row(y));
  }

  /** スクロールバックの `i` 行目（0 が一番古い）のセル。 */
  readScrollbackRow(i: number): RowView {
    return this.view(this.core.read_scrollback_row(i));
  }

  /** 見えている画面の `y` 行目の字（空のセルは空白）。末尾の空白は残す。 */
  getRowText(y: number): string {
    const row = this.readRow(y);
    let s = "";
    for (let x = 0; x < row.length; x++) {
      const cp = row.cells[x * CELL_STRIDE];
      const width = row.cells[x * CELL_STRIDE + 1];
      if (width === 0) continue;
      if (cp === 0) s += " ";
      else if (cp & CLUSTER_FLAG) s += row.cluster(cp & ~CLUSTER_FLAG);
      else s += String.fromCodePoint(cp);
    }
    return s;
  }

  private view(length: number): RowView {
    const core = this.core;
    const wrapped = core.row_wrapped();
    const cells = new Uint32Array(
      getMemory().buffer,
      core.cells_ptr() as unknown as number,
      length * CELL_STRIDE,
    );
    return { length, cells, wrapped, cluster: (i) => core.cluster(i) };
  }

  /** 後始末をする。書き込み待ちのデータは捨てる（`done` は呼ばない）。 */
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.queue.length = 0;
    for (const c of this.cleanups) c();
    this.cleanups.length = 0;
    this.element = undefined;
    this.dataListeners.clear();
    this.titleListeners.clear();
    this.bellListeners.clear();
    this.oscHandlers.clear();
    this.wasm.free();
  }
}

function addListener<T>(set: Set<T>, listener: T): Disposable {
  set.add(listener);
  return { dispose: () => void set.delete(listener) };
}

function stripUndefined<T extends object>(o: T): Partial<T> {
  return Object.fromEntries(
    Object.entries(o).filter(([, v]) => v !== undefined),
  ) as Partial<T>;
}
