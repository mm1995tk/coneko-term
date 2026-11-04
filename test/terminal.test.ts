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

/** `write` して、処理し終えるまで待つ。 */
function write(t: Terminal, data: string | Uint8Array): Promise<void> {
  return new Promise((resolve) => t.write(data, resolve));
}

const dec = new TextDecoder();

test("write reaches the screen", async () => {
  const t = new Terminal({ cols: 10, rows: 3 });
  await write(t, "hello\r\nworld");
  assert.equal(t.getRowText(0), "hello     ");
  assert.equal(t.getRowText(1), "world     ");
  assert.deepEqual(
    { x: t.cursor.x, y: t.cursor.y },
    { x: 5, y: 1 },
  );
  assert.equal(t.cols, 10);
  assert.equal(t.rows, 3);
  t.dispose();
});

test("write accepts bytes, and a character split across writes", async () => {
  const t = new Terminal({ cols: 4, rows: 1 });
  const bytes = new TextEncoder().encode("あ");
  t.write(bytes.subarray(0, 1));
  await write(t, bytes.subarray(1));
  assert.equal(t.getRowText(0), "あ  ");
  const row = t.readRow(0);
  assert.equal(row.cells[0], "あ".codePointAt(0));
  assert.equal(row.cells[1], 2);
  t.dispose();
});

test("done is called after the data is processed, in order", async () => {
  const t = new Terminal({ cols: 10, rows: 2 });
  const seen: string[] = [];
  const p1 = new Promise<void>((r) =>
    t.write("one", () => {
      seen.push("1:" + t.getRowText(0).trim());
      r();
    }),
  );
  const p2 = new Promise<void>((r) =>
    t.write("two", () => {
      seen.push("2:" + t.getRowText(0).trim());
      r();
    }),
  );
  assert.deepEqual(seen, []);
  await Promise.all([p1, p2]);
  assert.deepEqual(seen, ["1:one", "2:onetwo"]);
  t.dispose();
});

test("a large write is split so other tasks can run", async () => {
  const t = new Terminal({ cols: 80, rows: 24, scrollback: 100 });
  const line = "x".repeat(79) + "\r\n";
  const data = line.repeat(60_000); // 約 4.8 MB
  let finished = false;
  let ticks = 0;
  const done = new Promise<void>((r) =>
    t.write(data, () => {
      finished = true;
      r();
    }),
  );
  const timer = setInterval(() => {
    if (!finished) ticks++;
  }, 0);
  await done;
  clearInterval(timer);
  assert.ok(ticks > 0, "timers should run while the write is being processed");
  assert.equal(t.getRowText(0), "x".repeat(79) + " ");
  t.dispose();
});

test("DSR 6 is answered through onData", async () => {
  const t = new Terminal({ cols: 10, rows: 5 });
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  await write(t, "\r\n\r\nab\x1b[6n");
  assert.deepEqual(out, ["\x1b[3;3R"]);
  t.dispose();
});

test("DA1 and color queries use the theme", async () => {
  const t = new Terminal({ theme: { background: "#102030" } });
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  await write(t, "\x1b]11;?\x07");
  assert.deepEqual(out, ["\x1b]11;rgb:1010/2020/3030\x07"]);
  t.dispose();
});

test("input goes out through the same onData", () => {
  const t = new Terminal();
  const out: Uint8Array[] = [];
  t.onData((d) => out.push(d));
  t.input("ls\r");
  t.input(Uint8Array.of(1, 2, 3));
  assert.deepEqual(out.map((d) => [...d]), [[108, 115, 13], [1, 2, 3]]);
  t.dispose();
});

test("OSC handlers get registered numbers only", async () => {
  const t = new Terminal();
  const got: string[] = [];
  const titles: string[] = [];
  t.onTitleChange((s) => titles.push(s));
  const d = t.registerOscHandler(133, (s) => got.push(s));
  await write(t, "\x1b]133;A;日本\x07\x1b]2;title\x07\x1b]999;x\x07");
  assert.deepEqual(got, ["A;日本"]);
  assert.deepEqual(titles, ["title"]);
  d.dispose();
  await write(t, "\x1b]133;B\x07");
  assert.deepEqual(got, ["A;日本"]);
  t.dispose();
});

test("bell and focus reports", async () => {
  const t = new Terminal();
  let bells = 0;
  t.onBell(() => bells++);
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  await write(t, "\x07");
  assert.equal(bells, 1);
  t.setFocused(true);
  assert.deepEqual(out, []);
  await write(t, "\x1b[?1004h");
  t.setFocused(false);
  assert.deepEqual(out, ["\x1b[O"]);
  t.dispose();
});

test("dirty rows are reported once, and cells carry attributes", async () => {
  const t = new Terminal({ cols: 5, rows: 3 });
  t.takeDirtyRows();
  await write(t, "\x1b[2;1H\x1b[1;38;2;1;2;3mA\x1b[0mé");
  assert.deepEqual(t.takeDirtyRows(), [1]);
  assert.deepEqual(t.takeDirtyRows(), []);
  const row = t.readRow(1);
  assert.equal(row.cells[0], "A".codePointAt(0));
  assert.equal(row.cells[2], (2 << 24) | 0x010203);
  assert.equal(row.cells[4] & 1, 1);
  assert.equal(t.getRowText(1), "Aé   ");
  t.dispose();
});

test("scrollback, reset and resize", async () => {
  const t = new Terminal({ cols: 5, rows: 2, scrollback: 10 });
  await write(t, "a\r\nb\r\nc");
  assert.equal(t.scrollbackLength, 1);
  assert.equal(dec.decode(Uint8Array.of(t.readScrollbackRow(0).cells[0])), "a");
  t.resize(8, 4);
  assert.equal(t.cols, 8);
  assert.equal(t.rows, 4);
  t.reset();
  assert.equal(t.getRowText(0).trim(), "");
  t.dispose();
});

test("write after dispose is ignored", async () => {
  const t = new Terminal();
  t.dispose();
  let called = false;
  t.write("x", () => (called = true));
  await new Promise((r) => setTimeout(r, 5));
  assert.equal(called, false);
});
