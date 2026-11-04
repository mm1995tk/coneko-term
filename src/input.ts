// キーボードと IME の入力を受ける部分。xterm.js 6.0.0 の `CoreBrowserTerminal` の
// `_keyDown`・`_keyPress`・`_inputEvent` と `CompositionHelper` の動きに合わせる。
// DOM の要素そのものではなく、使う部分だけの形（`TextareaLike` など）に頼るので、DOM の無い
// テストでも動かせる。

import {
  evaluateKeyboardEvent,
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

  /** 変換中の字を、カーソルの位置へ動かす。`textarea` も同じ位置に置く（IME の候補の窓が出る場所）。 */
  updateCompositionElements(dontRecurse = false): void {
    if (!this.composing) return;
    const a = this.anchor();
    if (a) {
      const f = this.font();
      const vs = this.view.style;
      vs.left = `${a.left}px`;
      vs.top = `${a.top}px`;
      vs.height = `${a.cellHeight}px`;
      vs.lineHeight = `${a.cellHeight}px`;
      vs.fontFamily = f.family;
      vs.fontSize = `${f.size}px`;
      const b = this.view.getBoundingClientRect();
      const ts = this.textarea.style;
      ts.left = `${a.left}px`;
      ts.top = `${a.top}px`;
      // 1x1 より小さいと動かない IME がある
      ts.width = `${Math.max(b.width, 1)}px`;
      ts.height = `${Math.max(b.height, 1)}px`;
      ts.lineHeight = `${b.height}px`;
    }
    if (!dontRecurse) this.schedule(() => this.updateCompositionElements(true));
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
      flags & KITTY_REPORT_EVENT_TYPES && ev.repeat ? 2 : 1,
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
    if (
      result.key === ev.key &&
      !ev.ctrlKey &&
      !ev.altKey &&
      !ev.metaKey &&
      ev.key.length === 1
    ) {
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

    if (flags & KITTY_REPORT_EVENT_TYPES && result.key.startsWith("\x1b[")) {
      this.reported.add(keyId(ev));
    }
    h.send(result.key);
    this.cancel(ev);
    return false;
  }

  keyUp(ev: KeyDownLike): void {
    this.keyDownSeen = false;
    const custom = this.host.customKeyHandler();
    if (custom && custom(ev as unknown as KeyboardEvent) === false) return;
    this.keyPressHandled = false;
    // 離す（kitty のフラグ 2）。押したのを `CSI` の形で送ったキーだけ
    if (!this.reported.delete(keyId(ev))) return;
    const h = this.host;
    const flags = h.keyboardFlags();
    if (!(flags & KITTY_REPORT_EVENT_TYPES)) return;
    // 変換の途中の keyup は送らない（変換の動きは変えない）
    if (this.composition.isComposing || ev.keyCode === 229) return;
    const result = evaluateKeyboardEvent(
      ev,
      h.applicationCursorKeys(),
      h.isMac,
      h.macOptionIsMeta,
      flags,
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
    this.cancel(ev);
    let key: number;
    if (ev.charCode) key = ev.charCode;
    else if (ev.which === null || ev.which === undefined) key = ev.keyCode;
    else if (ev.which !== 0 && ev.charCode !== 0) key = ev.which;
    else return false;
    if (!key || ((ev.altKey || ev.ctrlKey || ev.metaKey) && !this.isThirdLevelShift(ev))) {
      return false;
    }
    this.host.send(String.fromCharCode(key));
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
      this.cancel(ev);
      return true;
    }
    return false;
  }
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
