import { clone, compose, fromRaw, solid, type Pixels } from "./animation.js";

/** `WasmTerminal.placements` の 1 つの配置が使う数。並びは `ImagePlacement` の順。 */
export const PLACEMENT_STRIDE = 16;

/** `z` がこれより小さい画像は、背景色の下に描く（kitty の仕様）。 */
export const Z_UNDER_BACKGROUND = -1_073_741_824;

/** 画面に置いてある画像の配置 1 つ。 */
export interface ImagePlacement {
  /** 左上のセルの行。見えている画面の一番上を 0 とする。スクロールバックの中は負。 */
  row: number;
  /** 画像の ID。 */
  imageId: number;
  /** 配置の番号（0 は無い）。 */
  placementId: number;
  /** 左上のセルの列。 */
  col: number;
  /** 元の画像の使う部分の左端・上端・幅・高さ（画素）。幅と高さが 0 なら端まで。 */
  srcX: number;
  srcY: number;
  srcW: number;
  srcH: number;
  /** 左上のセルの中でのずれ（画素）。 */
  offsetX: number;
  offsetY: number;
  /** 使うセルの列数・行数。 */
  cols: number;
  rows: number;
  /** 重ねる順。負なら字の下、0 以上なら字の上。 */
  z: number;
  /** 画素のまま置くか（そうでなければ `cols` x `rows` のセルいっぱいに引き伸ばす）。 */
  native: boolean;
  /** 画素で決めた描く大きさ（幅・高さ）。0 なら `native` と `cols` x `rows` で決める。 */
  destW: number;
  destH: number;
}

/** `WasmTerminal.placements` の並びを読む。 */
export function parsePlacements(flat: ArrayLike<number>): ImagePlacement[] {
  const out: ImagePlacement[] = [];
  for (let o = 0; o + PLACEMENT_STRIDE <= flat.length; o += PLACEMENT_STRIDE) {
    out.push({
      row: flat[o],
      imageId: flat[o + 1],
      placementId: flat[o + 2],
      col: flat[o + 3],
      srcX: flat[o + 4],
      srcY: flat[o + 5],
      srcW: flat[o + 6],
      srcH: flat[o + 7],
      offsetX: flat[o + 8],
      offsetY: flat[o + 9],
      cols: flat[o + 10],
      rows: flat[o + 11],
      z: flat[o + 12],
      native: flat[o + 13] !== 0,
      destW: flat[o + 14],
      destH: flat[o + 15],
    });
  }
  return out;
}

/** 描ける画像（`ImageBitmap` など）。 */
export interface DrawableImage {
  readonly width: number;
  readonly height: number;
  close?(): void;
}

/** 画像を読み解く道具。既定はブラウザーの機能で、テストでは入れ替える。 */
export interface ImageDecoder {
  /** zlib（`o=z`）を戻す。 */
  inflate(bytes: Uint8Array): Promise<Uint8Array>;
  /** PNG を読み解く。 */
  fromPng(bytes: Uint8Array): Promise<DrawableImage>;
  /** RGBA（1 画素 4 バイト）の画素から作る。 */
  fromRgba(rgba: Uint8ClampedArray, width: number, height: number): Promise<DrawableImage>;
  /** PNG を RGBA の画素に読み解く。アニメーションのコマの合成に使う。無ければ、PNG の画像はアニメーションにせず、根だけを描く。 */
  toRgba?(bytes: Uint8Array): Promise<Pixels>;
}

/** `DecompressionStream`、`createImageBitmap`、`ImageData` を使う、既定の `ImageDecoder`。 */
export const browserImageDecoder: ImageDecoder = {
  async inflate(bytes) {
    const stream = new Blob([bytes as BlobPart])
      .stream()
      .pipeThrough(new DecompressionStream("deflate"));
    return new Uint8Array(await new Response(stream).arrayBuffer());
  },
  fromPng(bytes) {
    return createImageBitmap(new Blob([bytes as BlobPart], { type: "image/png" }));
  },
  fromRgba(rgba, width, height) {
    return createImageBitmap(new ImageData(rgba as ImageDataArray, width, height));
  },
  async toRgba(bytes) {
    const bitmap = await createImageBitmap(new Blob([bytes as BlobPart], { type: "image/png" }));
    try {
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const ctx = canvas.getContext("2d");
      if (!ctx) throw new Error("coneko-term: canvas 2d context is unavailable");
      ctx.drawImage(bitmap, 0, 0);
      const data = ctx.getImageData(0, 0, bitmap.width, bitmap.height);
      return { rgba: data.data, width: bitmap.width, height: bitmap.height };
    } finally {
      bitmap.close();
    }
  },
};

/** `ImageCache` が `Terminal` から読むもの。 */
export interface ImageSource {
  /** 持っている画像の `[ID, 通し番号]` の並び。 */
  imageSerials(): ArrayLike<number>;
  /** 画像の `[通し番号, 形式（0 RGB、1 RGBA、2 PNG）, 圧縮, 幅, 高さ]`。無ければ空。 */
  imageInfo(id: number): ArrayLike<number>;
  /** 画像のバイト列（圧縮も PNG もそのまま）。 */
  imageData(id: number): Uint8Array;
  /** アニメーションの状態 `[コマの数, いまのコマ, 再生の状態（1 止まる、2・3 回す）, いまのコマの表示時間（ミリ秒）, 操作の数]`。無ければ空。 */
  animationInfo?(id: number): ArrayLike<number>;
  /** いまのコマの表示時間が過ぎたと知らせる。次のコマへ進んだら `true`。 */
  animationStep?(id: number): boolean;
  /** コマを作る `index` 番目の操作（`WasmTerminal.frame_op` の並び）。 */
  frameOp?(id: number, index: number): ArrayLike<number>;
  /** 操作のバイト列。 */
  frameOpData?(id: number, index: number): Uint8Array;
}

/** アニメーションする画像の、コマごとの画素と、見せるコマの絵。 */
interface AnimState {
  /** コマごとの画素（0 番目が根）。操作を順に当てて作る。 */
  frames: Pixels[];
  /** 当て終えた操作の数。 */
  applied: number;
  /** 操作を当てている最中か（非同期なので、重ならないようにする）。 */
  busy: boolean;
  /** 根の画像が読み解けなかった。もう試さない。 */
  dead: boolean;
  /** コマごとの絵（描ける形）。作り終えたもの。 */
  bitmaps: Map<number, DrawableImage>;
  /** 絵を作っている最中のコマ。 */
  pending: Set<number>;
  /** いま見せている絵。次のコマの絵ができるまでは、これを見せ続ける。 */
  shown?: DrawableImage;
  /** 見せる絵が変わるたびに増える。描き直すかの判断に使う。 */
  version: number;
}

interface Entry {
  serial: number;
  image?: DrawableImage;
  /** 根を通常の画像として読み解き始めたか。 */
  started?: boolean;
  anim?: AnimState;
}

/** 絵を覚えておくコマの数。これを超えたら古いものから閉じる。 */
const BITMAP_LIMIT = 8;

/** コマの表示時間を数え始めた時刻。 */
interface Timer {
  current: number;
  since: number;
}

/**
 * 読み解いた画像を ID ごとに覚えておく。画像が消えたり、同じ ID で別の画像に変わったら捨てる
 * （`sync`）。読み解けない画像は描かない（エラーにしない）。読み解きは非同期なので、
 * 終わったら `onReady` を呼ぶ。
 *
 * コマを足した画像（アニメーション）は、コマを作る操作を順に当てて画素を作り、いまのコマの
 * 絵を返す（`get`）。コマを進める時刻は `tick` で決める。時計はこちらが持つ。
 */
export class ImageCache {
  private entries = new Map<number, Entry>();
  private timers = new Map<number, Timer>();

  constructor(
    private source: ImageSource,
    private decoder: ImageDecoder,
    private onReady: () => void,
  ) {}

  /** 持っている画像に合わせて、いらなくなったものを捨てる。 */
  sync(): void {
    if (this.entries.size === 0) return;
    const alive = new Map<number, number>();
    const flat = this.source.imageSerials();
    for (let i = 0; i + 1 < flat.length; i += 2) alive.set(flat[i], flat[i + 1]);
    for (const [id, entry] of this.entries) {
      if (alive.get(id) !== entry.serial) this.drop(id, entry);
    }
    for (const id of this.timers.keys()) {
      if (!alive.has(id)) this.timers.delete(id);
    }
  }

  /** 画像 `id` の、コマを足してあれば、その状態。 */
  private animation(id: number): ArrayLike<number> | undefined {
    const info = this.source.animationInfo?.(id);
    return info && info.length >= 5 && info[4] > 0 ? info : undefined;
  }

  /**
   * 時刻 `now`（ミリ秒）で、画像 `id` のコマを進める。表示時間が過ぎていたら次のコマへ。
   * 回し続けている（また呼ぶ必要がある）なら `true`。
   */
  tick(id: number, now: number): boolean {
    const info = this.animation(id);
    if (!info) return false;
    const [frames, current, state, gap] = Array.from(info);
    if (frames < 2) return false;
    const timer = this.timers.get(id);
    if (!timer || timer.current !== current || state === 1) {
      this.timers.set(id, { current, since: now });
      return state !== 1;
    }
    if (now - timer.since >= Math.max(gap, 0)) {
      if (this.source.animationStep?.(id)) {
        const after = this.source.animationInfo?.(id);
        this.timers.set(id, { current: after?.[1] ?? current, since: now });
      } else {
        timer.since = now;
      }
    }
    return true;
  }

  /** 画像 `id` の絵が変わるたびに増える数（アニメーションでなければ 0）。 */
  version(id: number): number {
    return this.entries.get(id)?.anim?.version ?? 0;
  }

  /** 読み解き終えた画像。まだ、または読み解けなかったら `undefined`（初めて頼まれたら読み解きを始める）。 */
  get(id: number): DrawableImage | undefined {
    let entry = this.entries.get(id);
    const anim = entry?.anim?.dead ? undefined : this.animation(id);
    if (anim) {
      if (!entry) {
        const info = this.source.imageInfo(id);
        if (info.length < 5) return undefined;
        entry = { serial: info[0] };
        this.entries.set(id, entry);
      }
      return this.getAnimated(id, entry, anim);
    }
    if (entry) {
      // コマを使えない画像は、根だけを通常の画像として描く
      if (entry.anim?.dead && !entry.started) this.startDecode(id, entry);
      return entry.image;
    }
    const info = this.source.imageInfo(id);
    if (info.length < 5) return undefined;
    const fresh: Entry = { serial: info[0] };
    this.entries.set(id, fresh);
    this.startDecode(id, fresh);
    return undefined;
  }

  /** 画像そのもの（根）を読み解き始める。 */
  private startDecode(id: number, fresh: Entry): void {
    const info = this.source.imageInfo(id);
    if (info.length < 5) return;
    fresh.started = true;
    const data = this.source.imageData(id);
    void this.decode(info, data).then(
      (image) => {
        if (image && this.entries.get(id) === fresh) {
          fresh.image = image;
          this.onReady();
        } else {
          image?.close?.();
        }
      },
      () => {}, // 読み解けない画像は描かない
    );
  }

  private drop(id: number, entry: Entry): void {
    this.entries.delete(id);
    entry.image?.close?.();
    if (entry.anim) {
      const all = new Set([...entry.anim.bitmaps.values(), entry.anim.shown]);
      for (const b of all) b?.close?.();
      entry.anim.bitmaps.clear();
      entry.anim.shown = undefined;
    }
  }

  private async decode(
    info: ArrayLike<number>,
    data: Uint8Array,
  ): Promise<DrawableImage | undefined> {
    const [, format, compressed, width, height] = Array.from(info);
    let bytes = data;
    if (compressed) bytes = await this.decoder.inflate(bytes);
    if (format === 2) return this.decoder.fromPng(bytes);
    const pixels = fromRaw(bytes, format === 0 ? 3 : 4, width, height);
    if (!pixels) return undefined;
    return this.decoder.fromRgba(pixels.rgba, width, height);
  }

  /** 形式 `format`（0 RGB、1 RGBA、2 PNG）のデータを、画素の並びに読み解く。 */
  private async toPixels(
    format: number,
    compressed: boolean,
    data: Uint8Array,
    width: number,
    height: number,
  ): Promise<Pixels | undefined> {
    const bytes = compressed ? await this.decoder.inflate(data) : data;
    if (format === 2) return this.decoder.toRgba?.(bytes);
    return fromRaw(bytes, format === 0 ? 3 : 4, width, height);
  }

  private getAnimated(
    id: number,
    entry: Entry,
    info: ArrayLike<number>,
  ): DrawableImage | undefined {
    const a: AnimState = (entry.anim ??= {
      frames: [],
      applied: 0,
      busy: false,
      dead: false,
      bitmaps: new Map(),
      pending: new Set(),
      version: 0,
    });
    if (a.applied < info[4] && !a.busy && !a.dead) void this.pump(id, entry, a);
    const frame = info[1];
    const bitmap = a.bitmaps.get(frame);
    if (bitmap) {
      if (a.shown !== bitmap) {
        a.shown = bitmap;
        a.version++;
      }
      return bitmap;
    }
    const pixels = a.frames[frame - 1];
    if (pixels && !a.pending.has(frame)) {
      a.pending.add(frame);
      void this.decoder.fromRgba(pixels.rgba.slice(), pixels.width, pixels.height).then(
        (image) => {
          a.pending.delete(frame);
          if (this.entries.get(id) !== entry) {
            image.close?.();
            return;
          }
          // 操作でこのコマが変わっていたら、古い絵なので使わない
          if (a.frames[frame - 1] !== pixels) {
            image.close?.();
          } else {
            a.bitmaps.set(frame, image);
            this.trim(a);
          }
          this.onReady();
        },
        () => a.pending.delete(frame), // 作れない絵は描かない
      );
    }
    return a.shown ?? entry.image;
  }

  /** 覚えている絵が多すぎたら、見せていない古いものから閉じる。 */
  private trim(a: AnimState): void {
    for (const [frame, image] of a.bitmaps) {
      if (a.bitmaps.size <= BITMAP_LIMIT) break;
      if (image === a.shown) continue;
      a.bitmaps.delete(frame);
      image.close?.();
    }
  }

  /** 届いている操作を、まだ当てていないぶん順に当てる。 */
  private async pump(id: number, entry: Entry, a: AnimState): Promise<void> {
    a.busy = true;
    const alive = () => this.entries.get(id) === entry;
    try {
      if (a.frames.length === 0) {
        const info = this.source.imageInfo(id);
        if (info.length < 5) return;
        const [, format, compressed, width, height] = Array.from(info);
        const root = await this.toPixels(
          format,
          compressed !== 0,
          this.source.imageData(id),
          width,
          height,
        );
        if (!alive()) return;
        if (!root) {
          a.dead = true;
          return;
        }
        a.frames.push(root);
      }
      for (;;) {
        const total = this.animation(id)?.[4] ?? 0;
        if (a.applied >= total || !alive()) break;
        const op = this.source.frameOp?.(id, a.applied);
        if (!op || op.length === 0) break;
        const touched = await this.apply(id, a, op, a.applied);
        if (!alive()) return;
        a.applied++;
        // 変わったコマの絵は作り直す
        const old = a.bitmaps.get(touched);
        a.bitmaps.delete(touched);
        if (old && old !== a.shown) old.close?.();
      }
    } catch {
      // 根が読めなければ、コマは使わない。操作が当てられないだけなら、飛ばして次へ
      if (a.frames.length === 0) a.dead = true;
    } finally {
      a.busy = false;
    }
    if (alive()) this.onReady();
  }

  /** 操作 1 つを当てる。変わったコマの番号（1 始まり）を返す。 */
  private async apply(
    id: number,
    a: AnimState,
    op: ArrayLike<number>,
    index: number,
  ): Promise<number> {
    const root = a.frames[0];
    if (op[0] === 1) {
      const [, srcFrame, dstFrame, sx, sy, dx, dy, w, h, overwrite] = Array.from(op);
      const src = a.frames[srcFrame - 1];
      const dst = a.frames[dstFrame - 1];
      if (src && dst) {
        // 絵を作っている途中の写しと取り違えないよう、新しい画素に差し替える
        const next = clone(dst);
        compose(next, src, sx, sy, dx, dy, w, h, overwrite !== 0);
        a.frames[dstFrame - 1] = next;
      }
      return dstFrame;
    }
    const [, target, baseKind, baseValue, x, y, width, height, overwrite, format, compressed] =
      Array.from(op);
    const base =
      baseKind === 0
        ? a.frames[target - 1]
        : baseKind === 1
          ? a.frames[baseValue - 1]
          : undefined;
    // 差し替えるので、元の画素は書き換えない
    const canvas = base
      ? clone(base)
      : solid(root.width, root.height, baseKind === 2 ? baseValue : 0);
    // 読み解けないデータでも、コマは作る（下地のまま）
    const data = this.source.frameOpData?.(id, index) ?? new Uint8Array();
    try {
      const pixels = await this.toPixels(format, compressed !== 0, data, width, height);
      if (pixels) compose(canvas, pixels, 0, 0, x, y, pixels.width, pixels.height, overwrite !== 0);
    } catch {
      // 下地のまま
    }
    if (target > a.frames.length) a.frames.push(canvas);
    else a.frames[target - 1] = canvas;
    return target;
  }

  /** 全部捨てる。 */
  dispose(): void {
    for (const [id, entry] of [...this.entries]) this.drop(id, entry);
    this.timers.clear();
  }
}
