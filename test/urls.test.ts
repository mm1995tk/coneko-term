import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { before, test } from "node:test";
import { init, Terminal, type UrlClick } from "../dist/index.js";

before(async () => {
  await init(
    await readFile(
      new URL("../dist/wasm/coneko_term_wasm_bg.wasm", import.meta.url),
    ),
  );
});

function write(t: Terminal, data: string): Promise<void> {
  return new Promise((resolve) => t.write(data, resolve));
}

test("urls: finds http and https, trims trailing punctuation", async () => {
  const t = new Terminal({ cols: 60, rows: 5 });
  await write(t, "see https://example.com/a?b=1, and (http://x.org/p).\r\nftp://no.example\r\n");
  const urls = t.getVisibleUrls();
  assert.deepEqual(
    urls.map((u) => u.url),
    ["https://example.com/a?b=1", "http://x.org/p"],
  );
  assert.deepEqual(urls[0].segments, [{ row: 0, startCol: 4, endCol: 29 }]);
  t.dispose();
});

test("urls: stops at non-ASCII text", async () => {
  const t = new Terminal({ cols: 60, rows: 3 });
  await write(t, "https://example.com/pathを見て");
  assert.equal(t.getVisibleUrls()[0].url, "https://example.com/path");
  t.dispose();
});

test("urls: a wrapped URL is one URL with a segment per row", async () => {
  const t = new Terminal({ cols: 10, rows: 5 });
  await write(t, "https://example.com/abc");
  const urls = t.getVisibleUrls();
  assert.equal(urls.length, 1);
  assert.equal(urls[0].url, "https://example.com/abc");
  assert.deepEqual(urls[0].segments, [
    { row: 0, startCol: 0, endCol: 10 },
    { row: 1, startCol: 0, endCol: 10 },
    { row: 2, startCol: 0, endCol: 3 },
  ]);
  // マウスを乗せていなければ波線は無い。どの行に乗せても、URL 全体に出る
  assert.deepEqual(t.urlSpans(1), []);
  hover(t, 4, 1);
  assert.deepEqual(t.urlSpans(0), [[0, 10]]);
  assert.deepEqual(t.urlSpans(1), [[0, 10]]);
  assert.deepEqual(t.urlSpans(2), [[0, 3]]);
  t.underlineUrls = false;
  assert.deepEqual(t.urlSpans(1), []);
  t.dispose();
});

test("urls: a hard newline ends the URL", async () => {
  const t = new Terminal({ cols: 10, rows: 5 });
  await write(t, "https://a.b\r\nc.d");
  assert.equal(t.getVisibleUrls()[0].url, "https://a.b");
  t.dispose();
});

test("urls: click reports the URL and modifiers; drag does not", async () => {
  const t = new Terminal({ cols: 40, rows: 3 });
  await write(t, "go https://example.com now");
  const clicks: UrlClick[] = [];
  t.onUrlClick((e) => clicks.push(e));
  const ev = (type: "down" | "up", col: number, extra = {}) => ({
    type,
    button: 0,
    col,
    row: 0,
    ...extra,
  });
  t.handleMouse(ev("down", 5, { metaKey: true, ctrlKey: true }));
  t.handleMouse(ev("up", 5));
  assert.deepEqual(clicks, [
    {
      url: "https://example.com",
      shiftKey: false,
      altKey: false,
      ctrlKey: true,
      metaKey: true,
    },
  ]);
  t.handleMouse(ev("down", 5));
  t.handleMouse(ev("up", 8));
  t.handleMouse(ev("down", 0));
  t.handleMouse(ev("up", 0));
  assert.equal(clicks.length, 1);
  t.dispose();
});

test("urls: a URL deep in a logical line longer than 64 rows is found", async () => {
  const t = new Terminal({ cols: 10, rows: 80, scrollback: 0 });
  await write(t, "x".repeat(700) + " https://a.b/c");
  assert.deepEqual(
    t.getVisibleUrls().map((u) => u.url),
    ["https://a.b/c"],
  );
  hover(t, 2, 70);
  assert.equal(t.urlSpans(70).length, 1);
  t.dispose();
});

test("urls: a double click does not fire onUrlClick", async () => {
  const t = new Terminal({ cols: 40, rows: 3 });
  await write(t, "go https://example.com now");
  let n = 0;
  t.onUrlClick(() => n++);
  for (const clickCount of [1, 2]) {
    t.handleMouse({ type: "down", button: 0, col: 5, row: 0, clickCount });
    t.handleMouse({ type: "up", button: 0, col: 5, row: 0 });
  }
  assert.equal(n, 1);
  t.dispose();
});

test("urls: a short wrapped URL across row 63 is not cut", async () => {
  const t = new Terminal({ cols: 10, rows: 80, scrollback: 0 });
  await write(t, "\r\n".repeat(63) + "https://example.com/abc");
  const urls = t.getVisibleUrls();
  assert.equal(urls.length, 1);
  assert.equal(urls[0].url, "https://example.com/abc");
  assert.equal(urls[0].segments.length, 3);
  t.dispose();
});

const hover = (t: Terminal, col: number, row: number, extra = {}) =>
  t.handleMouse({ type: "move", button: 0, col, row, ...extra });

const PROMPT = "\x1b]133;A\x07$ \x1b]133;B\x07";
const OUTPUT = "\x1b]133;C\x07";

test("urls: no underline on the prompt line, still one on output lines", async () => {
  const t = new Terminal({ cols: 60, rows: 6 });
  await write(t, PROMPT + "echo https://typed.example\r\n" + OUTPUT);
  await write(t, "https://out.example\r\n\x1b]133;D;0\x07" + PROMPT);
  await write(t, "curl https://typing.example");
  // 乗せても、コマンドの行（打っている途中と、実行し終えたあと）には波線が出ない
  hover(t, 10, 0);
  assert.deepEqual(t.urlSpans(0), []);
  hover(t, 10, 2);
  assert.deepEqual(t.urlSpans(2), []);
  // 乗せていない出力の行には出ず、乗せると出る
  assert.deepEqual(t.urlSpans(1), []);
  hover(t, 3, 1);
  assert.deepEqual(t.urlSpans(1), [[0, 19]]);
  assert.deepEqual(
    t.getVisibleUrls().map((u) => u.url),
    ["https://out.example"],
  );
  // 打っている途中の URL は、クリックでも開かない
  const clicks: UrlClick[] = [];
  t.onUrlClick((e) => clicks.push(e));
  t.handleMouse({ type: "down", button: 0, col: 12, row: 2 });
  t.handleMouse({ type: "up", button: 0, col: 12, row: 2 });
  assert.equal(clicks.length, 0);
  t.dispose();
});

test("urls: a multi-line command and a wrapped prompt line have no underline", async () => {
  const t = new Terminal({ cols: 10, rows: 6 });
  await write(t, PROMPT + "https://example.com/abc\r\n> https://b.example");
  assert.equal(t.getVisibleUrls().length, 0);
  await write(t, "\r\n" + OUTPUT + "https://example.com/x");
  assert.equal(t.getVisibleUrls().length, 1);
  t.dispose();
});

test("urls: the prompt mark is kept when the screen is resized", async () => {
  const t = new Terminal({ cols: 40, rows: 4 });
  await write(t, PROMPT + "https://a.example/long/path\r\n" + OUTPUT);
  t.resize(10, 4);
  assert.equal(t.getVisibleUrls().length, 0);
  t.dispose();
});

test("urls: without OSC 133 marks, every line is underlined as before", async () => {
  const t = new Terminal({ cols: 40, rows: 3 });
  await write(t, "$ echo https://example.com");
  hover(t, 10, 0);
  assert.equal(t.urlSpans(0).length, 1);
  t.dispose();
});

test("urls: the marks are tracked even when the host registers OSC 133", async () => {
  const t = new Terminal({ cols: 40, rows: 3 });
  t.registerOscHandler(133, () => {});
  await write(t, PROMPT + "https://example.com");
  hover(t, 5, 0);
  assert.deepEqual(t.urlSpans(0), []);
  t.dispose();
});

test("urls: hoveringUrl is true only with cmd held over a URL", async () => {
  const t = new Terminal({ cols: 40, rows: 3 });
  await write(t, "go https://example.com now");
  const move = (col: number, extra = {}) =>
    t.handleMouse({ type: "move", button: 0, col, row: 0, ...extra });
  move(5);
  assert.equal(t.hoveringUrl, false);
  move(5, { metaKey: true });
  assert.equal(t.hoveringUrl, true);
  move(0, { metaKey: true });
  assert.equal(t.hoveringUrl, false);
  move(5, { metaKey: true });
  t.handleMouse({ type: "leave", button: 0, col: 5, row: 0 });
  assert.equal(t.hoveringUrl, false);
  t.dispose();
});

test("urls: hoveringUrl needs Shift while the app reports the mouse", async () => {
  const t = new Terminal({ cols: 40, rows: 3 });
  await write(t, "go https://example.com now\x1b[?1000h");
  const move = (extra = {}) =>
    t.handleMouse({ type: "move", button: 0, col: 5, row: 0, metaKey: true, ...extra });
  move();
  assert.equal(t.hoveringUrl, false);
  move({ shiftKey: true });
  assert.equal(t.hoveringUrl, true);
  t.dispose();
});

test("urls: drops an incomplete percent-encoding at the end", async () => {
  const t = new Terminal({ cols: 60, rows: 10 });
  await write(
    t,
    "https://example.com/%\r\nhttps://example.com/%2\r\nhttps://example.com/a%20b\r\n",
  );
  assert.deepEqual(
    t.getVisibleUrls().map((u) => u.url),
    ["https://example.com/", "https://example.com/", "https://example.com/a%20b"],
  );
  t.dispose();
});

test("urls: incomplete percent-encoding combined with other trimming", async () => {
  const t = new Terminal({ cols: 60, rows: 10 });
  await write(
    t,
    "https://example.com/%.\r\nhttps://example.com/(a%)\r\nhttps://example.com/a%20%\r\n",
  );
  assert.deepEqual(
    t.getVisibleUrls().map((u) => u.url),
    ["https://example.com/", "https://example.com/(a%)", "https://example.com/a%20"],
  );
  t.dispose();
});

test("urls: the underline follows the mouse and goes away on leave", async () => {
  const t = new Terminal({ cols: 40, rows: 3 });
  await write(t, "go https://example.com now\r\nhttps://b.example");
  assert.deepEqual(t.urlSpans(0), []);
  hover(t, 5, 0);
  assert.deepEqual(t.urlSpans(0), [[3, 22]]);
  assert.deepEqual(t.urlSpans(1), []);
  hover(t, 0, 1);
  assert.deepEqual(t.urlSpans(0), []);
  assert.deepEqual(t.urlSpans(1), [[0, 17]]);
  hover(t, 30, 1);
  assert.deepEqual(t.urlSpans(1), []);
  hover(t, 0, 1);
  t.handleMouse({ type: "leave", button: 0, col: 0, row: 1 });
  assert.deepEqual(t.urlSpans(1), []);
  t.dispose();
});

test("urls: the underline needs no cmd, but cmd decides the pointer", async () => {
  const t = new Terminal({ cols: 40, rows: 3 });
  await write(t, "go https://example.com now");
  hover(t, 5, 0);
  assert.equal(t.urlSpans(0).length, 1);
  assert.equal(t.hoveringUrl, false);
  hover(t, 5, 0, { metaKey: true });
  assert.equal(t.urlSpans(0).length, 1);
  assert.equal(t.hoveringUrl, true);
  t.dispose();
});

test("urls: the command line is not underlined, not opened, and its output is", async () => {
  const t = new Terminal({ cols: 60, rows: 5 });
  await write(
    t,
    PROMPT + "printf 'https://example.com/'\r\n" + OUTPUT + "https://example.com/\r\n\x1b]133;D;0\x07" + PROMPT,
  );
  const clicks: UrlClick[] = [];
  t.onUrlClick((e) => clicks.push(e));
  hover(t, 12, 0, { metaKey: true });
  assert.deepEqual(t.urlSpans(0), []);
  assert.equal(t.hoveringUrl, false);
  t.handleMouse({ type: "down", button: 0, col: 12, row: 0, metaKey: true });
  t.handleMouse({ type: "up", button: 0, col: 12, row: 0, metaKey: true });
  assert.equal(clicks.length, 0);
  hover(t, 3, 1, { metaKey: true });
  assert.deepEqual(t.urlSpans(1), [[0, 20]]);
  assert.equal(t.hoveringUrl, true);
  t.handleMouse({ type: "down", button: 0, col: 3, row: 1, metaKey: true });
  t.handleMouse({ type: "up", button: 0, col: 3, row: 1, metaKey: true });
  assert.equal(clicks.length, 1);
  t.dispose();
});
