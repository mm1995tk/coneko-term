import { WasmTerminal } from "./wasm/coneko_term_wasm.js";
import { getMemory } from "./memory.js";
import {
  CompositionHelper,
  KeyHandler,
  detectPlatform,
  type CompositionViewLike,
} from "./input.js";
import { bracketTextForPaste, prepareTextForTerminal } from "./keyboard.js";
import {
  CanvasRenderer,
  Palette,
  Scrollbar,
  SCROLLBAR_WIDTH,
  proposeDimensions,
  type CellSize,
} from "./renderer.js";

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
  /** 選択範囲の背景。選択の表示（#10）が使う。 */
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
  /** 描画に使うフォント（CSS の `font-family`）。既定は `monospace`。 */
  fontFamily?: string;
  /** 字の大きさ（CSS ピクセル）。既定は 15（xterm.js と同じ）。 */
  fontSize?: number;
  /** 行の高さ。字の高さに掛ける倍率（xterm.js と同じ）。既定は 1。 */
  lineHeight?: number;
  /** カーソルを点滅させるか。`CSI Ps SP q` で変わる。既定は false。 */
  cursorBlink?: boolean;
  /** スクロールバックの行数。既定は 1000。 */
  scrollback?: number;
  /** テーマ。 */
  theme?: Theme;
  /**
   * macOS で Option を Meta（ESC の前置き）として送るか。既定は false（xterm.js と同じ。
   * Option は字を打つキーとして扱う）。
   */
  macOptionIsMeta?: boolean;
  /** キーを打ったとき、さかのぼって見ていたら一番下へ戻すか。既定は true（xterm.js と同じ）。 */
  scrollOnUserInput?: boolean;
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
 * Node.js でも動く（`open` を呼ぶまで DOM に触らない）。`open` すると Canvas 2D で描く。
 *
 * 使う前に `init()` を呼ぶこと。
 */
export class Terminal {
  readonly options: Readonly<TerminalOptions>;
  private resolvedTheme: Required<Omit<Theme, "selectionBackground">> =
    DEFAULT_THEME;
  private renderer?: CanvasRenderer;
  private scrollbar?: Scrollbar;
  private focused = false;
  /** 画面の一番下から何行さかのぼって見ているか。0 なら一番下。 */
  private viewOffset = 0;
  private lastScrollback = 0;
  private scrollListeners = new Set<(viewportY: number) => void>();
  private cellSizeListeners = new Set<(size: CellSize) => void>();
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
  private textarea?: HTMLTextAreaElement;
  private composition?: CompositionHelper;
  private customKeyHandler?: (ev: KeyboardEvent) => boolean;
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
    this.resolvedTheme = t;
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

  /** bracketed paste（2004）が有効か。`paste` が使う。 */
  get bracketedPaste(): boolean {
    return this.core.bracketed_paste();
  }

  /** アプリケーションカーソルキー（DECCKM）か。矢印・Home・End の送り方が変わる。 */
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
        this.syncViewport();
        this.afterChange();
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
   * ユーザーの入力（キー・変換の確定・貼り付け）を出口から出す。`scrollOnUserInput` が false でなく、
   * さかのぼって見ていたら、一番下へ戻す。
   */
  private sendUserInput(data: string): void {
    if (this.disposed || data.length === 0) return;
    if (this.options.scrollOnUserInput !== false) this.scrollToBottom();
    this.input(data);
  }

  /**
   * 貼り付ける。改行は `\r` にそろえ（xterm.js と同じ）、bracketed paste（2004）が有効なら
   * `ESC [200~` と `ESC [201~` で囲んで、出口から出す。`paste` イベントも、これを呼ぶ。
   */
  paste(text: string): void {
    if (this.disposed) return;
    this.sendUserInput(
      bracketTextForPaste(prepareTextForTerminal(text), this.bracketedPaste),
    );
    if (this.textarea) this.textarea.value = "";
  }

  /**
   * キーのイベントを、ターミナルより先に見るハンドラー（xterm.js の `attachCustomKeyEventHandler`）。
   * `false` を返したキーは、ターミナルが処理しない。外すには `undefined`。
   */
  attachCustomKeyEventHandler(
    handler: ((ev: KeyboardEvent) => boolean) | undefined,
  ): void {
    this.customKeyHandler = handler;
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
    this.viewOffset = 0;
    this.lastScrollback = this.core.scrollback_len();
    this.afterChange();
  }

  /** 大きさを変える。 */
  resize(cols: number, rows: number): void {
    this.core.resize(Math.max(1, rows), Math.max(1, cols));
    this.lastScrollback = this.core.scrollback_len();
    this.viewOffset = Math.min(this.viewOffset, this.lastScrollback);
    this.renderer?.resize();
    this.afterChange();
  }

  /**
   * 欄（`open` に渡した要素）の大きさに合わせて列と行の数を決め、大きさを変える
   * （xterm.js の FitAddon の `fit()` と同じ式。スクロールバーの幅を引く）。
   * 返すのは変えたあとの大きさ。`open` の前や、欄が見えていない（大きさが 0）ときや、
   * フォントの大きさがまだ測れていないときは、何もせず、いまの大きさを返す。
   */
  fit(): { cols: number; rows: number } {
    const dims = this.proposeDimensions();
    if (dims && (dims.cols !== this.cols || dims.rows !== this.rows)) {
      this.resize(dims.cols, dims.rows);
    }
    return { cols: this.cols, rows: this.rows };
  }

  /** `fit()` が使う列と行の数を、変えずに出す。 */
  proposeDimensions(): { cols: number; rows: number } | undefined {
    const host = this.host;
    const cell = this.renderer?.cellSize;
    if (!host || !cell) return undefined;
    const style = host.ownerDocument.defaultView?.getComputedStyle(host);
    const px = (v: string | undefined) => Number.parseFloat(v ?? "") || 0;
    const width =
      host.clientWidth - px(style?.paddingLeft) - px(style?.paddingRight);
    const height =
      host.clientHeight - px(style?.paddingTop) - px(style?.paddingBottom);
    return proposeDimensions({
      width,
      height,
      cellWidth: cell.width,
      cellHeight: cell.height,
      scrollbarWidth: this.options.scrollback === 0 ? 0 : SCROLLBAR_WIDTH,
    });
  }

  /** フォーカスがあるか（`setFocused` か、要素のフォーカスで決まる）。 */
  get isFocused(): boolean {
    return this.focused;
  }

  /** ウィンドウの代わりに、フォーカスを得た・失ったことを知らせる。 */
  setFocused(focused: boolean): void {
    this.focused = focused;
    this.core.set_focus(focused);
    this.flush();
    this.renderer?.touch();
  }

  /**
   * 欄の要素に付ける。canvas とスクロールバーを持つ要素（`tabindex` つき）を作り、
   * Canvas 2D で描き始める。字の幅と高さは、フォントを読み終えてから測り直す。
   * 欄の大きさに合わせるには `fit()` を呼ぶ。
   */
  open(host: HTMLElement): void {
    if (this.element) throw new Error("coneko-term: already opened");
    const doc = host.ownerDocument;
    const el = doc.createElement("div");
    el.className = "coneko-term";
    Object.assign(el.style, {
      position: "relative",
      width: "100%",
      height: "100%",
      overflow: "hidden",
      outline: "none",
      background: this.resolvedTheme.background,
    });
    const canvas = doc.createElement("canvas");
    canvas.className = "coneko-term-screen";
    Object.assign(canvas.style, { position: "absolute", left: "0", top: "0" });
    el.appendChild(canvas);
    this.cleanups.push(() => el.remove());
    host.appendChild(el);
    this.element = el;
    this.host = host;

    this.scrollbar = new Scrollbar(el, {
      total: () => this.scrollbackLength + this.rows,
      rows: () => this.rows,
      viewportY: () => this.viewportY,
      scrollToLine: (n) => this.scrollToLine(n),
      scrollLines: (n) => this.scrollLines(n),
    });
    this.cleanups.push(() => this.scrollbar?.dispose());

    const renderer = new CanvasRenderer(canvas, this, this.renderOptions());
    this.renderer = renderer;
    renderer.onCellSizeChange((size) => {
      for (const l of [...this.cellSizeListeners]) l(size);
    });
    this.cleanups.push(() => renderer.dispose());
    this.setupInput(doc, el);
    this.afterChange();

    this.cleanups.push(this.watchFonts(doc, renderer));
    this.cleanups.push(this.watchDevicePixelRatio(doc, renderer));
  }

  /**
   * 見えない `textarea` を置いて、キー・変換・貼り付け・フォーカスを受ける。
   * 変換中の字は、カーソルの位置に重ねて見せる。
   */
  private setupInput(doc: Document, el: HTMLElement): void {
    const textarea = doc.createElement("textarea");
    textarea.className = "coneko-term-input";
    textarea.setAttribute("aria-label", "Terminal input");
    textarea.setAttribute("autocorrect", "off");
    textarea.setAttribute("autocapitalize", "off");
    textarea.setAttribute("autocomplete", "off");
    textarea.setAttribute("spellcheck", "false");
    textarea.setAttribute("wrap", "off");
    textarea.tabIndex = 0;
    Object.assign(textarea.style, {
      position: "absolute",
      left: "-9999em",
      top: "0",
      width: "0",
      height: "0",
      padding: "0",
      border: "0",
      margin: "0",
      opacity: "0",
      zIndex: "-5",
      whiteSpace: "nowrap",
      overflow: "hidden",
      resize: "none",
    });

    const viewEl = doc.createElement("div");
    viewEl.className = "coneko-term-composition";
    Object.assign(viewEl.style, {
      display: "none",
      position: "absolute",
      whiteSpace: "nowrap",
      zIndex: "1",
      background: this.resolvedTheme.background,
      color: this.resolvedTheme.foreground,
    });
    const view: CompositionViewLike = {
      get textContent() {
        return viewEl.textContent;
      },
      set textContent(v) {
        viewEl.textContent = v;
      },
      style: viewEl.style as unknown as CompositionViewLike["style"],
      getBoundingClientRect: () => viewEl.getBoundingClientRect(),
      show: () => void (viewEl.style.display = "inline-block"),
      hide: () => void (viewEl.style.display = "none"),
    };
    el.append(textarea, viewEl);

    const platform = detectPlatform();
    const composition = new CompositionHelper(
      textarea,
      view,
      (data) => this.sendUserInput(data),
      () => {
        const cell = this.renderer?.cellSize;
        if (!cell || this.viewportOffset !== 0) return undefined;
        const c = this.cursor;
        return {
          left: Math.min(c.x, this.cols - 1) * cell.width,
          top: c.y * cell.height,
          cellHeight: cell.height,
        };
      },
      () => ({
        family: this.options.fontFamily ?? "monospace",
        size: this.options.fontSize ?? 15,
      }),
    );
    this.composition = composition;
    const keys = new KeyHandler(
      {
        send: (data) => this.sendUserInput(data),
        applicationCursorKeys: () => this.applicationCursorKeys,
        scrollPages: (n) => this.scrollLines(n * (this.rows - 1)),
        isMac: platform.isMac,
        isWindows: platform.isWindows,
        macOptionIsMeta: this.options.macOptionIsMeta ?? false,
        customKeyHandler: () => this.customKeyHandler,
      },
      textarea,
      composition,
    );
    this.textarea = textarea;

    const on = <E extends Event>(
      target: EventTarget,
      type: string,
      f: (ev: E) => void,
      capture = false,
    ) => {
      const h = f as EventListener;
      target.addEventListener(type, h, capture);
      this.cleanups.push(() => target.removeEventListener(type, h, capture));
    };
    on<KeyboardEvent>(textarea, "keydown", (e) => void keys.keyDown(e), true);
    on<KeyboardEvent>(textarea, "keyup", () => keys.keyUp(), true);
    on<KeyboardEvent>(textarea, "keypress", (e) => void keys.keyPress(e), true);
    on<InputEvent>(textarea, "input", (e) => void keys.input(e), true);
    on<CompositionEvent>(textarea, "compositionstart", () =>
      composition.compositionstart(),
    );
    on<CompositionEvent>(textarea, "compositionupdate", (e) =>
      composition.compositionupdate(e.data),
    );
    on<CompositionEvent>(textarea, "compositionend", () =>
      composition.compositionend(),
    );
    const onPaste = (e: ClipboardEvent) => {
      e.stopPropagation();
      e.preventDefault();
      const text = e.clipboardData?.getData("text/plain");
      if (text) this.paste(text);
    };
    on<ClipboardEvent>(textarea, "paste", onPaste);
    on<ClipboardEvent>(el, "paste", onPaste);
    on<FocusEvent>(textarea, "focus", () => this.setFocused(true));
    on<FocusEvent>(textarea, "blur", () => {
      textarea.value = "";
      this.setFocused(false);
    });
    // 欄を押したら、フォーカスは `textarea` へ（押した先に動かさない）。マウスの報告や選択は #10
    on<MouseEvent>(el, "mousedown", (e) => {
      if (e.target !== textarea) e.preventDefault();
      textarea.focus();
    });
  }

  /** フォントを読み終えてから測り直す（読む前は代わりのフォントの大きさになる）。戻り値は後始末。 */
  private watchFonts(doc: Document, renderer: CanvasRenderer): () => void {
    const fonts = doc.fonts;
    if (!fonts) return () => {};
    const remeasure = () => {
      if (!this.disposed) renderer.measure();
    };
    const spec = `${this.options.fontSize ?? 15}px ${this.options.fontFamily ?? "monospace"}`;
    void fonts.load(spec).then(remeasure, () => {});
    void fonts.ready.then(remeasure, () => {});
    fonts.addEventListener("loadingdone", remeasure);
    return () => fonts.removeEventListener("loadingdone", remeasure);
  }

  /** `devicePixelRatio` が変わったら（別のディスプレイへ動かした、ズームした）測り直す。戻り値は後始末。 */
  private watchDevicePixelRatio(
    doc: Document,
    renderer: CanvasRenderer,
  ): () => void {
    const win = doc.defaultView;
    if (!win?.matchMedia) return () => {};
    let mq: MediaQueryList | undefined;
    const watch = () => {
      mq?.removeEventListener("change", onChange);
      mq = win.matchMedia(`(resolution: ${win.devicePixelRatio}dppx)`);
      mq.addEventListener("change", onChange);
    };
    const onChange = () => {
      watch();
      if (!this.disposed) renderer.measure();
    };
    watch();
    return () => mq?.removeEventListener("change", onChange);
  }

  private renderOptions() {
    const t = this.resolvedTheme;
    return {
      fontFamily: this.options.fontFamily ?? "monospace",
      fontSize: this.options.fontSize ?? 15,
      lineHeight: this.options.lineHeight ?? 1,
      palette: new Palette(
        ANSI_NAMES.map((n) => t[n]),
        t.foreground,
        t.background,
        t.cursor,
      ),
    };
  }

  /** セルの大きさ（CSS ピクセル）。`open` の前は `undefined`。 */
  get cellSize(): CellSize | undefined {
    return this.renderer?.cellSize;
  }

  /** セルの大きさが変わった（フォントを読み終えた、`devicePixelRatio` が変わった）。`fit()` を呼び直す目印に使う。 */
  onCellSizeChange(listener: (size: CellSize) => void): Disposable {
    return addListener(this.cellSizeListeners, listener);
  }

  // ---- 表示位置（viewport）

  /** 実際に使う、さかのぼっている行数。代替画面では常に 0。 */
  get viewportOffset(): number {
    return this.core.is_alt_screen() ? 0 : this.viewOffset;
  }

  /** 一番上に見えている行の番号。0 がスクロールバックの一番古い行で、一番下を見ているときは `scrollbackLength`。 */
  get viewportY(): number {
    return this.scrollbackLength - this.viewportOffset;
  }

  /** 一番下を見ているか（新しい出力に付いていく状態）。 */
  get isAtBottom(): boolean {
    return this.viewportOffset === 0;
  }

  /** `n` 行ぶん動かす。負の数で上（古い方）、正の数で下。 */
  scrollLines(n: number): void {
    this.setViewOffset(this.viewOffset - Math.trunc(n));
  }

  /** 一番上に見える行を `line`（0 がスクロールバックの一番古い行）にする。 */
  scrollToLine(line: number): void {
    this.setViewOffset(this.scrollbackLength - Math.trunc(line));
  }

  /** 一番古い行へ。 */
  scrollToTop(): void {
    this.setViewOffset(this.scrollbackLength);
  }

  /** 一番下へ。 */
  scrollToBottom(): void {
    this.setViewOffset(0);
  }

  /** 表示位置が変わった。引数は新しい `viewportY`。 */
  onScroll(listener: (viewportY: number) => void): Disposable {
    return addListener(this.scrollListeners, listener);
  }

  private setViewOffset(offset: number): void {
    if (this.disposed) return;
    const before = this.viewportY;
    this.viewOffset = Math.max(0, Math.min(offset, this.scrollbackLength));
    if (this.viewportY === before) return;
    this.renderer?.invalidate();
    this.afterChange();
    for (const l of [...this.scrollListeners]) this.guard(() => l(this.viewportY));
  }

  /** 書き込みでスクロールバックが増えたとき、さかのぼって見ているなら同じ内容を見続ける。 */
  private syncViewport(): void {
    const len = this.core.scrollback_len();
    const grown = len - this.lastScrollback;
    this.lastScrollback = len;
    if (this.viewOffset > 0 && grown > 0) this.viewOffset += grown;
    this.viewOffset = Math.min(this.viewOffset, len);
  }

  /** 描画とスクロールバーに、画面が変わったことを知らせる。 */
  private afterChange(): void {
    const r = this.renderer;
    if (!r) return;
    r.touch();
    this.composition?.updateCompositionElements(true);
    const cell = r.cellSize;
    this.scrollbar?.update(
      this.rows * cell.height,
      !this.core.is_alt_screen(),
    );
  }

  /** 画面の `y` 行目（表示位置を考えた、いま見えている行）のセル。 */
  readViewportRow(y: number): RowView {
    const i = this.viewportY + y;
    const sb = this.scrollbackLength;
    return i < sb ? this.readScrollbackRow(i) : this.readRow(i - sb);
  }

  /** 欄の要素。 */
  private host?: HTMLElement;

  /** `open` で作った要素。`open` の前は `undefined`。 */
  get hostElement(): HTMLElement | undefined {
    return this.element;
  }

  /** 見えない `textarea` にフォーカスを当てる。`open` の前は何もしない。 */
  focus(): void {
    this.textarea?.focus();
  }

  /** フォーカスを外す。`open` の前は何もしない。 */
  blur(): void {
    this.textarea?.blur();
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
    this.textarea = undefined;
    this.composition = undefined;
    this.host = undefined;
    this.renderer = undefined;
    this.scrollbar = undefined;
    this.scrollListeners.clear();
    this.cellSizeListeners.clear();
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
