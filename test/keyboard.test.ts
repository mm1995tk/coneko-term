import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { before, test } from "node:test";
import {
  bracketTextForPaste,
  CompositionHelper,
  evaluateKeyboardEvent,
  init,
  KeyHandler,
  prepareTextForTerminal,
  Terminal,
  type KeyEventLike,
} from "../dist/index.js";

before(async () => {
  await init(
    await readFile(
      new URL("../dist/wasm/coneko_term_wasm_bg.wasm", import.meta.url),
    ),
  );
});

function ev(keyCode: number, extra: Partial<KeyEventLike> = {}): KeyEventLike {
  return {
    keyCode,
    key: "",
    code: "",
    shiftKey: false,
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    ...extra,
  };
}

const ESC = "\x1b";
const key = (
  e: KeyEventLike,
  app = false,
  isMac = false,
  optMeta = false,
) => evaluateKeyboardEvent(e, app, isMac, optMeta).key;

test("keys: Enter, Backspace, Tab, Esc", () => {
  assert.equal(key(ev(13)), "\r");
  assert.equal(key(ev(13, { altKey: true })), ESC + "\r");
  assert.equal(key(ev(8)), "\x7f");
  assert.equal(key(ev(8, { ctrlKey: true })), "\b");
  assert.equal(key(ev(8, { altKey: true })), ESC + "\x7f");
  assert.equal(key(ev(9)), "\t");
  assert.equal(key(ev(9, { shiftKey: true })), ESC + "[Z");
  assert.equal(key(ev(27)), ESC);
  assert.equal(key(ev(27, { altKey: true })), ESC + ESC);
  assert.equal(evaluateKeyboardEvent(ev(13), false, false, false).cancel, true);
  assert.equal(evaluateKeyboardEvent(ev(8), false, false, false).cancel, false);
});

test("keys: arrows switch between CSI and SS3 with the cursor key mode", () => {
  const codes: [number, string][] = [
    [38, "A"],
    [40, "B"],
    [39, "C"],
    [37, "D"],
  ];
  for (const [code, l] of codes) {
    assert.equal(key(ev(code)), `${ESC}[${l}`);
    assert.equal(key(ev(code), true), `${ESC}O${l}`);
    // 修飾キーつきは、モードに関係なく CSI 1;m X
    assert.equal(key(ev(code, { shiftKey: true }), true), `${ESC}[1;2${l}`);
    assert.equal(key(ev(code, { altKey: true })), `${ESC}[1;3${l}`);
    assert.equal(key(ev(code, { ctrlKey: true })), `${ESC}[1;5${l}`);
    assert.equal(
      key(ev(code, { shiftKey: true, ctrlKey: true })),
      `${ESC}[1;6${l}`,
    );
    assert.equal(key(ev(code, { metaKey: true })), undefined);
  }
});

test("keys: Home, End, Insert, Delete, PageUp, PageDown", () => {
  assert.equal(key(ev(36)), `${ESC}[H`);
  assert.equal(key(ev(36), true), `${ESC}OH`);
  assert.equal(key(ev(36, { ctrlKey: true })), `${ESC}[1;5H`);
  assert.equal(key(ev(35)), `${ESC}[F`);
  assert.equal(key(ev(35), true), `${ESC}OF`);
  assert.equal(key(ev(35, { shiftKey: true })), `${ESC}[1;2F`);
  assert.equal(key(ev(45)), `${ESC}[2~`);
  assert.equal(key(ev(45, { shiftKey: true })), undefined);
  assert.equal(key(ev(45, { ctrlKey: true })), undefined);
  assert.equal(key(ev(46)), `${ESC}[3~`);
  assert.equal(key(ev(46, { altKey: true })), `${ESC}[3;3~`);
  assert.equal(key(ev(33)), `${ESC}[5~`);
  assert.equal(key(ev(34)), `${ESC}[6~`);
  assert.equal(key(ev(33, { ctrlKey: true })), `${ESC}[5;5~`);
  assert.equal(key(ev(34, { ctrlKey: true })), `${ESC}[6;5~`);
  // Shift+PageUp・PageDown は表示位置を動かす
  assert.deepEqual(evaluateKeyboardEvent(ev(33, { shiftKey: true }), false, false, false), {
    type: "pageUp",
    cancel: false,
    key: undefined,
  });
  assert.equal(
    evaluateKeyboardEvent(ev(34, { shiftKey: true }), false, false, false).type,
    "pageDown",
  );
});

test("keys: F1 to F12, with and without modifiers", () => {
  const plain = [
    "OP", "OQ", "OR", "OS", "[15~", "[17~", "[18~", "[19~", "[20~", "[21~", "[23~", "[24~",
  ];
  plain.forEach((s, i) => assert.equal(key(ev(112 + i)), ESC + s));
  const shifted = [
    "[1;2P", "[1;2Q", "[1;2R", "[1;2S", "[15;2~", "[17;2~", "[18;2~", "[19;2~", "[20;2~", "[21;2~", "[23;2~", "[24;2~",
  ];
  shifted.forEach((s, i) =>
    assert.equal(key(ev(112 + i, { shiftKey: true })), ESC + s),
  );
  assert.equal(key(ev(112, { ctrlKey: true, altKey: true })), `${ESC}[1;7P`);
  assert.equal(key(ev(116, { ctrlKey: true })), `${ESC}[15;5~`);
});

test("keys: Ctrl + letter, Alt as an ESC prefix, plain characters", () => {
  assert.equal(key(ev(65, { ctrlKey: true })), "\x01");
  assert.equal(key(ev(67, { ctrlKey: true })), "\x03");
  assert.equal(key(ev(90, { ctrlKey: true })), "\x1a");
  assert.equal(key(ev(32, { ctrlKey: true })), "\0");
  assert.equal(key(ev(51, { ctrlKey: true })), ESC);
  assert.equal(key(ev(52, { ctrlKey: true })), "\x1c");
  assert.equal(key(ev(53, { ctrlKey: true })), "\x1d");
  assert.equal(key(ev(54, { ctrlKey: true })), "\x1e");
  assert.equal(key(ev(55, { ctrlKey: true })), "\x1f");
  assert.equal(key(ev(56, { ctrlKey: true })), "\x7f");
  assert.equal(key(ev(219, { ctrlKey: true })), ESC);
  assert.equal(key(ev(220, { ctrlKey: true })), "\x1c");
  assert.equal(key(ev(221, { ctrlKey: true })), "\x1d");
  assert.equal(key(ev(50, { ctrlKey: true, shiftKey: true, key: "@" })), "\0");
  assert.equal(key(ev(189, { ctrlKey: true, shiftKey: true, key: "_" })), "\x1f");
  assert.equal(key(ev(65, { altKey: true })), ESC + "a");
  assert.equal(key(ev(65, { altKey: true, shiftKey: true })), ESC + "A");
  assert.equal(key(ev(65, { altKey: true, ctrlKey: true })), ESC + "\x01");
  assert.equal(key(ev(32, { altKey: true })), ESC + " ");
  assert.equal(key(ev(49, { altKey: true })), ESC + "1");
  assert.equal(key(ev(49, { altKey: true, shiftKey: true })), ESC + "!");
  assert.equal(key(ev(190, { altKey: true })), ESC + ".");
  assert.equal(key(ev(65, { key: "a" })), "a");
  assert.equal(key(ev(65, { key: "A", shiftKey: true })), "A");
  assert.equal(key(ev(49, { key: "1" })), "1");
  // 1 字にならないキー（NumLock など）は送らない
  assert.equal(key(ev(144, { key: "NumLock" })), undefined);
  assert.equal(key(ev(16, { key: "Shift", shiftKey: true })), undefined);
});

test("keys: on macOS, Option types a character unless macOptionIsMeta; Cmd is not sent", () => {
  // 既定: Option は字を打つキー。ESC の前置きにしない
  assert.equal(key(ev(65, { altKey: true }), false, true, false), undefined);
  // macOptionIsMeta: ESC の前置き
  assert.equal(key(ev(65, { altKey: true }), false, true, true), ESC + "a");
  assert.equal(key(ev(65, { metaKey: true, key: "a" }), false, true), undefined);
  assert.equal(key(ev(67, { metaKey: true, key: "c" }), false, true), undefined);
  assert.equal(key(ev(37, { metaKey: true }), false, true), undefined);
  // Alt+N などの dead key
  assert.equal(
    key(ev(229, { altKey: true, key: "Dead", code: "KeyN" }), false, false),
    ESC + "n",
  );
});

test("keys: iOS arrows", () => {
  assert.equal(key(ev(0, { key: "UIKeyInputUpArrow" })), `${ESC}[A`);
  assert.equal(key(ev(0, { key: "UIKeyInputLeftArrow" }), true), `${ESC}OD`);
});

test("paste: newlines become CR, and bracketed paste wraps the text", () => {
  assert.equal(prepareTextForTerminal("a\nb\r\nc\rd"), "a\rb\rc\rd");
  assert.equal(bracketTextForPaste("x", false), "x");
  assert.equal(bracketTextForPaste("x", true), `${ESC}[200~x${ESC}[201~`);
});

const dec = new TextDecoder();

test("Terminal.paste goes out through onData, bracketed when 2004 is on", async () => {
  const t = new Terminal({ cols: 10, rows: 3 });
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  t.paste("a\nb");
  await new Promise<void>((r) => t.write("\x1b[?2004h", r));
  t.paste("one\r\ntwo\nthree");
  await new Promise<void>((r) => t.write("\x1b[?2004l", r));
  t.paste("x\ny");
  t.paste("");
  assert.deepEqual(out, [
    "a\rb",
    `${ESC}[200~one\rtwo\rthree${ESC}[201~`,
    "x\ry",
  ]);
  t.dispose();
});

test("Terminal.paste returns to the bottom when scrolled back", async () => {
  const t = new Terminal({ cols: 5, rows: 2, scrollback: 10 });
  await new Promise<void>((r) => t.write("a\r\nb\r\nc\r\nd", r));
  t.scrollToTop();
  assert.equal(t.isAtBottom, false);
  t.paste("x");
  assert.equal(t.isAtBottom, true);
  t.dispose();
});

// ---- KeyHandler と CompositionHelper（DOM の代わりに偽物を使う）

function makeHandler(opts: { isMac?: boolean; macOptionIsMeta?: boolean; app?: boolean; flags?: number } = {}) {
  const sent: string[] = [];
  const pages: number[] = [];
  const timers: (() => void)[] = [];
  const textarea = { value: "", style: {} as Record<string, string> as never };
  const viewStyle = {} as Record<string, string>;
  const view = {
    textContent: "" as string | null,
    style: viewStyle as never,
    shown: false,
    getBoundingClientRect: () => ({ width: 30, height: 20 }),
    show() {
      this.shown = true;
    },
    hide() {
      this.shown = false;
    },
  };
  const composition = new CompositionHelper(
    textarea,
    view,
    (d) => sent.push(d),
    () => ({ left: 16, top: 40, cellHeight: 20 }),
    () => ({ family: "monospace", size: 15 }),
    (f) => timers.push(f),
  );
  const handler = new KeyHandler(
    {
      send: (d) => sent.push(d),
      applicationCursorKeys: () => opts.app ?? false,
      keyboardFlags: () => opts.flags ?? 0,
      scrollPages: (n) => pages.push(n),
      scrollToBottom: () => {},
      isMac: opts.isMac ?? false,
      isWindows: false,
      macOptionIsMeta: opts.macOptionIsMeta ?? false,
      customKeyHandler: () => undefined,
    },
    textarea,
    composition,
  );
  const run = () => {
    while (timers.length) timers.shift()!();
  };
  return { handler, composition, sent, pages, textarea, view, run };
}

function domEvent(keyCode: number, extra: Partial<KeyEventLike> = {}) {
  const e = {
    ...ev(keyCode, extra),
    prevented: false,
    preventDefault() {
      this.prevented = true;
    },
    stopPropagation() {},
  };
  return e;
}

test("KeyHandler sends the key and cancels the event; Shift+PageUp scrolls", () => {
  const h = makeHandler();
  const e = domEvent(13);
  h.handler.keyDown(e);
  assert.deepEqual(h.sent, ["\r"]);
  assert.equal(e.prevented, true);
  h.handler.keyDown(domEvent(65, { key: "a" }));
  assert.deepEqual(h.sent, ["\r", "a"]);
  h.handler.keyDown(domEvent(33, { shiftKey: true }));
  assert.deepEqual(h.pages, [-1]);
  assert.equal(h.sent.length, 2);
});

test("KeyHandler leaves capital letters to keypress, and Cmd keys to the app", () => {
  const h = makeHandler({ isMac: true });
  h.handler.keyDown(domEvent(65, { key: "A", shiftKey: true }));
  assert.deepEqual(h.sent, []);
  h.handler.keyPress({ ...domEvent(65, { key: "A" }), charCode: 65 });
  assert.deepEqual(h.sent, ["A"]);
  const cmdC = domEvent(67, { key: "c", metaKey: true });
  assert.equal(h.handler.keyDown(cmdC), true);
  assert.equal(cmdC.prevented, false);
  assert.deepEqual(h.sent, ["A"]);
  h.handler.keyUp(domEvent(65));
  // Option は字を打つキー。keydown では送らず、字は input で来る
  const opt = domEvent(65, { key: "å", altKey: true });
  h.handler.keyDown(opt);
  assert.deepEqual(h.sent, ["A"]);
  h.handler.input({
    data: "å",
    inputType: "insertText",
    composed: false,
    preventDefault() {},
    stopPropagation() {},
  });
  assert.deepEqual(h.sent, ["A", "å"]);
});

test("composition: nothing is sent while converting, the confirmed text is sent once", () => {
  const h = makeHandler();
  h.composition.compositionstart();
  assert.equal(h.view.shown, true);
  // 変換中のキー（keyCode 229）は何も送らない
  assert.equal(h.handler.keyDown(domEvent(229, { key: "Process" })), false);
  h.textarea.value = "に";
  h.composition.compositionupdate("に");
  assert.equal(h.view.textContent, "に");
  assert.equal(h.view.style["left"], "16px");
  assert.equal(h.view.style["top"], "40px");
  assert.equal(h.textarea.style["left"], "16px");
  assert.equal(h.textarea.style["width"], "30px");
  h.run();
  h.handler.keyDown(domEvent(229, { key: "Process" }));
  h.textarea.value = "日本語";
  h.composition.compositionupdate("日本語");
  h.run();
  assert.deepEqual(h.sent, []);
  h.composition.compositionend();
  assert.equal(h.view.shown, false);
  assert.deepEqual(h.sent, []);
  h.run();
  assert.deepEqual(h.sent, ["日本語"]);
  assert.equal(h.composition.isComposing, false);
});

test("composition: Enter while converting confirms the text first", () => {
  const h = makeHandler();
  h.composition.compositionstart();
  h.textarea.value = "かな";
  h.composition.compositionupdate("かな");
  h.run();
  h.handler.keyDown(domEvent(13));
  // 確定した字が先、Enter が後
  assert.deepEqual(h.sent, ["かな", "\r"]);
  h.run();
  assert.deepEqual(h.sent, ["かな", "\r"]);
});

test("composition: a Cmd key while converting confirms the text, then is left to the app", () => {
  const h = makeHandler({ isMac: true });
  h.composition.compositionstart();
  h.textarea.value = "かな";
  h.composition.compositionupdate("かな");
  h.run();
  const e = domEvent(67, { key: "c", metaKey: true });
  assert.equal(h.handler.keyDown(e), true);
  assert.deepEqual(h.sent, ["かな"]);
  assert.equal(e.prevented, false);
});

// ---- kitty keyboard protocol: フラグ 1（disambiguate escape codes） ----

const kkey = (e: KeyEventLike, flags = 1, isMac = false, optMeta = false) =>
  evaluateKeyboardEvent(e, false, isMac, optMeta, flags).key;

test("kitty flag 1: Esc and modified Esc", () => {
  assert.equal(kkey(ev(27)), `${ESC}[27u`);
  assert.equal(kkey(ev(27, { shiftKey: true })), `${ESC}[27;2u`);
  assert.equal(kkey(ev(27, { altKey: true })), `${ESC}[27;3u`);
});

test("kitty flag 1: Ctrl/Alt/Super with text keys use the unshifted code", () => {
  assert.equal(kkey(ev(65, { ctrlKey: true })), `${ESC}[97;5u`); // Ctrl+A
  assert.equal(kkey(ev(65, { altKey: true })), `${ESC}[97;3u`);
  assert.equal(kkey(ev(65, { altKey: true, shiftKey: true })), `${ESC}[97;4u`);
  assert.equal(kkey(ev(65, { ctrlKey: true, altKey: true })), `${ESC}[97;7u`);
  assert.equal(kkey(ev(65, { metaKey: true })), `${ESC}[97;9u`);
  assert.equal(kkey(ev(32, { ctrlKey: true })), `${ESC}[32;5u`);
  assert.equal(kkey(ev(49, { ctrlKey: true })), `${ESC}[49;5u`); // Ctrl+1
  assert.equal(kkey(ev(49, { altKey: true, shiftKey: true })), `${ESC}[49;4u`); // Alt+Shift+1
  assert.equal(kkey(ev(219, { ctrlKey: true })), `${ESC}[91;5u`); // Ctrl+[
});

test("kitty flag 1: plain and Shift-only text keys stay text", () => {
  assert.equal(kkey(ev(65, { key: "a" })), "a");
  assert.equal(kkey(ev(65, { key: "A", shiftKey: true })), "A");
  assert.equal(kkey(ev(32, { key: " " })), kkey(ev(32, { key: " " }), 0));
});

test("kitty flag 1: Enter, Tab, Backspace stay legacy unless modified", () => {
  assert.equal(kkey(ev(13)), "\r");
  assert.equal(kkey(ev(9)), "\t");
  assert.equal(kkey(ev(8)), "\x7f");
  assert.equal(kkey(ev(13, { ctrlKey: true })), `${ESC}[13;5u`);
  assert.equal(kkey(ev(13, { altKey: true })), `${ESC}[13;3u`);
  assert.equal(kkey(ev(9, { shiftKey: true })), `${ESC}[9;2u`);
  assert.equal(kkey(ev(8, { ctrlKey: true })), `${ESC}[127;5u`);
});

test("kitty flag 1: F3 avoids CSI R; other functional keys are unchanged", () => {
  assert.equal(kkey(ev(114)), `${ESC}[13~`);
  assert.equal(kkey(ev(114, { shiftKey: true })), `${ESC}[13;2~`);
  for (const k of [ev(37), ev(38, { ctrlKey: true }), ev(36), ev(46), ev(112), ev(115), ev(116), ev(45)]) {
    assert.equal(kkey(k), kkey(k, 0));
  }
});

test("kitty flag 1: macOS Option without macOptionIsMeta stays a text key", () => {
  assert.equal(kkey(ev(65, { altKey: true }), 1, true, false), undefined);
  assert.equal(kkey(ev(65, { altKey: true }), 1, true, true), `${ESC}[97;3u`);
});

test("flag 0 leaves the legacy bytes unchanged", () => {
  assert.equal(kkey(ev(27), 0), ESC);
  assert.equal(kkey(ev(65, { ctrlKey: true }), 0), "\x01");
  assert.equal(kkey(ev(114), 0), `${ESC}OR`);
  // 対応していないフラグだけが立っていても変わらない
  assert.equal(kkey(ev(27), 2), ESC);
});

test("KeyHandler sends the kitty form when the terminal's flag is 1", () => {
  const h = makeHandler({ flags: 1 });
  h.handler.keyDown(domEvent(27));
  h.handler.keyDown(domEvent(65, { ctrlKey: true }));
  assert.deepEqual(h.sent, [`${ESC}[27u`, `${ESC}[97;5u`]);
});

// ---- kitty keyboard protocol: フラグ 2・4・8・16 ----
// 期待値は仕様（https://sw.kovidgoyal.net/kitty/keyboard-protocol/ ）の形。
// フラグ: 1 disambiguate、2 event types、4 alternate keys、8 all keys、16 text

const F1 = 1;
const F2 = 2;
const F4 = 4;
const F8 = 8;
const F16 = 16;
const PRESS = 1;
const REPEAT = 2;
const RELEASE = 3;

const ek = (
  e: KeyEventLike,
  flags: number,
  type: 1 | 2 | 3 = PRESS,
  app = false,
) => evaluateKeyboardEvent(e, app, false, false, flags, type).key;

// ブラウザーが出す形のイベント（key・code つき）
const letter = (ch: string, extra: Partial<KeyEventLike> = {}) =>
  ev(ch.toUpperCase().charCodeAt(0), {
    key: extra.shiftKey ? ch.toUpperCase() : ch,
    code: `Key${ch.toUpperCase()}`,
    ...extra,
  });
const ENTER = ev(13, { key: "Enter", code: "Enter" });
const TAB = ev(9, { key: "Tab", code: "Tab" });
const BACKSPACE = ev(8, { key: "Backspace", code: "Backspace" });
const ESCAPE = ev(27, { key: "Escape", code: "Escape" });
const UP = ev(38, { key: "ArrowUp", code: "ArrowUp" });
const SHIFT_L = ev(16, { key: "Shift", code: "ShiftLeft", shiftKey: true });

test("kitty flag 2: event types on keys that are already escape codes", () => {
  // 押す・繰り返し・離す。押すは `1` を省く
  assert.equal(ek(UP, F2, PRESS), `${ESC}[A`);
  assert.equal(ek(UP, F2, REPEAT), `${ESC}[1;1:2A`);
  assert.equal(ek(UP, F2, RELEASE), `${ESC}[1;1:3A`);
  assert.equal(ek({ ...UP, ctrlKey: true }, F2, RELEASE), `${ESC}[1;5:3A`);
  assert.equal(ek(ev(46, { key: "Delete", code: "Delete" }), F2, REPEAT), `${ESC}[3;1:2~`);
  assert.equal(ek(ev(112, { key: "F1", code: "F1" }), F2, RELEASE), `${ESC}[1;1:3P`);
  assert.equal(ek(ev(116, { key: "F5", code: "F5" }, ), F2, RELEASE), `${ESC}[15;1:3~`);
  assert.equal(ek(ev(114, { key: "F3", code: "F3" }), F2, REPEAT), `${ESC}[13;1:2~`);
  // 押すは、フラグ 8 が無ければ今までの送り方のまま（SS3 も使う）
  assert.equal(ek(ev(112), F2, PRESS), `${ESC}OP`);
  assert.equal(ek(UP, F2, PRESS, true), `${ESC}OA`);
  // フラグ 1 のときの Esc・Ctrl+字は、繰り返し・離すも `CSI u`
  assert.equal(ek(ESCAPE, F1 | F2, REPEAT), `${ESC}[27;1:2u`);
  assert.equal(ek(ESCAPE, F1 | F2, RELEASE), `${ESC}[27;1:3u`);
  assert.equal(ek(letter("a", { ctrlKey: true }), F1 | F2, RELEASE), `${ESC}[97;5:3u`);
  assert.equal(ek(letter("a", { ctrlKey: true }), F1 | F2, REPEAT), `${ESC}[97;5:2u`);
});

test("kitty flag 2: text keys, Enter, Tab and Backspace are not released without flag 8", () => {
  assert.equal(ek(letter("a"), F2, PRESS), "a");
  assert.equal(ek(letter("a"), F2, REPEAT), "a");
  assert.equal(ek(letter("a"), F2, RELEASE), undefined);
  assert.equal(ek(letter("a", { shiftKey: true }), F1 | F2, RELEASE), undefined);
  for (const k of [ENTER, TAB, BACKSPACE]) {
    assert.equal(ek(k, F1 | F2, RELEASE), undefined);
    assert.equal(ek({ ...k, ctrlKey: true }, F1 | F2, RELEASE), undefined);
  }
  assert.equal(ek(ENTER, F1 | F2, REPEAT), "\r");
  // 修飾キーだけは、フラグ 8 が無ければ何も送らない
  assert.equal(ek(SHIFT_L, F1 | F2, PRESS), undefined);
  assert.equal(ek(SHIFT_L, F1 | F2, RELEASE), undefined);
  // フラグ 2 が無いと、離すも繰り返しも押すと同じ
  assert.equal(ek(UP, F1, RELEASE), `${ESC}[A`);
  assert.equal(ek(UP, 0, REPEAT), `${ESC}[A`);
});

test("kitty flag 2: keys the terminal keeps for itself are not reported", () => {
  const shiftPageUp = ev(33, { key: "PageUp", code: "PageUp", shiftKey: true });
  assert.equal(ek(shiftPageUp, F2, RELEASE), undefined);
  assert.equal(evaluateKeyboardEvent(shiftPageUp, false, false, false, F2, REPEAT).type, "pageUp");
  assert.equal(ek(ev(45, { ctrlKey: true, code: "Insert" }), F2, RELEASE), undefined);
  assert.equal(ek({ ...UP, metaKey: true }, F2, RELEASE), undefined);
});

test("kitty flag 8: every key is an escape code", () => {
  assert.equal(ek(letter("a"), F8), `${ESC}[97u`);
  assert.equal(ek(letter("a", { shiftKey: true }), F8), `${ESC}[97;2u`);
  assert.equal(ek(letter("a", { ctrlKey: true }), F8), `${ESC}[97;5u`);
  assert.equal(ek(ev(32, { key: " ", code: "Space" }), F8), `${ESC}[32u`);
  assert.equal(ek(ev(49, { key: "1", code: "Digit1" }), F8), `${ESC}[49u`);
  assert.equal(ek(ev(49, { key: "!", code: "Digit1", shiftKey: true }), F8), `${ESC}[49;2u`);
  assert.equal(ek(ENTER, F8), `${ESC}[13u`);
  assert.equal(ek(TAB, F8), `${ESC}[9u`);
  assert.equal(ek({ ...TAB, shiftKey: true }, F8), `${ESC}[9;2u`);
  assert.equal(ek(BACKSPACE, F8), `${ESC}[127u`);
  assert.equal(ek(ESCAPE, F8), `${ESC}[27u`);
  // CapsLock は、押した字ではなくキーそのもの
  assert.equal(ek(letter("a", { key: "A" }), F8), `${ESC}[97u`);
  // 機能キーは今までの終端。アプリケーションカーソルキーでも `CSI`、F1〜F4 も `CSI`
  assert.equal(ek(UP, F8, PRESS, true), `${ESC}[A`);
  assert.equal(ek({ ...UP, shiftKey: true }, F8), `${ESC}[1;2A`);
  assert.equal(ek(ev(112, { key: "F1", code: "F1" }), F8), `${ESC}[P`);
  assert.equal(ek(ev(114, { key: "F3", code: "F3" }), F8), `${ESC}[13~`);
  assert.equal(ek(ev(116, { key: "F5", code: "F5" }), F8), `${ESC}[15~`);
  assert.equal(ek(ev(124, { key: "F13", code: "F13" }), F8), `${ESC}[57376u`);
  assert.equal(ek(ev(20, { key: "CapsLock", code: "CapsLock" }), F8), `${ESC}[57358u`);
  assert.equal(ek(ev(19, { key: "Pause", code: "Pause" }), F8), `${ESC}[57362u`);
});

test("kitty flag 8: modifier keys alone and the keypad", () => {
  assert.equal(ek(SHIFT_L, F8), `${ESC}[57441;2u`);
  assert.equal(ek(SHIFT_L, F8 | F2, REPEAT), `${ESC}[57441;2:2u`);
  assert.equal(ek({ ...SHIFT_L, shiftKey: false }, F8 | F2, RELEASE), `${ESC}[57441;1:3u`);
  assert.equal(
    ek(ev(17, { key: "Control", code: "ControlLeft", ctrlKey: true }), F8),
    `${ESC}[57442;5u`,
  );
  assert.equal(
    ek(ev(18, { key: "Alt", code: "AltRight", altKey: true }), F8),
    `${ESC}[57449;3u`,
  );
  assert.equal(
    ek(ev(91, { key: "Meta", code: "MetaLeft", metaKey: true }), F8),
    `${ESC}[57444;9u`,
  );
  assert.equal(ek(ev(13, { key: "Enter", code: "NumpadEnter" }), F8), `${ESC}[57414u`);
  assert.equal(ek(ev(101, { key: "5", code: "Numpad5" }), F8), `${ESC}[57404u`);
  assert.equal(ek(ev(107, { key: "+", code: "NumpadAdd" }), F8), `${ESC}[57413u`);
  // NumLock が切れたテンキーの矢印
  assert.equal(ek(ev(37, { key: "ArrowLeft", code: "Numpad4" }), F8), `${ESC}[57417u`);
  // フラグ 8 が無ければ、修飾キーだけ・テンキーは今まで通り
  assert.equal(ek(SHIFT_L, F1), undefined);
  assert.equal(ek(ev(13, { key: "Enter", code: "NumpadEnter" }), F1), "\r");
});

test("kitty flag 8: flag 1 or 2 without 8 does not escape plain text", () => {
  assert.equal(ek(letter("a"), F1 | F2 | F4 | F16), "a");
  assert.equal(ek(letter("a", { shiftKey: true }), F1 | F2 | F4 | F16), "A");
});

test("kitty flag 16: the text the key makes", () => {
  const all = F8 | F16;
  assert.equal(ek(letter("a"), all), `${ESC}[97;;97u`);
  assert.equal(ek(letter("a", { shiftKey: true }), all), `${ESC}[97;2;65u`);
  assert.equal(ek(ev(32, { key: " ", code: "Space" }), all), `${ESC}[32;;32u`);
  assert.equal(ek(ev(49, { key: "!", code: "Digit1", shiftKey: true }), all), `${ESC}[49;2;33u`);
  // 現在の配列の字。キーの番号はその字、ベースのレイアウトの字は別
  assert.equal(ek(ev(50, { key: "é", code: "Digit2" }), all), `${ESC}[233;;233u`);
  // 絵文字などの 1 コードポイント
  assert.equal(ek(ev(0, { key: "😀", code: "" }), all), `${ESC}[128512;;128512u`);
  // 繰り返しは字つき、離すは字なし
  assert.equal(ek(letter("a"), all | F2, REPEAT), `${ESC}[97;1:2;97u`);
  assert.equal(ek(letter("a"), all | F2, RELEASE), `${ESC}[97;1:3u`);
  // 字を作らないキーには添えない
  assert.equal(ek(ENTER, all), `${ESC}[13u`);
  assert.equal(ek(ESCAPE, all), `${ESC}[27u`);
  assert.equal(ek(UP, all), `${ESC}[A`);
  assert.equal(ek(SHIFT_L, all), `${ESC}[57441;2u`);
  assert.equal(ek(letter("a", { ctrlKey: true }), all), `${ESC}[97;5u`);
  assert.equal(ek(letter("a", { altKey: true }), all), `${ESC}[97;3u`);
  assert.equal(ek(ev(101, { key: "5", code: "Numpad5" }), all), `${ESC}[57404;;53u`);
  // フラグ 8 が無ければ、字は今まで通りそのまま
  assert.equal(ek(letter("a"), F16), "a");
  assert.equal(ek(letter("a", { ctrlKey: true }), F1 | F16), `${ESC}[97;5u`);
});

test("kitty flag 4: shifted key and base layout key", () => {
  assert.equal(ek(letter("a", { shiftKey: true }), F8 | F4), `${ESC}[97:65;2u`);
  assert.equal(ek(letter("a"), F8 | F4), `${ESC}[97u`);
  assert.equal(ek(letter("a", { ctrlKey: true, shiftKey: true }), F1 | F4), `${ESC}[97:65;6u`);
  assert.equal(
    ek(ev(49, { key: "!", code: "Digit1", shiftKey: true, ctrlKey: true }), F1 | F4),
    `${ESC}[49:33;6u`,
  );
  // ロシア語配列の Ctrl+с（US 配列の C の位置）: 番号は с(1089)、ベースは c(99)
  assert.equal(
    ek(ev(67, { key: "с", code: "KeyC", ctrlKey: true }), F1 | F4),
    `${ESC}[1089::99;5u`,
  );
  // Shift つきなら、Shift をかけた字(С = 1057)も入る
  assert.equal(
    ek(ev(67, { key: "С", code: "KeyC", ctrlKey: true, shiftKey: true }), F1 | F4),
    `${ESC}[1089:1057:99;6u`,
  );
  // フラグ 4 が無ければ入らない。番号は同じ
  assert.equal(ek(ev(67, { key: "с", code: "KeyC", ctrlKey: true }), F1), `${ESC}[1089;5u`);
  // 機能キーとフラグ 4 だけでは変わらない
  assert.equal(ek(UP, F4), `${ESC}[A`);
  assert.equal(ek(letter("a", { shiftKey: true }), F4), "A");
});

test("kitty flags 2|4|8|16 together", () => {
  const all = F2 | F4 | F8 | F16;
  const shiftA = letter("a", { shiftKey: true });
  assert.equal(ek(shiftA, all, PRESS), `${ESC}[97:65;2;65u`);
  assert.equal(ek(shiftA, all, REPEAT), `${ESC}[97:65;2:2;65u`);
  assert.equal(ek(shiftA, all, RELEASE), `${ESC}[97:65;2:3u`);
  assert.equal(ek(letter("a"), all, RELEASE), `${ESC}[97;1:3u`);
  assert.equal(ek(ENTER, all, RELEASE), `${ESC}[13;1:3u`);
  assert.equal(ek(BACKSPACE, all, REPEAT), `${ESC}[127;1:2u`);
  assert.equal(ek(UP, all, RELEASE), `${ESC}[1;1:3A`);
  assert.equal(ek(ev(0, { key: "Unidentified" }), all, PRESS), undefined);
  assert.equal(ek(ev(0, { key: "Unidentified" }), all, RELEASE), undefined);
});

test("kitty flags: eventType is ignored without flag 2", () => {
  assert.equal(ek(letter("a"), F8, REPEAT), `${ESC}[97u`);
  assert.equal(ek(letter("a"), F8, RELEASE), `${ESC}[97u`);
});

// ---- KeyHandler（keyup と IME） ----

test("KeyHandler sends repeat and release with flag 2, only for keys it reported", () => {
  const h = makeHandler({ flags: F1 | F2 | F8 | F16 });
  h.handler.keyDown(domEvent(65, { key: "a", code: "KeyA" }));
  h.handler.keyDown(domEvent(65, { key: "a", code: "KeyA", repeat: true }));
  h.handler.keyUp(domEvent(65, { key: "a", code: "KeyA" }));
  assert.deepEqual(h.sent, [`${ESC}[97;;97u`, `${ESC}[97;1:2;97u`, `${ESC}[97;1:3u`]);
  // 押したのを送っていないキーの離すは送らない
  h.handler.keyUp(domEvent(66, { key: "b", code: "KeyB" }));
  assert.equal(h.sent.length, 3);
  // 大文字（Shift）も keypress を待たずに送る。キーを離したときに Shift は付かない
  h.sent.length = 0;
  h.handler.keyDown(domEvent(65, { key: "A", code: "KeyA", shiftKey: true }));
  h.handler.keyUp(domEvent(65, { key: "a", code: "KeyA" }));
  assert.deepEqual(h.sent, [`${ESC}[97;2;65u`, `${ESC}[97;1:3u`]);
});

test("KeyHandler reports modifier keys alone with flag 8", () => {
  const h = makeHandler({ flags: F2 | F8 });
  h.handler.keyDown(domEvent(16, { key: "Shift", code: "ShiftLeft", shiftKey: true }));
  h.handler.keyUp(domEvent(16, { key: "Shift", code: "ShiftLeft" }));
  assert.deepEqual(h.sent, [`${ESC}[57441;2u`, `${ESC}[57441;1:3u`]);
});

test("KeyHandler: without flag 2 a release sends nothing; flag 0 is unchanged", () => {
  const h = makeHandler({ flags: F8 });
  h.handler.keyDown(domEvent(65, { key: "a", code: "KeyA" }));
  h.handler.keyUp(domEvent(65, { key: "a", code: "KeyA" }));
  assert.deepEqual(h.sent, [`${ESC}[97u`]);
  const z = makeHandler();
  z.handler.keyDown(domEvent(65, { key: "a", code: "KeyA" }));
  z.handler.keyUp(domEvent(65, { key: "a", code: "KeyA" }));
  assert.deepEqual(z.sent, ["a"]);
});

for (const flags of [F8, F1 | F2 | F4 | F8 | F16]) {
  test(`IME: converting and confirming is not changed by kitty flags (${flags})`, () => {
    const h = makeHandler({ flags });
    h.composition.compositionstart();
    // 変換中のキー（keyCode 229）は何も送らず、離しも送らない
    assert.equal(h.handler.keyDown(domEvent(229, { key: "Process", code: "KeyN" })), false);
    h.handler.keyUp(domEvent(229, { key: "Process", code: "KeyN" }));
    h.textarea.value = "に";
    h.composition.compositionupdate("に");
    h.run();
    h.handler.keyDown(domEvent(229, { key: "Process", code: "KeyH" }));
    h.textarea.value = "日本語";
    h.composition.compositionupdate("日本語");
    h.run();
    // 変換中は修飾キーだけも送らない
    h.handler.keyDown(domEvent(16, { key: "Shift", code: "ShiftLeft", shiftKey: true }));
    assert.deepEqual(h.sent, []);
    h.composition.compositionend();
    assert.deepEqual(h.sent, []);
    h.run();
    // 確定した字は、そのままの文字で 1 回だけ（CSI u にしない）
    assert.deepEqual(h.sent, ["日本語"]);
    // 確定の Enter の keyup は、押したのを送っていないので何も送らない
    h.handler.keyUp(domEvent(13, { key: "Enter", code: "Enter" }));
    assert.deepEqual(h.sent, ["日本語"]);
  });

  test(`IME: Enter while converting confirms first, then Enter is the escape code (${flags})`, () => {
    const h = makeHandler({ flags });
    h.composition.compositionstart();
    h.textarea.value = "かな";
    h.composition.compositionupdate("かな");
    h.run();
    h.handler.keyDown(domEvent(13, { key: "Enter", code: "Enter" }));
    assert.deepEqual(h.sent, ["かな", `${ESC}[13u`]);
    h.run();
    assert.deepEqual(h.sent, ["かな", `${ESC}[13u`]);
  });
}
