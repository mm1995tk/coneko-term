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
};

/** `ImageCache` が `Terminal` から読むもの。 */
export interface ImageSource {
  /** 持っている画像の `[ID, 通し番号]` の並び。 */
  imageSerials(): ArrayLike<number>;
  /** 画像の `[通し番号, 形式（0 RGB、1 RGBA、2 PNG）, 圧縮, 幅, 高さ]`。無ければ空。 */
  imageInfo(id: number): ArrayLike<number>;
  /** 画像のバイト列（圧縮も PNG もそのまま）。 */
  imageData(id: number): Uint8Array;
}

interface Entry {
  serial: number;
  image?: DrawableImage;
}

/**
 * 読み解いた画像を ID ごとに覚えておく。画像が消えたり、同じ ID で別の画像に変わったら捨てる
 * （`sync`）。読み解けない画像は描かない（エラーにしない）。読み解きは非同期なので、
 * 終わったら `onReady` を呼ぶ。
 */
export class ImageCache {
  private entries = new Map<number, Entry>();

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
  }

  /** 読み解き終えた画像。まだ、または読み解けなかったら `undefined`（初めて頼まれたら読み解きを始める）。 */
  get(id: number): DrawableImage | undefined {
    const entry = this.entries.get(id);
    if (entry) return entry.image;
    const info = this.source.imageInfo(id);
    if (info.length < 5) return undefined;
    const fresh: Entry = { serial: info[0] };
    this.entries.set(id, fresh);
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
    return undefined;
  }

  private drop(id: number, entry: Entry): void {
    this.entries.delete(id);
    entry.image?.close?.();
  }

  private async decode(
    info: ArrayLike<number>,
    data: Uint8Array,
  ): Promise<DrawableImage | undefined> {
    const [, format, compressed, width, height] = Array.from(info);
    let bytes = data;
    if (compressed) bytes = await this.decoder.inflate(bytes);
    if (format === 2) return this.decoder.fromPng(bytes);
    const bpp = format === 0 ? 3 : 4;
    if (!(width > 0) || !(height > 0) || bytes.length !== width * height * bpp) {
      return undefined;
    }
    let rgba: Uint8ClampedArray;
    if (bpp === 4) {
      rgba = new Uint8ClampedArray(bytes.buffer, bytes.byteOffset, bytes.length);
    } else {
      rgba = new Uint8ClampedArray(width * height * 4);
      for (let i = 0, j = 0; i < bytes.length; i += 3, j += 4) {
        rgba[j] = bytes[i];
        rgba[j + 1] = bytes[i + 1];
        rgba[j + 2] = bytes[i + 2];
        rgba[j + 3] = 255;
      }
    }
    return this.decoder.fromRgba(rgba, width, height);
  }

  /** 全部捨てる。 */
  dispose(): void {
    for (const [id, entry] of [...this.entries]) this.drop(id, entry);
  }
}
