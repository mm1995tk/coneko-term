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

function makeHandler(opts: { isMac?: boolean; macOptionIsMeta?: boolean; app?: boolean } = {}) {
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
      scrollPages: (n) => pages.push(n),
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
  h.handler.keyUp();
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
