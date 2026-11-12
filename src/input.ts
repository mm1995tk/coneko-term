// キーボードと IME の入力を受ける部分。xterm.js 6.0.0 の `CoreBrowserTerminal` の
// `_keyDown`・`_keyPress`・`_inputEvent` と `CompositionHelper` の動きに合わせる。
// DOM の要素そのものではなく、使う部分だけの形（`TextareaLike` など）に頼るので、DOM の無い
// テストでも動かせる。

import {
  evaluateKeyboardEvent,
  KITTY_REPORT_ALL_KEYS,
  KITTY_REPORT_EVENT_TYPES,
  type KeyEventLike,
} from "./keyboard.js";

/** 位置と大きさを決めるのに使う `style` の部分。 */
export interface StyleLike {
  left: string;
  top: string;
  width: string;
  height: string;
  lineHeight: string;
  fontFamily: string;
  fontSize: string;
}

/** 見えない `textarea` のうち、ここで使う部分。 */
export interface TextareaLike {
  value: string;
  readonly style: StyleLike;
  /** あれば、`nudge` で選択範囲を動かすのに使う（値の終わりの後ろへ動かし、値の終わりへ戻す）。 */
  setSelectionRange?(start: number, end: number): void;
  /** あれば、`textarea` にフォーカスがあるかを見るのに使う（無ければ、あるとみなす）。 */
  readonly ownerDocument?: { readonly activeElement: unknown } | null;
}

/** 変換中の字を見せる要素のうち、ここで使う部分。 */
export interface CompositionViewLike {
  textContent: string | null;
  readonly style: StyleLike;
  getBoundingClientRect(): { width: number; height: number };
  show(): void;
  hide(): void;
}

/** 変換中の字を出す位置（欄の左上からの CSS ピクセル）。見せないなら `undefined`。 */
export interface CompositionAnchor {
  left: number;
  top: number;
  cellWidth: number;
  cellHeight: number;
}

/**
 * 変換（IME）の間は送らず、確定した字を送る。変換中の字は、カーソルの位置に見せる。
 * xterm.js の `CompositionHelper` と同じ流れ。
 */
export class CompositionHelper {
  private composing = false;
  private sending = false;
  private position = { start: 0, end: 0 };
  private dataAlreadySent = "";

  constructor(
    private readonly textarea: TextareaLike,
    private readonly view: CompositionViewLike,
    private readonly send: (data: string) => void,
    private readonly anchor: () => CompositionAnchor | undefined,
    private readonly font: () => { family: string; size: number },
    private readonly schedule: (f: () => void) => void = (f) => void setTimeout(f, 0),
  ) {}

  get isComposing(): boolean {
    return this.composing;
  }

  compositionstart(): void {
    this.composing = true;
    this.position.start = this.textarea.value.length;
    this.view.textContent = "";
    this.dataAlreadySent = "";
    this.view.show();
  }

  compositionupdate(data: string): void {
    this.view.textContent = data;
    this.updateCompositionElements();
    this.schedule(() => {
      this.position.end = this.textarea.value.length;
    });
  }

  compositionend(): void {
    this.finalize(true);
  }

  /**
   * キーが押されたとき、変換のためのキーなら `false`（呼ぶ側は何もしない）、
   * 普通のキーなら `true`。変換の途中で普通のキーが来たら、先に変換を確定する。
   */
  keydown(ev: Pick<KeyEventLike, "keyCode">): boolean {
    if (this.composing || this.sending) {
      // 20 は CapsLock、229 は変換中のキー
      if (ev.keyCode === 20 || ev.keyCode === 229) return false;
      // 修飾キーだけなら変換を続ける
      if (ev.keyCode === 16 || ev.keyCode === 17 || ev.keyCode === 18) {
        return false;
      }
      // Enter などで、コマンドが走る前に確定した字を送る
      this.finalize(false);
    }
    if (ev.keyCode === 229) {
      // 変換の字ではない字（数字や記号）が、IME が有効なまま打たれた
      this.handleAnyTextareaChanges();
      return false;
    }
    return true;
  }

  private finalize(waitForPropagation: boolean): void {
    this.view.hide();
    this.composing = false;
    if (!waitForPropagation) {
      this.sending = false;
      const input = this.textarea.value.substring(
        this.position.start,
        this.position.end,
      );
      this.send(input);
      return;
    }
    // 多くのブラウザーで、composition のイベントは `textarea` の値が変わる前に来る。
    // `compositionend` の `data` はあてにならないので、一度待って値を読む
    const current = { ...this.position };
    this.sending = true;
    this.schedule(() => {
      if (!this.sending) return;
      this.sending = false;
      // keydown で先に送った分を引く（二重に打たれるのを防ぐ）
      current.start += this.dataAlreadySent.length;
      const input = this.composing
        ? this.textarea.value.substring(current.start, this.position.start)
        : this.textarea.value.substring(current.start);
      if (input.length > 0) this.send(input);
    });
  }

  private handleAnyTextareaChanges(): void {
    const oldValue = this.textarea.value;
    this.schedule(() => {
      if (this.composing) return;
      const newValue = this.textarea.value;
      const diff = newValue.replace(oldValue, "");
      this.dataAlreadySent = diff;
      if (newValue.length > oldValue.length) this.send(diff);
      else if (newValue.length < oldValue.length) this.send("\x7f");
      else if (newValue !== oldValue) this.send(newValue);
    });
  }

  /**
   * `textarea` が画面の中で動いたとき、macOS が入力の表示（あ / A）を出す位置を求め直すように促す。
   * WebKit（WKWebView）は、選択範囲の矩形を UI プロセスへ送り直すのを、選択範囲・値・スタイルの
   * 適用が変わったときにしかしない。レイアウトで要素が動いただけでは送らず、同じ位置に選択範囲を
   * 置き直しても何もしない。そこで、値の終わりに字を 1 つ足して選択範囲を本当に動かし、すぐ元に戻す。
   * コードから値を変えても `input` イベントは来ないので、PTY には何も送らない。
   * フォーカスが無いときは何もしない（WebKit は送らず、フォーカスを得たときに自分で送り直す）。
   */
  nudge(): void {
    // 変換中に値や選択範囲を変えると、変換中の字が確定や取り消しになるブラウザーがある
    if (this.composing) return;
    const ta = this.textarea;
    if (!ta.setSelectionRange) return;
    if (ta.ownerDocument && ta.ownerDocument.activeElement !== ta) return;
    const v = ta.value;
    ta.value = v + " ";
    ta.setSelectionRange(v.length + 1, v.length + 1);
    ta.value = v;
    ta.setSelectionRange(v.length, v.length);
  }

  /**
   * `textarea` を、いつもカーソルの位置に置く（macOS の入力の表示（あ / A）や IME の候補の窓が、
   * カーソルの近くに出る）。変換中は、変換の字もカーソルの位置へ動かす。
   * カーソルが見えないとき（さかのぼって見ているとき）は、動かさない。
   */
  updateCompositionElements(dontRecurse = false): void {
    const a = this.anchor();
    if (!a) {
      if (this.composing && !dontRecurse) {
        this.schedule(() => this.updateCompositionElements(true));
      }
      return;
    }
    const ts = this.textarea.style;
    ts.left = `${a.left}px`;
    ts.top = `${a.top}px`;
    if (!this.composing) {
      // xterm.js と同じく 1 セルにする。0x0 だと動かない IME がある
      ts.width = `${Math.max(a.cellWidth, 1)}px`;
      ts.height = `${Math.max(a.cellHeight, 1)}px`;
      ts.lineHeight = `${a.cellHeight}px`;
      return;
    }
    const f = this.font();
    const vs = this.view.style;
    vs.left = `${a.left}px`;
    vs.top = `${a.top}px`;
    vs.height = `${a.cellHeight}px`;
    vs.lineHeight = `${a.cellHeight}px`;
    vs.fontFamily = f.family;
    vs.fontSize = `${f.size}px`;
    const b = this.view.getBoundingClientRect();
    // 1x1 より小さいと動かない IME がある
    ts.width = `${Math.max(b.width, 1)}px`;
    ts.height = `${Math.max(b.height, 1)}px`;
    ts.lineHeight = `${b.height}px`;
    if (!dontRecurse) this.schedule(() => this.updateCompositionElements(true));
  }
}

/**
 * `textarea` の、画面の中の位置が変わったかを見る。変わったら `onMoved` を呼ぶ
 * （大きさだけの変化は数えない）。置いた要素ごと動いたときも、カーソルのセルへ `textarea` を
 * 動かしたときも数える。動いたことを知らせる決まったイベントが無いので、
 * キーやフォーカスや画面の更新のたびに `check` を呼ぶ。
 */
export class HostMoveWatcher {
  private last: { left: number; top: number } | undefined;

  constructor(
    private readonly position: () => { left: number; top: number },
    private readonly onMoved: () => void,
  ) {}

  /**
   * `textarea` の今の位置を読み、前回の `check` から変わっていれば `onMoved` を呼ぶ。
   * 最初の呼び出しでは位置を覚えるだけで、知らせない（作ったときに一度呼んでおくと、その後の動きを見逃さない）。
   */
  check(): void {
    const p = this.position();
    const last = this.last;
    this.last = { left: p.left, top: p.top };
    if (last && (last.left !== p.left || last.top !== p.top)) this.onMoved();
  }
}

/** `KeyHandler` が読む、プラットフォームとモード。 */
export interface KeyHandlerHost {
  /** PTY へ送る（ユーザーが打った入力として）。 */
  send(data: string): void;
  /** DECCKM（#5）。 */
  applicationCursorKeys(): boolean;
  /** kitty keyboard protocol のいまのフラグ。 */
  keyboardFlags(): number;
  /** 表示位置を `n` ページ動かす（負で上）。 */
  scrollPages(n: number): void;
  /** 一番下へ戻す（`scrollOnUserInput` が true のとき）。 */
  scrollToBottom(): void;
  readonly isMac: boolean;
  readonly isWindows: boolean;
  readonly macOptionIsMeta: boolean;
  /** `false` を返すと、ターミナルはそのキーを処理しない（xterm.js の `attachCustomKeyEventHandler`）。 */
  customKeyHandler(): ((ev: KeyboardEvent) => boolean) | undefined;
}

/** イベントのうち、ここで触る部分。 */
type KeyDownLike = KeyEventLike & {
  readonly type?: string;
  preventDefault(): void;
  stopPropagation(): void;
  getModifierState?(key: string): boolean;
};

/**
 * `textarea` の `keydown`・`keypress`・`keyup`・`input` と変換のイベントを受けて、送る字を決める。
 * キーを送るバイト列は `evaluateKeyboardEvent`（xterm.js 6.0.0 と同じ）。
 */
export class KeyHandler {
  private keyDownSeen = false;
  private keyPressHandled = false;
  private unprocessedDeadKey = false;
  // keydown で `CSI` の形で送ったキー。離すを送るのは、これに入っているキーだけ。
  // 変換の確定の Enter のように、押したのを送っていないキーの離すだけを送ってしまわないため
  private readonly reported = new Set<string>();
  // 字を打つキーの押すを keypress か input で送ったキー。フラグ 8 が無いと、押すは字のまま送るので
  // `reported` に入らない。kitty は、この場合も離すを `CSI` の形で送る
  private readonly textPressed = new Set<string>();
  // 直近の keydown のキー（keypress・input の字を、どのキーが打ったか結びつける）。変換のキーと、macOS の Cmd を押したキーでは null
  private lastDown: string | null = null;

  constructor(
    private readonly host: KeyHandlerHost,
    private readonly textarea: TextareaLike,
    readonly composition: CompositionHelper,
  ) {}

  private cancel(ev: { preventDefault(): void; stopPropagation(): void }): void {
    ev.preventDefault();
    ev.stopPropagation();
  }

  /** macOS の Option（`macOptionIsMeta` が false のとき）と、Windows の AltGr は、字を打つキー。 */
  private isThirdLevelShift(ev: KeyDownLike): boolean {
    const h = this.host;
    const thirdLevel =
      (h.isMac && !h.macOptionIsMeta && ev.altKey && !ev.ctrlKey && !ev.metaKey) ||
      (h.isWindows && ev.altKey && ev.ctrlKey && !ev.metaKey) ||
      (h.isWindows && !!ev.getModifierState?.("AltGraph"));
    if (ev.type === "keypress") return thirdLevel;
    // 矢印や Backspace などは、keydown では三段目のシフトとして扱わない
    return thirdLevel && (!ev.keyCode || ev.keyCode > 47);
  }

  keyDown(ev: KeyDownLike): boolean | undefined {
    this.keyDownSeen = true;
    this.lastDown = null;
    const custom = this.host.customKeyHandler();
    if (custom && custom(ev as unknown as KeyboardEvent) === false) return false;
    const h = this.host;
    // macOptionIsMeta のとき、Option の変換は使わない
    const ignoreComposition = h.isMac && h.macOptionIsMeta && ev.altKey;
    if (!ignoreComposition && !this.composition.keydown(ev)) {
      // 変換のキー。変換中の字は一番下のカーソルの位置に出すので、さかのぼっていたら戻す（xterm.js と同じ）
      h.scrollToBottom();
      return false;
    }
    // Cmd を押したキーは送らない（アプリのショートカットに任せる）。変換の確定のあとで判断する
    if (h.isMac && ev.metaKey) return true;
    this.lastDown = keyId(ev);

    if (!ignoreComposition && (ev.key === "Dead" || ev.key === "AltGraph")) {
      this.unprocessedDeadKey = true;
    }

    const flags = h.keyboardFlags();
    const result = evaluateKeyboardEvent(
      ev,
      h.applicationCursorKeys(),
      h.isMac,
      h.macOptionIsMeta,
      flags,
      ev.repeat ? 2 : 1, // フラグ 2 が無ければ evaluateKeyboardEvent が無視する
    );

    if (result.type === "pageUp" || result.type === "pageDown") {
      h.scrollPages(result.type === "pageUp" ? -1 : 1);
      this.cancel(ev);
      return false;
    }
    if (this.isThirdLevelShift(ev)) return true;
    if (result.cancel) this.cancel(ev);
    if (!result.key) return true;

    // 大文字は keypress で処理する（CapsLock のとき小文字が打てない macOS の IME への対策）。
    // kitty の `CSI` の形で送るキーは keypress を通らないので、ここで送る
    if (result.key === ev.key && hasNoCommandModifier(ev) && ev.key.length === 1) {
      const c = ev.key.charCodeAt(0);
      if (c >= 65 && c <= 90) return true;
    }
    if (this.unprocessedDeadKey) {
      this.unprocessedDeadKey = false;
      return true;
    }
    // xterm.js と同じく、Ctrl+C と Enter のあとは `textarea` に字をためない
    if (
      result.key === "\x03" ||
      result.key === "\r" ||
      ev.keyCode === 13 ||
      (ev.ctrlKey && ev.keyCode === 67)
    ) {
      this.textarea.value = "";
    }

    this.rememberPress(ev, result.key, flags);
    h.send(result.key);
    this.cancel(ev);
    return false;
  }

  /** 押すを送るキーを覚える（離すを送るため）。 */
  private rememberPress(ev: KeyDownLike, key: string, flags: number): void {
    // アプリケーションカーソルキーの `ESC O A` などもここに入れる
    if (flags & KITTY_REPORT_EVENT_TYPES && (key.startsWith("\x1b[") || key.startsWith("\x1bO"))) {
      this.reported.add(keyId(ev));
    }
    // keydown が字を送る場合（小文字、Ctrl+a の `\x01`、Alt+a の `ESC a` など）。
    // kitty は、押すを従来の形で送ったキーも、離すは `CSI <code>;<mods>:3u` で送る
    if (!ev.metaKey && [...ev.key].length === 1) {
      this.textPressed.add(keyId(ev));
    }
  }

  /** 押したままフォーカスを失うと keyup が来ないので、`textarea` の blur で呼ぶ。 */
  reset(): void {
    this.reported.clear();
    this.textPressed.clear();
  }

  keyUp(ev: KeyDownLike): void {
    this.keyDownSeen = false;
    const custom = this.host.customKeyHandler();
    if (custom && custom(ev as unknown as KeyboardEvent) === false) return;
    this.keyPressHandled = false;
    // 離す（kitty のフラグ 2）。押したのを `CSI` の形か、字として（keypress・input で）送ったキーだけ
    const id = keyId(ev);
    const wasReported = this.reported.delete(id);
    const wasText = this.textPressed.delete(id);
    if (!wasReported && !wasText) return;
    const h = this.host;
    const flags = h.keyboardFlags();
    if (!(flags & KITTY_REPORT_EVENT_TYPES)) return;
    // 変換の途中の keyup は送らない（変換の動きは変えない）
    if (this.composition.isComposing || ev.keyCode === 229) return;
    // 字を打つ修飾（macOS の Option・Windows の AltGr）。kitty は離すのとき、Option を Alt として
    // 送り、AltGr は修飾に出さない（Linux）。押すは字のまま送るので、ここは離すだけの扱い
    const thirdLevel = wasText && this.isThirdLevelShift(ev);
    const result = evaluateKeyboardEvent(
      !thirdLevel ? ev : h.isWindows ? override(ev, { ctrlKey: false, altKey: false }) : override(ev, { key: "" }),
      h.applicationCursorKeys(),
      // kittyKeyOf は macOS の Option を字のキーとして除く。離すは除かない。
      // Option で変わった字（å）ではなく、押したキー（a）の番号で送る（kitty と同じ）
      thirdLevel && h.isMac ? false : h.isMac,
      h.macOptionIsMeta,
      // 字のキーの離すは、フラグ 8 が無くても `CSI <code>;<mods>:3u`（kitty と同じ）。
      // evaluateKeyboardEvent は字のキーをフラグ 8 のときだけ `CSI` にするので、離すに限って足す。
      // Enter・Tab・Backspace は textPressed に入らず、CSI の形の離すは reported のキーだけ
      wasText ? flags | KITTY_REPORT_ALL_KEYS : flags,
      3,
    );
    if (result.key) h.send(result.key);
  }

  keyPress(
    ev: KeyDownLike & {
      readonly charCode: number;
      readonly which?: number | null;
    },
  ): boolean {
    this.keyPressHandled = false;
    const custom = this.host.customKeyHandler();
    if (custom && custom(ev as unknown as KeyboardEvent) === false) return false;
    // 送らないキー（Cmd を押したキーなど）は止めない。macOS の WKWebView は、keypress が止められると
    // キーを「ページが処理した」と見なし、メニューのキーの割り当て（Cmd+V の貼り付けなど）に送らない
    let key: number;
    if (ev.charCode) key = ev.charCode;
    else if (ev.which === null || ev.which === undefined) key = ev.keyCode;
    else if (ev.which !== 0 && ev.charCode !== 0) key = ev.which;
    else return false;
    if (!key || ((ev.altKey || ev.ctrlKey || ev.metaKey) && !this.isThirdLevelShift(ev))) {
      return false;
    }
    this.cancel(ev);
    this.host.send(String.fromCharCode(key));
    if (this.lastDown) this.textPressed.add(this.lastDown);
    this.keyPressHandled = true;
    this.unprocessedDeadKey = false;
    return true;
  }

  /** `input` イベント。絵文字の IME など、keydown・keypress を通らない字を送る。 */
  input(ev: {
    readonly data: string | null;
    readonly inputType: string;
    readonly composed: boolean;
    preventDefault(): void;
    stopPropagation(): void;
  }): boolean {
    if (
      ev.data &&
      ev.inputType === "insertText" &&
      (!ev.composed || !this.keyDownSeen)
    ) {
      if (this.keyPressHandled) return false;
      this.unprocessedDeadKey = false;
      this.host.send(ev.data);
      if (this.keyDownSeen && this.lastDown) this.textPressed.add(this.lastDown);
      this.cancel(ev);
      return true;
    }
    return false;
  }
}

/** `ev` の一部の値を差し替えて見せる。 */
function override(ev: KeyDownLike, values: Partial<Record<keyof KeyDownLike, unknown>>): KeyDownLike {
  return new Proxy(ev, {
    get(target, prop) {
      if (prop in values) return values[prop as keyof KeyDownLike];
      const v = Reflect.get(target, prop, target);
      return typeof v === "function" ? v.bind(target) : v;
    },
  });
}

/** Ctrl・Alt・Cmd のどれも押していない。 */
function hasNoCommandModifier(ev: KeyEventLike): boolean {
  return !ev.ctrlKey && !ev.altKey && !ev.metaKey;
}

/** 押したキーと離したキーを結びつける印。 */
function keyId(ev: KeyEventLike): string {
  return ev.code || String(ev.keyCode);
}

/** `navigator` から、macOS か・Windows かを決める（xterm.js の `Browser.isMac`・`isWindows`）。 */
export function detectPlatform(): { isMac: boolean; isWindows: boolean } {
  const nav = (globalThis as { navigator?: Navigator }).navigator;
  const platform = nav?.platform ?? "";
  const ua = nav?.userAgent ?? "";
  return {
    isMac: ["Macintosh", "MacIntel", "MacPPC", "Mac68K"].includes(platform),
    isWindows: ["Windows", "Win16", "Win32", "WinCE"].includes(platform) || /Windows/.test(ua),
  };
}
