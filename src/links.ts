import { CELL_STRIDE, CLUSTER_FLAG, type RowView } from "./terminal.js";

/** URL が載っている 1 行ぶんの区間。`row` は、スクロールバックの先頭を 0 とした通し番号。 */
export interface UrlSegment {
  row: number;
  /** 開始の列。 */
  startCol: number;
  /** 終わりの列（含まない）。 */
  endCol: number;
}

/** 見つけた URL。折り返した URL は、複数の行に区間が分かれる。 */
export interface FoundUrl {
  url: string;
  segments: UrlSegment[];
}

/** 1 つの論理行（折り返しでつながった行の並び）に使う行数の上限。 */
const MAX_LOGICAL_ROWS = 64;

/** 論理行の先頭を探して戻る行数の上限。 */
const SEARCH_BACK = 4096;

// `http://` と `https://` で始まり、空白と URL に使わない ASCII の記号、ASCII 以外の字の手前まで。
// 日本語の文の中に URL が埋まっていても、URL の後ろの字を巻き込まない。
const URL_RE = /\bhttps?:\/\/[^\s\x00-\x20\x7f-\uffff<>"'`\\^{}|]+/gi;

const CLOSERS: Record<string, string> = { ")": "(", "]": "[" };

/** 文の区切りや、対応する開きの無い閉じ括弧が URL の末尾に付いていたら外す。 */
function trimTail(s: string): string {
  for (;;) {
    const last = s[s.length - 1];
    if (last === undefined) return s;
    if (".,;:!?*".includes(last)) {
      s = s.slice(0, -1);
      continue;
    }
    const open = CLOSERS[last];
    if (open !== undefined) {
      let opens = 0;
      let closes = 0;
      for (const c of s) {
        if (c === open) opens++;
        else if (c === last) closes++;
      }
      if (closes > opens) {
        s = s.slice(0, -1);
        continue;
      }
    }
    return s;
  }
}

/**
 * `absRow` 行目を含む論理行（折り返しでつながった行）にある URL を返す。
 * `read` は通し番号の行を読む。`total` は行の総数。
 */
export function findUrlsAround(
  read: (absRow: number) => RowView,
  total: number,
  absRow: number,
): FoundUrl[] {
  // 論理行は、本当の先頭から数えて 64 行ずつの決まった区切りで分ける（どの行から見ても同じ窓になり、
  // `absRow` の行は必ず窓に入る。64 行以内の論理行は切れない）。先頭を探して戻るのは SEARCH_BACK 行まで。
  // それより長い論理行だけは、通し番号の 64 の倍数で区切る。
  let start = absRow;
  while (start > 0 && absRow - start < SEARCH_BACK && read(start - 1).wrapped) start--;
  const found0 = start === 0 || !read(start - 1).wrapped;
  const base = found0 ? start : 0;
  const chunk = base + Math.floor((absRow - base) / MAX_LOGICAL_ROWS) * MAX_LOGICAL_ROWS;
  let first = absRow;
  while (first > chunk && read(first - 1).wrapped) first--;
  // UTF-16 の 1 単位ごとの位置
  const rowOf: number[] = [];
  const colOf: number[] = [];
  const endOf: number[] = [];
  let text = "";
  for (let r = first; r < total && r < chunk + MAX_LOGICAL_ROWS; r++) {
    const row = read(r);
    for (let x = 0; x < row.length; x++) {
      const o = x * CELL_STRIDE;
      const cp = row.cells[o];
      const width = row.cells[o + 1];
      if (width === 0) continue;
      let piece: string;
      if (row.cells[o + 6] & 0xffff) piece = " "; // OSC 66 の字は URL とは見なさない
      else if (cp === 0) piece = " ";
      else if (cp & CLUSTER_FLAG) piece = row.cluster(cp & ~CLUSTER_FLAG);
      else piece = String.fromCodePoint(cp);
      for (let i = 0; i < piece.length; i++) {
        rowOf.push(r);
        colOf.push(x);
        endOf.push(x + width);
      }
      text += piece;
    }
    if (!row.wrapped) break;
  }
  const found: FoundUrl[] = [];
  URL_RE.lastIndex = 0;
  for (let m = URL_RE.exec(text); m; m = URL_RE.exec(text)) {
    const url = trimTail(m[0]);
    if (!/^https?:\/\/./i.test(url)) continue;
    const start = m.index;
    const end = start + url.length; // 含まない
    const segments: UrlSegment[] = [];
    for (let i = start; i < end; i++) {
      const last = segments[segments.length - 1];
      if (last && last.row === rowOf[i]) last.endCol = endOf[i];
      else segments.push({ row: rowOf[i], startCol: colOf[i], endCol: endOf[i] });
    }
    found.push({ url, segments });
  }
  return found;
}
