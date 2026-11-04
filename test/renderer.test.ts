import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { before, test } from "node:test";
import {
  CanvasRenderer,
  init,
  Palette,
  proposeDimensions,
  SCROLLBAR_WIDTH,
  Terminal,
} from "../dist/index.js";

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

// ---- fit

test("fit: cols and rows come from the size, minus the scrollbar", () => {
  assert.deepEqual(
    proposeDimensions({
      width: 814,
      height: 480,
      cellWidth: 9,
      cellHeight: 20,
      scrollbarWidth: SCROLLBAR_WIDTH,
    }),
    // (814 - 14) / 9 = 88.88 -> 88, 480 / 20 = 24
    { cols: 88, rows: 24 },
  );
});

test("fit: rounds down, and does not subtract the scrollbar when there is none", () => {
  assert.deepEqual(
    proposeDimensions({
      width: 100,
      height: 59,
      cellWidth: 10,
      cellHeight: 20,
      scrollbarWidth: 0,
    }),
    { cols: 10, rows: 2 },
  );
});

test("fit: at least 2 columns and 1 row, as FitAddon does", () => {
  assert.deepEqual(
    proposeDimensions({
      width: 15,
      height: 5,
      cellWidth: 10,
      cellHeight: 20,
      scrollbarWidth: SCROLLBAR_WIDTH,
    }),
    { cols: 2, rows: 1 },
  );
});

test("fit: undefined when the size is unknown or the pane is hidden", () => {
  const base = {
    width: 800,
    height: 400,
    cellWidth: 8,
    cellHeight: 16,
    scrollbarWidth: SCROLLBAR_WIDTH,
  };
  assert.equal(proposeDimensions({ ...base, cellWidth: 0 }), undefined);
  assert.equal(proposeDimensions({ ...base, cellHeight: 0 }), undefined);
  assert.equal(proposeDimensions({ ...base, width: 0 }), undefined);
  assert.equal(proposeDimensions({ ...base, height: 0 }), undefined);
});

test("fit before open leaves the size as it is", () => {
  const t = new Terminal({ cols: 30, rows: 7 });
  assert.deepEqual(t.fit(), { cols: 30, rows: 7 });
  t.dispose();
});

// ---- palette

test("palette: 16 colors from the theme, then the 256-color cube and grays", () => {
  const ansi = Array.from({ length: 16 }, (_, i) => `#0000${i.toString(16)}0`);
  const p = new Palette(ansi, "#fff", "#000", "#abc");
  assert.equal(p.get(1), "#000010");
  assert.equal(p.get(16), "#000000");
  assert.equal(p.get(21), "#0000ff");
  assert.equal(p.get(196), "#ff0000");
  assert.equal(p.get(231), "#ffffff");
  assert.equal(p.get(232), "#080808");
  assert.equal(p.get(255), "#eeeeee");
  const word = (kind: number, v: number) => ((kind << 24) | v) >>> 0;
  assert.equal(p.resolve(0, false), "#fff");
  assert.equal(p.resolve(0, true), "#000");
  assert.equal(p.resolve(word(2, 0x123456), false), "#123456");
  assert.equal(p.resolve(word(2, 0x000056), false), "#000056");
  assert.equal(p.resolve(word(1, 2), false), p.get(2));
  // 太字の 0 から 7 は明るい色
  assert.equal(p.resolve(word(1, 2), false, true), p.get(10));
  assert.equal(p.resolve(word(1, 12), false, true), p.get(12));
});

// ---- viewport

test("viewport: stays at the bottom and follows new output", async () => {
  const t = new Terminal({ cols: 5, rows: 3, scrollback: 100 });
  await write(t, "1\r\n2\r\n3\r\n4\r\n5\r\n");
  assert.equal(t.isAtBottom, true);
  assert.equal(t.viewportY, t.scrollbackLength);
  await write(t, "6\r\n7\r\n");
  assert.equal(t.isAtBottom, true);
  assert.equal(t.viewportY, t.scrollbackLength);
  t.dispose();
});

test("viewport: keeps looking at the same lines when new output comes", async () => {
  const t = new Terminal({ cols: 5, rows: 3, scrollback: 100 });
  const lines = Array.from({ length: 10 }, (_, i) => `L${i}`).join("\r\n");
  await write(t, lines);
  t.scrollToTop();
  assert.equal(t.viewportY, 0);
  assert.equal(t.readViewportRow(0).cells[0], "L".codePointAt(0));
  const seen = t.viewportY;
  await write(t, "\r\nL10\r\nL11");
  assert.equal(t.viewportY, seen);
  assert.equal(t.isAtBottom, false);
  // 一番上の行はいまも L0
  const row = t.readViewportRow(0);
  assert.equal(String.fromCodePoint(row.cells[0], row.cells[6]), "L0");
  t.scrollToBottom();
  assert.equal(t.isAtBottom, true);
  t.dispose();
});

test("viewport: scrollLines is clamped, and onScroll reports changes", async () => {
  const t = new Terminal({ cols: 5, rows: 3, scrollback: 100 });
  await write(t, Array.from({ length: 10 }, (_, i) => `${i}`).join("\r\n"));
  const sb = t.scrollbackLength;
  const seen: number[] = [];
  t.onScroll((y) => seen.push(y));
  t.scrollLines(-2);
  assert.equal(t.viewportY, sb - 2);
  t.scrollLines(-1000);
  assert.equal(t.viewportY, 0);
  t.scrollLines(-1); // 変わらない
  t.scrollLines(1);
  assert.equal(t.viewportY, 1);
  t.scrollToLine(5);
  assert.equal(t.viewportY, 5);
  t.scrollLines(1000);
  assert.equal(t.viewportY, sb);
  assert.deepEqual(seen, [sb - 2, 0, 1, 5, sb]);
  t.dispose();
});

// ---- 描画

type Call = [string, ...unknown[]];

/** 呼び出しを記録する Canvas 2D の代わり。 */
function fakeCanvas() {
  const calls: Call[] = [];
  const state: Record<string, unknown> = {};
  const ctx = new Proxy(
    {},
    {
      get(_t, prop: string) {
        if (prop === "measureText") {
          return (text: string) => ({
            width: text.length * 9,
            fontBoundingBoxAscent: 12,
            fontBoundingBoxDescent: 4,
          });
        }
        if (prop in state) return state[prop];
        return (...args: unknown[]) => calls.push([prop, ...args]);
      },
      set(_t, prop: string, value) {
        state[prop] = value;
        calls.push([`set ${prop}`, value]);
        return true;
      },
    },
  );
  const canvas = {
    width: 0,
    height: 0,
    style: {} as Record<string, string>,
    getContext: () => ctx,
  };
  return { canvas: canvas as unknown as HTMLCanvasElement, calls, raw: canvas };
}

function setup(t: Terminal, dpr = 1) {
  const fake = fakeCanvas();
  const frames: (() => void)[] = [];
  const env = {
    devicePixelRatio: () => dpr,
    requestFrame: (cb: () => void) => frames.push(cb),
    cancelFrame: () => {},
  };
  const palette = new Palette(
    Array.from({ length: 16 }, () => "#111111"),
    "#ffffff",
    "#000000",
    "#00ff00",
  );
  const r = new CanvasRenderer(
    fake.canvas,
    t,
    { fontFamily: "mono", fontSize: 16, lineHeight: 1, palette },
    env,
  );
  // DOM を使う `open` の代わりに、描画を直接つなぐ
  (t as unknown as { renderer: CanvasRenderer }).renderer = r;
  const flush = () => {
    fake.calls.length = 0;
    const fs = frames.splice(0);
    for (const f of fs) f();
    return fake.calls.filter((c) => c[0] === "fillText").map((c) => c[1]);
  };
  return { r, fake, frames, flush };
}

test("renderer: measures the cell and sizes the canvas in device pixels", () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const { r, fake } = setup(t, 2);
  // 幅は 'W' x 32 を 32 で割った 9、高さは 12 + 4
  assert.deepEqual(r.cellSize, { width: 4.5, height: 8 });
  assert.equal(fake.raw.width, 90);
  assert.equal(fake.raw.height, 64);
  assert.equal(fake.raw.style.width, "45px");
  assert.equal(fake.raw.style.height, "32px");
  t.dispose();
});

test("renderer: draws once per frame, and only the changed rows afterwards", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const { frames, flush } = setup(t);
  assert.equal(frames.length, 1);
  assert.deepEqual(flush(), []); // 最初は全部を描く（空）
  await write(t, "ab");
  await write(t, "cd");
  assert.equal(frames.length, 1, "writes in one frame draw once");
  const drawn = flush();
  assert.deepEqual(drawn, ["a", "b", "c", "d"]);
  // 2 行目へ動くと、カーソルがいた 1 行目と 2 行目を描く
  await write(t, "\r\nxy");
  assert.deepEqual(flush(), ["a", "b", "c", "d", "x", "y"]);
  // 2 行目の続きだけを書くと、1 行目は描き直さない
  await write(t, "z");
  assert.deepEqual(flush(), ["x", "y", "z"]);
  t.dispose();
});

test("renderer: wide characters and clusters are drawn once, at the head cell", async () => {
  const t = new Terminal({ cols: 10, rows: 2 });
  const { flush } = setup(t);
  flush();
  await write(t, "あaé");
  const drawn = flush();
  assert.equal(drawn[0], "あ");
  assert.equal(drawn.length, 3);
  t.dispose();
});

test("renderer: bold and italic change the font; invisible text is not drawn", async () => {
  const t = new Terminal({ cols: 10, rows: 2 });
  const { fake, flush } = setup(t);
  flush();
  await write(t, "\x1b[1mB\x1b[0m\x1b[3mI\x1b[0m\x1b[8mH\x1b[0m");
  const drawn = flush();
  assert.deepEqual(drawn, ["B", "I"]);
  const fonts = fake.calls.filter((c) => c[0] === "set font").map((c) => c[1]);
  assert.ok(fonts.some((f) => String(f).startsWith("bold ")));
  assert.ok(fonts.some((f) => String(f).startsWith("italic ")));
  t.dispose();
});

test("renderer: reverse video swaps the colors; underline and strikethrough draw lines", async () => {
  const t = new Terminal({ cols: 10, rows: 2 });
  const { fake, flush } = setup(t);
  flush();
  await write(t, "\x1b[7mR\x1b[0m");
  flush();
  // 反転: 背景が前景色（白）、字が背景色（黒）
  const styles = fake.calls
    .filter((c) => c[0] === "set fillStyle")
    .map((c) => c[1]);
  assert.ok(styles.includes("#ffffff"));
  await write(t, "\x1b[4:3mU\x1b[0m\x1b[9mS\x1b[0m");
  fake.calls.length = 0;
  flush();
  assert.ok(fake.calls.some((c) => c[0] === "stroke"), "curly underline");
  assert.ok(fake.calls.some((c) => c[0] === "fillRect"), "strikethrough");
  t.dispose();
});

test("renderer: the cursor is drawn as a block, an outline without focus, a bar or underline", async () => {
  const t = new Terminal({ cols: 10, rows: 2 });
  const { fake, flush } = setup(t);
  flush();
  // フォーカスあり・ブロック: カーソルの色で塗る
  t.setFocused(true);
  flush();
  let filled = fake.calls.filter(
    (c) => c[0] === "set fillStyle" && c[1] === "#00ff00",
  );
  assert.ok(filled.length > 0, "block");
  // フォーカスなし: 中抜き
  t.setFocused(false);
  flush();
  assert.ok(fake.calls.some((c) => c[0] === "strokeRect"), "hollow block");
  // 縦棒
  t.setFocused(true);
  await write(t, "\x1b[6 q");
  flush();
  assert.ok(
    fake.calls.some((c) => c[0] === "fillRect" && c[3] === 1 && c[4] === 16),
    "bar",
  );
  // 下線
  await write(t, "\x1b[4 q");
  flush();
  assert.ok(
    fake.calls.some((c) => c[0] === "fillRect" && c[3] === 9 && c[4] === 1),
    "underline",
  );
  // 非表示
  await write(t, "\x1b[?25l");
  flush();
  filled = fake.calls.filter(
    (c) => c[0] === "set fillStyle" && c[1] === "#00ff00",
  );
  assert.equal(filled.length, 0);
  t.dispose();
});

test("renderer: scrolled back, the old lines are drawn and the cursor is hidden", async () => {
  const t = new Terminal({ cols: 5, rows: 2, scrollback: 100 });
  const { flush } = setup(t);
  await write(t, "A\r\nB\r\nC\r\nD");
  flush();
  t.scrollToTop();
  const drawn = flush();
  assert.deepEqual(drawn, ["A", "B"]);
  t.dispose();
});
