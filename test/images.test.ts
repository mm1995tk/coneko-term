import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { before, test } from "node:test";
import { deflateSync } from "node:zlib";
import {
  browserImageDecoder,
  CanvasRenderer,
  init,
  Palette,
  Terminal,
  type DrawableImage,
  type ImageDecoder,
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
  return { canvas: canvas as unknown as HTMLCanvasElement, calls };
}

interface Bitmap extends DrawableImage {
  kind: string;
  closed: boolean;
}

/** 読み解いた結果を記録し、`Bitmap` を返す `ImageDecoder`。`inflate` は本物。 */
function fakeDecoder(opts: { failPng?: boolean } = {}) {
  const log: { kind: string; bytes: number[]; width?: number; height?: number }[] = [];
  const bitmaps: Bitmap[] = [];
  const make = (kind: string, width: number, height: number): Bitmap => {
    const b = {
      kind,
      width,
      height,
      closed: false,
      close() {
        this.closed = true;
      },
    };
    bitmaps.push(b);
    return b;
  };
  const decoder: ImageDecoder = {
    inflate: (bytes) => browserImageDecoder.inflate(bytes),
    async fromPng(bytes) {
      log.push({ kind: "png", bytes: Array.from(bytes) });
      if (opts.failPng) throw new Error("bad png");
      return make("png", 3, 5);
    },
    async fromRgba(rgba, width, height) {
      log.push({ kind: "rgba", bytes: Array.from(rgba), width, height });
      return make("rgba", width, height);
    },
  };
  return { decoder, log, bitmaps };
}

/** 1 つの文字が 9x16 画素のセルで描く。 */
function setup(t: Terminal, decoder: ImageDecoder) {
  const fake = fakeCanvas();
  const frames: (() => void)[] = [];
  const env = {
    devicePixelRatio: () => 1,
    requestFrame: (cb: () => void) => frames.push(cb),
    cancelFrame: () => {},
    imageDecoder: decoder,
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
  (t as unknown as { renderer: CanvasRenderer }).renderer = r;
  (t as unknown as { syncCellSize(): void }).syncCellSize();
  const flush = () => {
    fake.calls.length = 0;
    for (const f of frames.splice(0)) f();
    return fake.calls;
  };
  /** 画像の読み解きが終わって描き直しが頼まれるまで待ち、描く。 */
  const settle = async () => {
    flush();
    for (let i = 0; i < 200 && frames.length === 0; i++) {
      await new Promise((r) => setTimeout(r, 5));
    }
    return flush();
  };
  return { r, fake, frames, flush, settle };
}

const draws = (calls: Call[]) => calls.filter((c) => c[0] === "drawImage");

const b64 = (bytes: Uint8Array | number[]) => Buffer.from(bytes).toString("base64");
const apc = (keys: string, data: Uint8Array | number[]) =>
  `\x1b_G${keys};${b64(data)}\x1b\\`;

/** IHDR だけの PNG（3 x 5 画素）。 */
const PNG = [
  0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44,
  0x52, 0, 0, 0, 3, 0, 0, 0, 5, 8, 6, 0, 0, 0,
];

test("images: f=24 is turned into RGBA and drawn over the cells it is placed on", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const { decoder, log } = fakeDecoder();
  const { settle } = setup(t, decoder);
  // 1x2 の RGB（赤、緑）を 2 列 x 2 行に引き伸ばす
  await write(t, apc("a=T,f=24,s=1,v=2,i=1,c=2,r=2", [255, 0, 0, 0, 255, 0]));
  const calls = await settle();
  assert.deepEqual(log, [
    { kind: "rgba", bytes: [255, 0, 0, 255, 0, 255, 0, 255], width: 1, height: 2 },
  ]);
  // セルは 9 x 16 画素。1 行ずつ、画像のその行の部分を描く
  assert.deepEqual(
    draws(calls).map((c) => c.slice(2)),
    [
      [0, 0, 1, 1, 0, 0, 18, 16],
      [0, 1, 1, 1, 0, 16, 18, 16],
    ],
  );
  t.dispose();
});

test("images: f=32 with no c and r is drawn at its own size, at the cursor, honoring x/y/w/h/X/Y", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const { decoder } = fakeDecoder();
  const { settle } = setup(t, decoder);
  await write(t, "\r\n  ");
  // 4x2 の RGBA の右下 2x1 を、セルの中で (3, 4) ずらして置く
  const px = Array.from({ length: 4 * 2 * 4 }, (_, i) => i);
  await write(t, apc("a=T,f=32,s=4,v=2,i=1,x=2,y=1,w=2,h=1,X=3,Y=4", px));
  const calls = await settle();
  assert.deepEqual(
    draws(calls).map((c) => c.slice(2)),
    [[2, 1, 2, 1, 2 * 9 + 3, 16 + 4, 2, 1]],
  );
  t.dispose();
});

test("images: f=100 and o=z are decoded in the browser way, and decoded once", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const { decoder, log } = fakeDecoder();
  const { settle, flush } = setup(t, decoder);
  const compressedPng = [...deflateSync(Buffer.from(PNG))];
  await write(t, apc("a=T,f=100,o=z,s=3,v=5,i=1,c=3,r=1", compressedPng));
  assert.equal(draws(await settle()).length, 1);
  assert.deepEqual(log, [{ kind: "png", bytes: PNG }]);
  // 描き直しても、読み解き直さない
  await write(t, "x");
  flush();
  await write(t, "y");
  assert.equal(draws(flush()).length, 1);
  assert.equal(log.length, 1);

  // 圧縮した RGB も戻す
  await write(t, "\x1b[2J\x1b[H");
  await write(t, apc("a=T,f=24,o=z,s=1,v=1,i=2", [...deflateSync(Buffer.from([1, 2, 3]))]));
  assert.equal(draws(await settle()).length, 1);
  assert.deepEqual(log[1], { kind: "rgba", bytes: [1, 2, 3, 255], width: 1, height: 1 });
  t.dispose();
});

test("images: a size that does not match the pixels, and undecodable data, are not drawn and do not throw", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const { decoder, log } = fakeDecoder({ failPng: true });
  const { settle, flush } = setup(t, decoder);
  // 圧縮されているので、コアは長さを確かめない。戻した長さが s x v x 3 と違う
  await write(t, apc("a=T,f=24,o=z,s=2,v=2,i=1", [...deflateSync(Buffer.from([1, 2, 3]))]));
  await write(t, apc("a=T,f=100,i=2", PNG));
  await write(t, apc("a=T,f=32,o=z,s=1,v=1,i=3", [1, 2, 3, 4])); // zlib ではないデータ
  flush();
  await new Promise((r) => setTimeout(r, 50));
  assert.equal(draws(flush()).length, 0);
  assert.equal(log.length, 1); // PNG だけが読み解きまで届いた
  t.dispose();
});

test("images: z < 0 is under the text, z >= 0 over it, z < -1073741824 under the cell backgrounds", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const { decoder } = fakeDecoder();
  const { settle } = setup(t, decoder);
  // 背景が赤（パレットで #111111）の「X」の上に、4 枚の 1x1 を重ねる
  await write(t, "\x1b[41mX\x1b[0m");
  const px = [9, 9, 9, 255];
  for (const [id, z] of [
    [1, 1],
    [2, -1],
    [3, -1073741825],
    [4, 0],
  ]) {
    await write(t, `\x1b[1;1H${apc(`a=T,C=1,f=32,s=1,v=1,i=${id},c=1,r=1,z=${z}`, px)}`);
  }
  const calls = await settle();
  let fill = "";
  const order: string[] = [];
  for (const c of calls) {
    if (c[0] === "set fillStyle") fill = String(c[1]);
    if (c[0] === "fillRect" && c[2] === 0 && fill === "#111111") order.push("bg");
    if (c[0] === "fillText") order.push("text");
    if (c[0] === "drawImage") order.push("img");
  }
  // 行 0 を描いた部分だけを見る（最初の背景のあと）
  const row0 = order.slice(0, order.lastIndexOf("img") + 1);
  assert.deepEqual(row0, ["img", "bg", "img", "text", "img", "img"]);
  t.dispose();
});

test("images: deleting the placement stops drawing it; deleting the image too frees the decoded image", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const { decoder, bitmaps } = fakeDecoder();
  const { settle, flush } = setup(t, decoder);
  await write(t, apc("a=T,f=32,s=1,v=1,i=1,C=1", [1, 2, 3, 4]));
  assert.equal(draws(await settle()).length, 1);
  await write(t, "\x1b_Ga=d,d=a\x1b\\");
  assert.equal(draws(flush()).length, 0);
  // 配置だけ消した（d=a）ので、画像のデータも読み解いた画像も残る
  assert.equal(bitmaps[0].closed, false);
  // 置き直すと、読み解き直さずにすぐ描く
  await write(t, "\x1b_Ga=p,i=1,C=1\x1b\\");
  assert.equal(draws(flush()).length, 1);
  assert.equal(bitmaps.length, 1);
  // 画像ごと消す（d=A）と、読み解いた画像も捨てる
  await write(t, "\x1b_Ga=d,d=A\x1b\\");
  assert.equal(draws(flush()).length, 0);
  assert.equal(bitmaps[0].closed, true);
  t.dispose();
});

test("images: sending the same id again draws the new image", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const { decoder, log, bitmaps } = fakeDecoder();
  const { settle } = setup(t, decoder);
  await write(t, apc("a=T,f=32,s=1,v=1,i=1,C=1", [1, 2, 3, 4]));
  await settle();
  await write(t, apc("a=T,f=32,s=1,v=1,i=1,C=1", [5, 6, 7, 8]));
  assert.equal(draws(await settle()).length, 1);
  assert.deepEqual(
    log.map((l) => l.bytes),
    [
      [1, 2, 3, 4],
      [5, 6, 7, 8],
    ],
  );
  assert.equal(bitmaps[0].closed, true);
  t.dispose();
});

test("images: scrolling moves the image with the text, and an image scrolled out of view is not drawn", async () => {
  const t = new Terminal({ cols: 10, rows: 3, scrollback: 10 });
  const { decoder } = fakeDecoder();
  const { settle, flush } = setup(t, decoder);
  // 1x2 の画像を 1 列 x 2 行で、行 0 に置く
  await write(t, apc("a=T,f=32,s=1,v=2,i=1,C=1,c=1,r=2", [1, 2, 3, 4, 5, 6, 7, 8]));
  const first = draws(await settle());
  assert.deepEqual(
    first.map((c) => [c[3], c[7]]),
    [
      [0, 0],
      [1, 16],
    ],
  );
  // 一番下の行で改行して 1 行スクロールすると、画像は 1 行上がり、下半分だけが見える
  await write(t, "\r\n\r\n\r\n");
  assert.deepEqual(
    draws(flush()).map((c) => [c[3], c[7]]),
    [[1, 0]],
  );
  await write(t, "\r\n");
  assert.equal(draws(flush()).length, 0, "scrolled into the scrollback");
  t.scrollToTop();
  assert.equal(draws(flush()).length, 2, "visible again when looking back");
  t.dispose();
});

test("images: the core is told the cell size in pixels", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const { decoder } = fakeDecoder();
  setup(t, decoder);
  // 9 x 16 画素のセル。18 x 32 画素の画像は 2 列 2 行
  const px = Array.from({ length: 18 * 32 * 4 }, () => 0);
  await write(t, apc("a=T,f=32,s=18,v=32,i=1,C=1", px));
  const flat = t.readPlacements();
  assert.equal(flat[10], 2);
  assert.equal(flat[11], 2);
  t.dispose();
});

test("images: the browser inflate reads zlib data", async () => {
  const out = await browserImageDecoder.inflate(deflateSync(Buffer.from("hello")));
  assert.equal(Buffer.from(out).toString(), "hello");
});

test("images: Unicode placeholders draw the part of the image under each row, and not the placeholder glyph", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const { decoder } = fakeDecoder();
  const { settle } = setup(t, decoder);
  // 1x2 の RGB を、2 列 x 2 行の仮の配置にする（セルは 9 x 16 画素）
  await write(t, apc("a=t,f=24,s=1,v=2,i=7,q=2", [255, 0, 0, 0, 255, 0]));
  await write(t, "\x1b_Ga=p,U=1,i=7,c=2,r=2,q=2\x1b\\");
  const P = "\u{10EEEE}";
  await write(
    t,
    `\x1b[38;5;7m${P}̅̅${P}̅̍\r\n${P}̍̅${P}̍̍`,
  );
  const calls = await settle();
  // 枠は 18 x 32 画素、画像は 1x2 なので 16 x 32 に収まり、左右に 1 画素ずつの余白
  assert.deepEqual(
    draws(calls).map((c) => c.slice(2)),
    [
      [0, 0, 1, 1, 1, 0, 16, 16],
      [0, 1, 1, 1, 1, 16, 16, 16],
    ],
  );
  assert.deepEqual(
    calls.filter((c) => c[0] === "fillText"),
    [],
  );
  t.dispose();
});
