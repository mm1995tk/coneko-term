/** 画素の並び（RGBA、1 画素 4 バイト、まっすぐなアルファ）とその大きさ。 */
export interface Pixels {
  rgba: Uint8ClampedArray;
  width: number;
  height: number;
}

/** 1 色（0xRRGGBBAA）で塗った画素の並び。 */
export function solid(width: number, height: number, color: number): Pixels {
  const rgba = new Uint8ClampedArray(width * height * 4);
  const r = (color >>> 24) & 0xff;
  const g = (color >>> 16) & 0xff;
  const b = (color >>> 8) & 0xff;
  const a = color & 0xff;
  if (r || g || b || a) {
    for (let i = 0; i < rgba.length; i += 4) {
      rgba[i] = r;
      rgba[i + 1] = g;
      rgba[i + 2] = b;
      rgba[i + 3] = a;
    }
  }
  return { rgba, width, height };
}

/**
 * `src` の四角（`sx`, `sy` から幅 `w`・高さ `h`）を、`dst` の `dx`, `dy` へ重ねる。
 * `overwrite` なら置き換え、そうでなければアルファ合成（`src` が上）。どちらかの画素の外に
 * はみ出す分は切る。
 */
export function compose(
  dst: Pixels,
  src: Pixels,
  sx: number,
  sy: number,
  dx: number,
  dy: number,
  w: number,
  h: number,
  overwrite: boolean,
): void {
  // はみ出す分を切る
  const w2 = Math.min(w, src.width - sx, dst.width - dx);
  const h2 = Math.min(h, src.height - sy, dst.height - dy);
  for (let y = 0; y < h2; y++) {
    let s = ((sy + y) * src.width + sx) * 4;
    let d = ((dy + y) * dst.width + dx) * 4;
    for (let x = 0; x < w2; x++, s += 4, d += 4) {
      const sa = src.rgba[s + 3];
      if (overwrite || sa === 255) {
        dst.rgba[d] = src.rgba[s];
        dst.rgba[d + 1] = src.rgba[s + 1];
        dst.rgba[d + 2] = src.rgba[s + 2];
        dst.rgba[d + 3] = sa;
        continue;
      }
      if (sa === 0) continue;
      const da = dst.rgba[d + 3];
      // 上に重ねる（over）: 出力のアルファ = sa + da * (1 - sa)
      const wd = (da * (255 - sa)) / 255;
      const oa = sa + wd;
      for (let c = 0; c < 3; c++) {
        dst.rgba[d + c] = (src.rgba[s + c] * sa + dst.rgba[d + c] * wd) / oa;
      }
      dst.rgba[d + 3] = oa;
    }
  }
}

/** 画素の並びの写し。 */
export function clone(p: Pixels): Pixels {
  return { rgba: p.rgba.slice(), width: p.width, height: p.height };
}

/** 生の RGB（1 画素 3 バイト）か RGBA（4 バイト）から、RGBA の画素の並びを作る。長さが合わなければ `undefined`。 */
export function fromRaw(
  bytes: Uint8Array,
  bpp: 3 | 4,
  width: number,
  height: number,
): Pixels | undefined {
  if (!(width > 0) || !(height > 0) || bytes.length !== width * height * bpp) {
    return undefined;
  }
  if (bpp === 4) {
    return { rgba: new Uint8ClampedArray(bytes), width, height };
  }
  const rgba = new Uint8ClampedArray(width * height * 4);
  for (let i = 0, j = 0; i < bytes.length; i += 3, j += 4) {
    rgba[j] = bytes[i];
    rgba[j + 1] = bytes[i + 1];
    rgba[j + 2] = bytes[i + 2];
    rgba[j + 3] = 255;
  }
  return { rgba, width, height };
}
