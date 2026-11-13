import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { before, test } from "node:test";
import { init, Terminal } from "../dist/index.js";

before(async () => {
  await init(
    await readFile(
      new URL("../dist/wasm/coneko_term_wasm_bg.wasm", import.meta.url),
    ),
  );
});

function write(t: Terminal, data: string | Uint8Array): Promise<void> {
  return new Promise((resolve) => t.write(data, resolve));
}

/** `onData` に出たバイト列を文字列（1 バイト 1 字）で集める。 */
function collect(t: Terminal): { out: string[] } {
  const box = { out: [] as string[] };
  t.onData((d) => box.out.push(String.fromCharCode(...d)));
  return box;
}

const mouse = (
  type: "down" | "up" | "move" | "leave",
  col: number,
  row: number,
  extra: object = {},
) => ({ type, button: 0, col, row, ...extra });

// ---- マウスの報告

test("mouse: no report unless the app asked for one", async () => {
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  assert.equal(t.mouseReporting, false);
  t.handleMouse(mouse("down", 1, 1));
  t.handleMouse(mouse("up", 1, 1));
  assert.deepEqual(box.out, []);
  t.dispose();
});

test("mouse: SGR press, drag and release (1002 + 1006)", async () => {
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  await write(t, "\x1b[?1002h\x1b[?1006h");
  assert.equal(t.mouseReporting, true);
  assert.equal(t.handleMouse(mouse("down", 2, 1)), true);
  t.handleMouse(mouse("move", 3, 1));
  t.handleMouse(mouse("move", 3, 1)); // 同じ位置は続けて報告しない
  t.handleMouse(mouse("up", 4, 1));
  assert.deepEqual(box.out, [
    "\x1b[<0;3;2M",
    "\x1b[<32;4;2M",
    "\x1b[<0;5;2m",
  ]);
  // 押さずに動かしても 1002 は報告しない
  box.out.length = 0;
  t.handleMouse(mouse("move", 0, 0));
  assert.deepEqual(box.out, []);
  t.dispose();
});

test("mouse: 1003 reports motion without a button", async () => {
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  await write(t, "\x1b[?1003h\x1b[?1006h");
  t.handleMouse(mouse("move", 5, 2));
  assert.deepEqual(box.out, ["\x1b[<35;6;3M"]);
  t.dispose();
});

test("mouse: default encoding, modifiers and the right button", async () => {
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  await write(t, "\x1b[?1000h");
  t.handleMouse({ ...mouse("down", 0, 0), button: 2, ctrlKey: true });
  t.handleMouse({ ...mouse("up", 0, 0), button: 2, ctrlKey: true });
  // ボタン 2 + Ctrl(16) + 32、離したときは 3 + 16 + 32
  assert.deepEqual(box.out, ["\x1b[M2!!", "\x1b[M3!!"]);
  t.dispose();
});

test("mouse: X10 (mode 9) reports presses only", async () => {
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  await write(t, "\x1b[?9h");
  t.handleMouse(mouse("down", 0, 0));
  t.handleMouse(mouse("up", 0, 0));
  assert.deepEqual(box.out, ["\x1b[M !!"]);
  t.dispose();
});

test("mouse: SGR pixel encoding (1016) reports pixels", async () => {
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  await write(t, "\x1b[?1000h\x1b[?1016h");
  t.handleMouse(mouse("down", 1, 1, { x: 17.9, y: 33 }));
  assert.deepEqual(box.out, ["\x1b[<0;17;33M"]);
  t.dispose();
});

test("mouse: positions outside the screen are clamped to its edge", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const box = collect(t);
  await write(t, "\x1b[?1000h\x1b[?1006h");
  t.handleMouse(mouse("down", 99, -5));
  assert.deepEqual(box.out, ["\x1b[<0;10;1M"]);
  t.dispose();
});

test("mouse: default encoding drops positions that do not fit in a byte", async () => {
  const t = new Terminal({ cols: 300, rows: 4 });
  const box = collect(t);
  await write(t, "\x1b[?1000h");
  t.handleMouse(mouse("down", 250, 0));
  t.handleMouse(mouse("up", 250, 0));
  assert.deepEqual(box.out, []);
  t.handleMouse(mouse("down", 10, 0));
  assert.equal(box.out.length, 1);
  t.dispose();
});

test("mouse: Shift forces selection even while the app is listening", async () => {
  const t = new Terminal({ cols: 20, rows: 3 });
  const box = collect(t);
  await write(t, "hello world\x1b[?1000h\x1b[?1006h");
  t.handleMouse(mouse("down", 0, 0, { shiftKey: true }));
  t.handleMouse(mouse("move", 4, 0, { shiftKey: true }));
  t.handleMouse(mouse("up", 4, 0, { shiftKey: true }));
  assert.deepEqual(box.out, []);
  assert.equal(t.getSelection(), "hello");
  t.dispose();
});

test("mouse: a press the app owns keeps going to the app after Shift changes", async () => {
  const t = new Terminal({ cols: 20, rows: 3 });
  const box = collect(t);
  await write(t, "\x1b[?1002h\x1b[?1006h");
  t.handleMouse(mouse("down", 0, 0));
  t.handleMouse(mouse("move", 1, 0, { shiftKey: true }));
  t.handleMouse(mouse("up", 1, 0));
  assert.equal(box.out.length, 3);
  assert.equal(t.hasSelection(), false);
  t.dispose();
});

// ---- ホイール

test("wheel: reported as a wheel event when the app listens", async () => {
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  await write(t, "\x1b[?1000h\x1b[?1006h");
  assert.equal(t.handleWheel({ deltaY: -1, deltaMode: 1, col: 3, row: 2 }), true);
  assert.equal(t.handleWheel({ deltaY: 1, deltaMode: 1, col: 3, row: 2 }), true);
  assert.deepEqual(box.out, ["\x1b[<64;4;3M", "\x1b[<65;4;3M"]);
  t.dispose();
});

test("wheel: reports once per line, and small pixel deltas add up", async () => {
  // 描画が無いので、行の高さは 1 ピクセルとして数える
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  await write(t, "\x1b[?1000h\x1b[?1006h");
  // 1 行に満たない間は報告しない
  for (let i = 0; i < 3; i++) {
    assert.equal(t.handleWheel({ deltaY: 0.3, deltaMode: 0, col: 0, row: 0 }), true);
  }
  assert.deepEqual(box.out, []);
  // 合計が 1 行に届いたら 1 つ。端数（0.2）は次に回る
  t.handleWheel({ deltaY: 0.3, deltaMode: 0, col: 0, row: 0 });
  assert.deepEqual(box.out, ["\x1b[<65;1;1M"]);
  box.out.length = 0;
  // 行単位: 3 行で 3 つ
  t.handleWheel({ deltaY: -3, deltaMode: 1, col: 0, row: 0 });
  assert.equal(box.out.join(""), "\x1b[<64;1;1M".repeat(3));
  box.out.length = 0;
  // ページ単位: 画面の行数（5）だけ
  t.handleWheel({ deltaY: 1, deltaMode: 2, col: 0, row: 0 });
  assert.equal(box.out.join(""), "\x1b[<65;1;1M".repeat(5));
  box.out.length = 0;
  // Shift つきでも報告する
  t.handleWheel({ deltaY: 2, deltaMode: 1, col: 0, row: 0, shiftKey: true });
  assert.equal(box.out.join(""), "\x1b[<69;1;1M".repeat(2));
  t.dispose();
});

test("wheel: moves the viewport on the normal screen", async () => {
  const t = new Terminal({ cols: 10, rows: 3 });
  await write(t, Array.from({ length: 10 }, (_, i) => `l${i}`).join("\r\n"));
  assert.equal(t.isAtBottom, true);
  const bottom = t.viewportY;
  assert.equal(t.handleWheel({ deltaY: -3, deltaMode: 1 }), true);
  assert.equal(t.viewportY, bottom - 3);
  t.handleWheel({ deltaY: 1, deltaMode: 1 });
  assert.equal(t.viewportY, bottom - 2);
  // Shift つきは動かさない
  t.handleWheel({ deltaY: -3, deltaMode: 1, shiftKey: true });
  assert.equal(t.viewportY, bottom - 2);
  // ページ単位
  t.handleWheel({ deltaY: 1, deltaMode: 2 });
  assert.equal(t.viewportY, bottom);
  t.dispose();
});

test("wheel: no scrollback means the page may scroll", () => {
  const t = new Terminal({ cols: 10, rows: 3 });
  assert.equal(t.handleWheel({ deltaY: 5, deltaMode: 1 }), false);
  t.dispose();
});

test("wheel: arrow keys on the alternate screen without mouse reporting", async () => {
  const t = new Terminal({ cols: 10, rows: 3 });
  const box = collect(t);
  await write(t, "\x1b[?1049h");
  t.handleWheel({ deltaY: -2, deltaMode: 1 });
  t.handleWheel({ deltaY: 1, deltaMode: 1 });
  assert.deepEqual(box.out, ["\x1b[A\x1b[A", "\x1b[B"]);
  box.out.length = 0;
  await write(t, "\x1b[?1h");
  t.handleWheel({ deltaY: -1, deltaMode: 1 });
  assert.deepEqual(box.out, ["\x1bOA"]);
  t.dispose();
});

test("wheel: on the alternate screen, mouse reporting wins over arrow keys", async () => {
  const t = new Terminal({ cols: 10, rows: 3 });
  const box = collect(t);
  await write(t, "\x1b[?1049h\x1b[?1000h\x1b[?1006h");
  t.handleWheel({ deltaY: -1, deltaMode: 1 });
  assert.deepEqual(box.out, ["\x1b[<64;1;1M"]);
  t.dispose();
});

// ---- 選択

/** 左ボタンで `(c0, r0)` から `(c1, r1)` までドラッグする。 */
function drag(t: Terminal, c0: number, r0: number, c1: number, r1: number) {
  t.handleMouse(mouse("down", c0, r0));
  t.handleMouse(mouse("move", c1, r1));
  t.handleMouse(mouse("up", c1, r1));
}

test("selection: drag selects the range, both ends included", async () => {
  const t = new Terminal({ cols: 20, rows: 3 });
  await write(t, "hello world\r\nsecond line");
  assert.equal(t.hasSelection(), false);
  drag(t, 6, 0, 10, 0);
  assert.equal(t.getSelection(), "world");
  assert.deepEqual(t.getSelectionRange(), {
    start: { row: 0, col: 6 },
    end: { row: 0, col: 11 },
  });
  // 逆向きでも同じ
  drag(t, 10, 0, 6, 0);
  assert.equal(t.getSelection(), "world");
  // 複数行
  drag(t, 6, 0, 5, 1);
  assert.equal(t.getSelection(), "world\nsecond");
  t.dispose();
});

test("selection: a click without moving selects nothing and clears the old one", async () => {
  const t = new Terminal({ cols: 20, rows: 3 });
  await write(t, "hello world");
  drag(t, 0, 0, 4, 0);
  assert.equal(t.getSelection(), "hello");
  t.handleMouse(mouse("down", 2, 0));
  t.handleMouse(mouse("up", 2, 0));
  assert.equal(t.hasSelection(), false);
  assert.equal(t.getSelection(), "");
  t.dispose();
});

test("selection: double click selects a word, drag extends by words", async () => {
  const t = new Terminal({ cols: 30, rows: 3 });
  await write(t, "foo bar baz");
  t.handleMouse(mouse("down", 5, 0, { clickCount: 2 }));
  assert.equal(t.getSelection(), "bar");
  t.handleMouse(mouse("move", 9, 0));
  assert.equal(t.getSelection(), "bar baz");
  t.handleMouse(mouse("up", 9, 0));
  assert.equal(t.getSelection(), "bar baz");
  t.handleMouse(mouse("down", 1, 0, { clickCount: 2 }));
  t.handleMouse(mouse("move", 5, 0));
  t.handleMouse(mouse("up", 5, 0));
  assert.equal(t.getSelection(), "foo bar");
  t.dispose();
});

test("selection: wordSeparator option changes the word boundaries", async () => {
  const t = new Terminal({ cols: 30, rows: 3, wordSeparator: " /" });
  await write(t, "ls /usr/local/bin");
  t.handleMouse(mouse("down", 7, 0, { clickCount: 2 }));
  assert.equal(t.getSelection(), "/");
  t.handleMouse(mouse("up", 7, 0));
  t.handleMouse(mouse("down", 5, 0, { clickCount: 2 }));
  assert.equal(t.getSelection(), "usr");
  t.dispose();
});

test("selection: triple click selects the line, wrapped lines included", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  await write(t, "abcdefghijklmno\r\nsecond");
  // 1 行目は 2 行に折り返している
  t.handleMouse(mouse("down", 2, 1, { clickCount: 3 }));
  t.handleMouse(mouse("up", 2, 1));
  assert.equal(t.getSelection(), "abcdefghijklmno");
  t.handleMouse(mouse("down", 2, 2, { clickCount: 3 }));
  assert.equal(t.getSelection(), "second");
  // 続けてドラッグすると行ごとに広がる
  t.handleMouse(mouse("move", 2, 0));
  assert.equal(t.getSelection(), "abcdefghijklmno\nsecond");
  t.dispose();
});

test("selection: a wrapped line is joined without a newline", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  await write(t, "abcdefghijklmno\r\nsecond");
  drag(t, 8, 0, 3, 1);
  assert.equal(t.getSelection(), "ijklmn");
  t.dispose();
});

test("selection: Shift+click extends the selection", async () => {
  const t = new Terminal({ cols: 30, rows: 3 });
  await write(t, "0123456789");
  drag(t, 2, 0, 4, 0);
  assert.equal(t.getSelection(), "234");
  t.handleMouse(mouse("down", 7, 0, { shiftKey: true }));
  t.handleMouse(mouse("up", 7, 0, { shiftKey: true }));
  assert.equal(t.getSelection(), "234567");
  t.handleMouse(mouse("down", 0, 0, { shiftKey: true }));
  t.handleMouse(mouse("up", 0, 0, { shiftKey: true }));
  assert.equal(t.getSelection(), "01234567");
  t.dispose();
});

test("selection: scrollback is selectable and follows the lines as they scroll", async () => {
  const t = new Terminal({ cols: 10, rows: 3, scrollback: 100 });
  await write(t, Array.from({ length: 8 }, (_, i) => `row${i}`).join("\r\n"));
  t.scrollToTop();
  drag(t, 0, 0, 3, 1);
  assert.equal(t.getSelection(), "row0\nrow1");
  t.scrollToBottom();
  assert.equal(t.getSelection(), "row0\nrow1");
  // 新しい行でスクロールバックが増えても、同じ字を選んだまま
  await write(t, "\r\nrow8\r\nrow9");
  assert.equal(t.getSelection(), "row0\nrow1");
  t.dispose();
});

test("selection: lines dropped from the scrollback shrink the selection", async () => {
  const t = new Terminal({ cols: 10, rows: 2, scrollback: 3 });
  await write(t, Array.from({ length: 6 }, (_, i) => `r${i}`).join("\r\n"));
  // スクロールバックは r1 r2 r3、画面は r4 r5
  t.scrollToTop();
  drag(t, 0, 0, 1, 1); // r1 r2
  assert.equal(t.getSelection(), "r1\nr2");
  await write(t, "\r\nr6"); // r1 を捨てる
  assert.equal(t.getSelection(), "r2");
  await write(t, "\r\nr7"); // r2 も捨てる
  assert.equal(t.hasSelection(), false);
  t.dispose();
});

test("selection: selectAll and clearSelection, with change events", async () => {
  const t = new Terminal({ cols: 10, rows: 3 });
  await write(t, "ab\r\ncd");
  let changes = 0;
  t.onSelectionChange(() => changes++);
  t.selectAll();
  assert.equal(t.getSelection(), "ab\ncd\n");
  assert.equal(changes, 1);
  t.selectAll();
  assert.equal(changes, 1);
  t.clearSelection();
  assert.equal(t.hasSelection(), false);
  assert.equal(changes, 2);
  t.dispose();
});

test("selection: cleared by resize, reset and a switch of screens", async () => {
  const t = new Terminal({ cols: 10, rows: 3 });
  await write(t, "hello");
  drag(t, 0, 0, 3, 0);
  t.resize(12, 3);
  assert.equal(t.hasSelection(), false);
  drag(t, 0, 0, 3, 0);
  await write(t, "\x1b[?1049h");
  assert.equal(t.hasSelection(), false);
  await write(t, "\x1b[Halt");
  drag(t, 0, 0, 2, 0);
  assert.equal(t.getSelection(), "alt");
  await write(t, "\x1b[?1049l");
  assert.equal(t.hasSelection(), false);
  drag(t, 0, 0, 3, 0);
  t.reset();
  assert.equal(t.hasSelection(), false);
  t.dispose();
});

test("selection: wide characters", async () => {
  const t = new Terminal({ cols: 10, rows: 2 });
  await write(t, "あいう えお");
  t.handleMouse(mouse("down", 1, 0, { clickCount: 2 }));
  assert.equal(t.getSelection(), "あいう");
  t.dispose();
});

test("selection: copySelection puts the text in the clipboard data", async () => {
  const t = new Terminal({ cols: 20, rows: 2 });
  await write(t, "copy me");
  const got: [string, string][] = [];
  const data = { setData: (f: string, d: string) => void got.push([f, d]) };
  assert.equal(t.copySelection(data), false);
  assert.deepEqual(got, []);
  drag(t, 0, 0, 3, 0);
  assert.equal(t.copySelection(data), true);
  assert.deepEqual(got, [["text/plain", "copy"]]);
  t.dispose();
});

test("selection: selectionSpan gives the columns to paint on each visible row", async () => {
  const t = new Terminal({ cols: 10, rows: 3 });
  await write(t, "one\r\ntwo\r\nthree");
  drag(t, 1, 0, 2, 2);
  assert.deepEqual(t.selectionSpan(0), [1, 10]);
  assert.deepEqual(t.selectionSpan(1), [0, 10]);
  assert.deepEqual(t.selectionSpan(2), [0, 3]);
  t.clearSelection();
  assert.equal(t.selectionSpan(0), undefined);
  t.dispose();
});

test("selection: selectionBackground is validated", () => {
  assert.throws(
    () => new Terminal({ theme: { selectionBackground: "red" } }),
    /selectionBackground/,
  );
  new Terminal({ theme: { selectionBackground: "#11223344" } }).dispose();
  new Terminal({ theme: { selectionBackground: "#abc" } }).dispose();
});

// ---- ポインターの形（OSC 22）

test("pointer shape: set, push, pop and query follow OSC 22", async () => {
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  assert.equal(t.pointerShape, undefined);
  await write(t, "\x1b]22;pointer\x1b\\");
  assert.equal(t.pointerShape, "pointer");
  await write(t, "\x1b]22;>wait\x1b\\");
  assert.equal(t.pointerShape, "wait");
  await write(t, "\x1b]22;<\x1b\\");
  assert.equal(t.pointerShape, "pointer");
  await write(t, "\x1b]22;?__current__\x1b\\\x1b]22;?pointer,nope\x1b\\");
  assert.equal(box.out.join(""), "\x1b]22;pointer\x1b\\\x1b]22;1,0\x1b\\");
  await write(t, "\x1b]22;bogus\x1b\\");
  assert.equal(t.pointerShape, "pointer");
  // マウスの報告が有効でも無効でも効く
  await write(t, "\x1b[?1000h\x1b]22;crosshair\x1b\\");
  assert.equal(t.mouseReporting, true);
  assert.equal(t.pointerShape, "crosshair");
  t.reset();
  assert.equal(t.pointerShape, undefined);
  t.dispose();
});

// ---- kitty: マウスが窓から出たこと

test("mouse: leaving the window is reported with SGR pixels and 1003", async () => {
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  await write(t, "\x1b[?1003h\x1b[?1016h");
  t.handleMouse(mouse("leave", 0, 0));
  assert.deepEqual(box.out, ["\x1b[<163;0;0M"]);
  t.dispose();
});

test("mouse: leaving is not reported in other modes", async () => {
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  t.handleMouse(mouse("leave", 0, 0));
  await write(t, "\x1b[?1003h\x1b[?1006h");
  t.handleMouse(mouse("leave", 0, 0));
  await write(t, "\x1b[?1000h\x1b[?1016h");
  t.handleMouse(mouse("leave", 0, 0));
  assert.deepEqual(box.out, []);
  t.dispose();
});

test("mouse: leaving while a button is held is not reported", async () => {
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  await write(t, "\x1b[?1003h\x1b[?1016h");
  t.handleMouse(mouse("down", 1, 1));
  box.out.length = 0;
  t.handleMouse(mouse("leave", 0, 0));
  assert.deepEqual(box.out, []);
  t.dispose();
});

// ---- 横のホイールと、一続きの動きの間の軸の固定

test("wheel: horizontal delta is reported as wheel left / right", async () => {
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  await write(t, "\x1b[?1000h\x1b[?1006h");
  assert.equal(t.handleWheel({ deltaX: -1, deltaY: 0, deltaMode: 1, col: 3, row: 2, time: 0 }), true);
  assert.equal(t.handleWheel({ deltaX: 2, deltaY: 0, deltaMode: 1, col: 3, row: 2, time: 1000 }), true);
  assert.equal(box.out.join(""), "\x1b[<66;4;3M" + "\x1b[<67;4;3M".repeat(2));
  t.dispose();
});

test("wheel: small horizontal pixel deltas add up by cell width", async () => {
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  await write(t, "\x1b[?1000h\x1b[?1006h");
  for (let i = 0; i < 3; i++) t.handleWheel({ deltaX: 0.3, deltaY: 0, deltaMode: 0, time: i });
  assert.deepEqual(box.out, []);
  t.handleWheel({ deltaX: 0.3, deltaY: 0, deltaMode: 0, time: 3 });
  assert.deepEqual(box.out, ["\x1b[<67;1;1M"]);
  t.dispose();
});

test("wheel: the axis is fixed by the first event until the events stop", async () => {
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  await write(t, "\x1b[?1000h\x1b[?1006h");
  // 横が大きい動き: 混ざった縦は捨てる
  t.handleWheel({ deltaX: 3, deltaY: 1, deltaMode: 1, time: 0 });
  t.handleWheel({ deltaX: 0, deltaY: 2, deltaMode: 1, time: 50 });
  t.handleWheel({ deltaX: 1, deltaY: 2, deltaMode: 1, time: 100 });
  assert.equal(box.out.join(""), "\x1b[<67;1;1M".repeat(4));
  box.out.length = 0;
  // 149ms 空いただけなら、まだ同じ動き
  t.handleWheel({ deltaX: 0, deltaY: 3, deltaMode: 1, time: 249 });
  assert.deepEqual(box.out, []);
  // 150ms 空いたら終わり。縦がすぐ効き、混ざった横は捨てる
  t.handleWheel({ deltaX: 1, deltaY: 2, deltaMode: 1, time: 399 });
  assert.equal(box.out.join(""), "\x1b[<65;1;1M".repeat(2));
  box.out.length = 0;
  // 同じ大きさなら縦
  t.handleWheel({ deltaX: 2, deltaY: 2, deltaMode: 1, time: 1000 });
  assert.equal(box.out.join(""), "\x1b[<65;1;1M".repeat(2));
  t.dispose();
});

test("wheel: the axis switches when the other axis stays clearly larger", async () => {
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  await write(t, "\x1b[?1000h\x1b[?1006h");
  // 横の動き。そのあと間を空けずに縦へ。縦がはっきり大きい（2 倍より大きい）イベントが 3 回続くまで捨てる
  t.handleWheel({ deltaX: 4, deltaY: 0, deltaMode: 1, time: 0 });
  t.handleWheel({ deltaX: 1, deltaY: 3, deltaMode: 1, time: 10 });
  t.handleWheel({ deltaX: 1, deltaY: 3, deltaMode: 1, time: 20 });
  // 縦は捨てて、横の分（4 + 1 + 1）だけ出る
  assert.equal(box.out.join(""), "\x1b[<67;1;1M".repeat(6));
  box.out.length = 0;
  t.handleWheel({ deltaX: 0, deltaY: 3, deltaMode: 1, time: 30 });
  assert.equal(box.out.join(""), "\x1b[<65;1;1M".repeat(3));
  box.out.length = 0;
  // 切り替わったあとは、縦の動きの中の横は捨てる。縦から横へも同じ
  t.handleWheel({ deltaX: 1, deltaY: 3, deltaMode: 1, time: 40 });
  assert.equal(box.out.join(""), "\x1b[<65;1;1M".repeat(3));
  box.out.length = 0;
  for (let i = 0; i < 3; i++) t.handleWheel({ deltaX: 5, deltaY: 1, deltaMode: 1, time: 50 + i });
  // 最初の 2 回は横を捨てて縦の分（1 + 1）、3 回目で切り替わって横の 5
  assert.equal(box.out.join(""), "\x1b[<65;1;1M".repeat(2) + "\x1b[<67;1;1M".repeat(5));
  t.dispose();
});

test("wheel: the axis does not switch for a little of the other axis", async () => {
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  await write(t, "\x1b[?1000h\x1b[?1006h");
  // 縦の動きに横が混ざる。横が続けて 3 回はっきり大きくならない（2 倍ちょうども数えない）ので、切り替えない
  t.handleWheel({ deltaX: 0, deltaY: 3, deltaMode: 1, time: 0 });
  box.out.length = 0;
  const seq: Array<[number, number]> = [
    [4, 1],
    [4, 1],
    [1, 1],
    [4, 1],
    [4, 1],
    [2, 1],
    [4, 1],
    [4, 1],
    [1, 3],
  ];
  seq.forEach(([dx, dy], i) => t.handleWheel({ deltaX: dx, deltaY: dy, deltaMode: 1, time: 10 + i }));
  const lines = seq.reduce((n, [, dy]) => n + dy, 0);
  assert.equal(box.out.join(""), "\x1b[<65;1;1M".repeat(lines));
  t.dispose();
});

test("wheel: horizontal delta is ignored when the app does not listen", async () => {
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  assert.equal(t.handleWheel({ deltaX: 3, deltaY: 0, deltaMode: 1, time: 0 }), false);
  // 縦が大きい動きの横は捨て、縦は今まで通り
  await write(t, "\x1b[?1049h");
  assert.equal(t.handleWheel({ deltaX: 3, deltaY: 1, deltaMode: 1, time: 1000 }), false);
  assert.equal(t.handleWheel({ deltaX: 1, deltaY: 2, deltaMode: 1, time: 2000 }), true);
  assert.deepEqual(box.out, ["\x1b[B\x1b[B"]);
  t.dispose();
});

test("wheel: a fraction carries over between gestures on the same axis", async () => {
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  await write(t, "\x1b[?1000h\x1b[?1006h");
  t.handleWheel({ deltaY: 0.6, deltaMode: 0, time: 0 });
  t.handleWheel({ deltaY: 0.6, deltaMode: 0, time: 1000 });
  assert.deepEqual(box.out, ["\x1b[<65;1;1M"]);
  t.dispose();
});
