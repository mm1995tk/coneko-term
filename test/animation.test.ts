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

const b64 = (bytes: Uint8Array | number[]) => Buffer.from(bytes).toString("base64");
const apc = (keys: string, data: Uint8Array | number[] = []) =>
  `\x1b_G${keys}${data.length > 0 ? ";" + b64(data) : ""}\x1b\\`;

interface Bitmap extends DrawableImage {
  rgba: number[];
}

/** 読み解いた画素を持つ `Bitmap` を返す `ImageDecoder`。`inflate` は本物。PNG は 1x1 の赤。 */
function fakeDecoder(): ImageDecoder {
  return {
    inflate: (bytes) => browserImageDecoder.inflate(bytes),
    async fromPng() {
      throw new Error("no png here");
    },
    async fromRgba(rgba, width, height): Promise<Bitmap> {
      return { rgba: Array.from(rgba), width, height };
    },
    async toRgba() {
      return { rgba: new Uint8ClampedArray([255, 0, 0, 255]), width: 1, height: 1 };
    },
  };
}

/** 画像を 1 つ置いて、時刻を自分で進めながら描く。 */
function setup(t: Terminal) {
  const drawn: { image?: Bitmap } = {};
  const calls: unknown[][] = [];
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
        if (prop === "drawImage") {
          return (image: Bitmap) => {
            drawn.image = image;
          };
        }
        return () => calls.push([prop]);
      },
      set: () => true,
    },
  );
  const canvas = { width: 0, height: 0, style: {}, getContext: () => ctx };
  const frames: (() => void)[] = [];
  const clock = { now: 0 };
  const env = {
    devicePixelRatio: () => 1,
    requestFrame: (cb: () => void) => frames.push(cb),
    cancelFrame: () => {},
    imageDecoder: fakeDecoder(),
    now: () => clock.now,
  };
  const palette = new Palette(
    Array.from({ length: 16 }, () => "#111111"),
    "#ffffff",
    "#000000",
    "#00ff00",
  );
  const r = new CanvasRenderer(
    canvas as unknown as HTMLCanvasElement,
    t,
    { fontFamily: "mono", fontSize: 16, lineHeight: 1, palette },
    env,
  );
  (t as unknown as { renderer: CanvasRenderer }).renderer = r;
  (t as unknown as { syncCellSize(): void }).syncCellSize();
  const flush = () => {
    for (const f of frames.splice(0)) f();
  };
  /** いま描かれている画像の画素が `expected` になるまで、描画を回す。 */
  const showing = async (expected: number[]) => {
    for (let i = 0; i < 200; i++) {
      flush();
      if (JSON.stringify(drawn.image?.rgba) === JSON.stringify(expected)) return;
      await new Promise((resolve) => setTimeout(resolve, 2));
    }
    assert.deepEqual(drawn.image?.rgba, expected);
  };
  /** 時刻を `ms` 進めて、描画を回す（画像の読み解きの終わりも待つ）。 */
  const advance = async (ms: number) => {
    clock.now += ms;
    flush();
    await new Promise((resolve) => setTimeout(resolve, 10));
    flush();
  };
  return { r, frames, clock, flush, showing, advance };
}

const RED = [255, 0, 0, 255];
const GREEN = [0, 255, 0, 255];
const CLEAR = [0, 0, 0, 0];
const rep = (px: number[], n: number) => Array.from({ length: n }, () => px).flat();

/** 2x2 の赤い画像 `i=1` を、セルに置く。 */
async function sendRoot(t: Terminal): Promise<void> {
  await write(t, apc("a=T,f=32,s=2,v=2,i=1,c=2,r=1", rep(RED, 4)));
}

test("animation: frames are composed from the base frame and shown for their own gap, then it loops", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const { showing, advance } = setup(t);
  await sendRoot(t);
  // コマ 2: 1 コマ目の上の (1, 1) だけ緑。コマ 3: 下地は透明な黒（c を省く）で、左上だけ赤。
  await write(t, apc("a=f,f=32,s=1,v=1,i=1,c=1,x=1,y=1,z=100", GREEN));
  await write(t, apc("a=f,f=32,s=1,v=1,i=1,z=200", RED));
  // 根の表示時間は a=a で決める。回し始める。
  await write(t, apc("a=a,i=1,r=1,z=50,s=3"));
  const frame1 = rep(RED, 4);
  const frame2 = [...RED, ...RED, ...RED, ...GREEN];
  const frame3 = [...RED, ...rep(CLEAR, 3)];
  await showing(frame1);
  await advance(49);
  await showing(frame1);
  await advance(1);
  await showing(frame2);
  await advance(99);
  await showing(frame2);
  await advance(1);
  await showing(frame3);
  await advance(200);
  // 最後のコマの次は、最初に戻る
  await showing(frame1);
  t.dispose();
});

test("animation: a=a stops, sets the current frame, and counts loops", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const { showing, advance, flush, frames } = setup(t);
  await sendRoot(t);
  await write(t, apc("a=f,f=32,s=2,v=2,i=1,z=10", rep(GREEN, 4)));
  const red = rep(RED, 4);
  const green = rep(GREEN, 4);
  await write(t, apc("a=a,i=1,r=1,z=10"));
  await showing(red);
  // 止まっている間は、時間が過ぎても変わらず、次のフレームも頼まない
  await advance(1000);
  await showing(red);
  flush();
  assert.equal(frames.length, 0);
  // c= で今のコマを変える（止まったまま）
  await write(t, apc("a=a,i=1,c=2"));
  await showing(green);
  await advance(1000);
  await showing(green);
  // v=2 は 2 - 1 = 1 回。2 コマを 1 周すると止まる
  await write(t, apc("a=a,i=1,c=1,s=3,v=2"));
  await showing(red);
  await advance(10);
  await showing(green);
  await advance(10);
  await advance(10);
  await showing(green);
  assert.equal(t.animationInfo(1)[2], 1);
  // v=1 は回し続ける
  await write(t, apc("a=a,i=1,c=1,s=3,v=1"));
  await showing(red);
  await advance(10);
  await showing(green);
  await advance(10);
  await showing(red);
  await advance(10);
  await showing(green);
  await write(t, apc("a=a,i=1,s=1"));
  await advance(500);
  await showing(green);
  t.dispose();
});

test("animation: a gapless frame is skipped, and z=0 keeps the earlier gap", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const { showing, advance } = setup(t);
  await sendRoot(t);
  await write(t, apc("a=f,f=32,s=2,v=2,i=1,z=-1", rep(GREEN, 4)));
  await write(t, apc("a=f,f=32,s=2,v=2,i=1,z=20", rep(CLEAR, 4)));
  await write(t, apc("a=a,i=1,r=1,z=10,s=3"));
  // z=0 は、指定なしと同じ（20 のまま）
  await write(t, apc("a=a,i=1,r=3,z=0"));
  await showing(rep(RED, 4));
  await advance(10);
  // コマ 2 は見せずに、コマ 3 へ
  await showing(rep(CLEAR, 4));
  await advance(19);
  await showing(rep(CLEAR, 4));
  await advance(1);
  await showing(rep(RED, 4));
  t.dispose();
});

test("animation: r edits a frame, X=1 overwrites instead of blending, Y fills the background", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const { showing, advance } = setup(t);
  await sendRoot(t);
  const half = [0, 0, 255, 128];
  // コマ 2: 背景は 0xff000080（半透明の赤）。そこへ左上に半透明の青をアルファ合成
  await write(t, apc("a=f,f=32,s=1,v=1,i=1,Y=4278190208,z=10", half));
  await write(t, apc("a=a,i=1,r=1,z=10,s=3"));
  await showing(rep(RED, 4));
  await advance(10);
  // 背景の赤（128 / 255）の上に青（128 / 255）を重ねる: 出力のアルファ 128 + 128 * 127 / 255 = 191.7
  const bg = [255, 0, 0, 128];
  const blended = [85, 0, 170, 192];
  await showing([...blended, ...bg, ...bg, ...bg]);
  // r=2 で同じコマを編集する（下地はそのコマ自身）。X=1 は置き換え
  await write(t, apc("a=f,f=32,s=1,v=1,i=1,r=2,x=1,y=1,X=1", half));
  await advance(0);
  await showing([...blended, ...bg, ...bg, ...half]);
  t.dispose();
});

test("animation: a=c composes a rectangle of one frame onto another", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const { showing, advance } = setup(t);
  await sendRoot(t);
  await write(t, apc("a=f,f=32,s=2,v=2,i=1,z=10", rep(GREEN, 4)));
  await write(t, apc("a=a,i=1,r=1,z=10,s=3"));
  // 仕様の形: コマ 2 の (1, 0) の 1x2 の四角を、コマ 1 の (0, 0) へ（置き換え）
  await write(t, apc("a=c,i=1,r=2,c=1,w=1,h=2,X=1,Y=0,x=0,y=0,C=1"));
  await showing([...GREEN, ...RED, ...GREEN, ...RED]);
  await advance(10);
  await showing(rep(GREEN, 4));
  t.dispose();
});

test("animation: zlib frame data is inflated, and a frame that cannot be read leaves the base as it is", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const { showing, advance } = setup(t);
  await sendRoot(t);
  const z = deflateSync(Buffer.from(rep(GREEN, 4)));
  await write(t, apc("a=f,f=32,s=2,v=2,i=1,o=z,c=1,z=10", z));
  // 長さが合わない圧縮データ: 読めないので、コマは下地のまま
  await write(
    t,
    apc("a=f,f=32,s=2,v=2,i=1,o=z,c=1,z=10", deflateSync(Buffer.from([1, 2, 3]))),
  );
  await write(t, apc("a=a,i=1,r=1,z=10,s=3"));
  await showing(rep(RED, 4));
  await advance(10);
  await showing(rep(GREEN, 4));
  await advance(10);
  await showing(rep(RED, 4));
  t.dispose();
});

test("animation: PNG frames are read with the decoder's toRgba", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const { showing, advance } = setup(t);
  await write(t, apc("a=T,f=32,s=1,v=1,i=1,c=1,r=1", GREEN));
  // 1x1 の PNG（読み解きは fakeDecoder が赤を返す）。ヘッダーだけ本物に見えればよい
  const png = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44,
    0x52, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0,
  ];
  await write(t, apc("a=f,f=100,i=1,z=10", png));
  await write(t, apc("a=a,i=1,r=1,z=10,s=3"));
  await showing(GREEN);
  await advance(10);
  await showing(RED);
  t.dispose();
});

test("animation: a=c within one frame composes rectangles that do not overlap", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  const { showing, advance } = setup(t);
  await sendRoot(t);
  await write(t, apc("a=f,f=32,s=2,v=2,i=1,z=10", [...GREEN, ...RED, ...RED, ...RED]));
  await write(t, apc("a=a,i=1,r=1,z=10,s=3"));
  await showing(rep(RED, 4));
  await advance(10);
  await showing([...GREEN, ...RED, ...RED, ...RED]);
  // コマ 2 の左の列を、同じコマの右の列へ
  await write(t, apc("a=c,i=1,r=2,c=2,w=1,h=2,X=0,Y=0,x=1,y=0,C=1"));
  await showing([...GREEN, ...GREEN, ...RED, ...RED]);
  t.dispose();
});
