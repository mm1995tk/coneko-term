/** 行の並び（0 がスクロールバックの一番古い行。代替画面では画面の 0 行目）の中の位置。 */
export interface BufferPosition {
  /** 行。 */
  row: number;
  /** 列（0 始まり）。 */
  col: number;
}

/** 選択の範囲。`end` は含まない。 */
export interface SelectionRange {
  start: BufferPosition;
  end: BufferPosition;
}

/** 選択が、画面の中身を読むために使うもの。`Terminal` が渡す。 */
export interface SelectionHost {
  /** 列の数。 */
  readonly cols: number;
  /** 行の数（スクロールバック + 画面）。 */
  totalRows(): number;
  /** スクロールバックの上から捨てた行の数の合計。 */
  trimmed(): number;
  /** `[開始の行, 開始の列, 終わりの行, 終わりの列]`（終わりは含まない）。 */
  wordRange(row: number, col: number): number[];
  /** 折り返しでつながった行全体。並びは `wordRange` と同じ。 */
  lineRange(row: number): number[];
}

/** 選択のしかた。 */
export type SelectionKind = "cell" | "word" | "line";

function cmp(a: BufferPosition, b: BufferPosition): number {
  return a.row - b.row || a.col - b.col;
}

function pos(row: number, col: number): BufferPosition {
  return { row, col };
}

/**
 * 選択の状態。ドラッグで範囲、ダブルクリックで単語、トリプルクリックで行を選ぶ。
 *
 * 位置は、スクロールバックから行が捨てられても同じ行を指すように、`trimmed()` を足した
 * 通し番号の行で覚える。範囲を返すときに、いまの行の番号に直す。
 */
export class SelectionModel {
  private kind: SelectionKind = "cell";
  /** 最初に押した場所。単語・行のときは、その単語・行の `[a, b)`。 */
  private a?: BufferPosition;
  private b?: BufferPosition;
  /** ドラッグしている場所。単語・行のときは、その場所の単語・行の `[ca, cb)`。 */
  private cur?: BufferPosition;
  private curEnd?: BufferPosition;

  constructor(private host: SelectionHost) {}

  private stable(p: BufferPosition): BufferPosition {
    return pos(p.row + this.host.trimmed(), p.col);
  }

  private current(p: BufferPosition): BufferPosition {
    return pos(p.row - this.host.trimmed(), p.col);
  }

  private unit(kind: SelectionKind, p: BufferPosition): [BufferPosition, BufferPosition] {
    const [r0, c0, r1, c1] =
      kind === "word"
        ? this.host.wordRange(p.row, p.col)
        : this.host.lineRange(p.row);
    return [pos(r0, c0), pos(r1, c1)];
  }

  /** 押した場所から選び始める。`kind` が `word` か `line` なら、その単語・行をすぐ選ぶ。 */
  start(kind: SelectionKind, at: BufferPosition): void {
    const p = this.clamp(at);
    this.kind = kind;
    if (kind === "cell") {
      this.a = this.stable(p);
      this.b = undefined;
      this.cur = this.a;
      this.curEnd = undefined;
    } else {
      const [s, e] = this.unit(kind, p);
      this.a = this.stable(s);
      this.b = this.stable(e);
      this.cur = this.a;
      this.curEnd = this.b;
    }
  }

  /** ドラッグして `at` まで広げる（広げ始めていなければ何もしない）。 */
  extend(at: BufferPosition): void {
    if (!this.a) return;
    const p = this.clamp(at);
    if (this.kind === "cell") {
      this.cur = this.stable(p);
    } else {
      const [s, e] = this.unit(this.kind, p);
      this.cur = this.stable(s);
      this.curEnd = this.stable(e);
    }
  }

  /**
   * Shift を押してクリックしたとき、いまの選択を `at` まで広げる。選択が無ければ、`at` から
   * 選び始める。
   */
  extendSelection(at: BufferPosition): void {
    const r = this.range();
    if (!r) {
      this.start("cell", at);
      return;
    }
    const p = this.clamp(at);
    this.kind = "cell";
    this.b = undefined;
    this.curEnd = undefined;
    // 動かさない側を、最初に押した場所にする
    const last = pos(r.end.row, r.end.col - 1);
    const anchor = cmp(p, r.start) < 0 ? last : r.start;
    this.a = this.stable(anchor);
    this.cur = this.stable(p);
  }

  /** 全部を選ぶ。 */
  selectAll(): void {
    const total = this.host.totalRows();
    if (total === 0) return;
    this.kind = "cell";
    this.a = this.stable(pos(0, 0));
    this.b = undefined;
    this.cur = this.stable(pos(total - 1, this.host.cols - 1));
    this.curEnd = undefined;
  }

  /** 選択を消す。 */
  clear(): void {
    this.a = this.b = this.cur = this.curEnd = undefined;
  }

  /** 選択が空でないか。 */
  get active(): boolean {
    return this.range() !== undefined;
  }

  private clamp(p: BufferPosition): BufferPosition {
    const rows = Math.max(1, this.host.totalRows());
    return pos(
      Math.max(0, Math.min(rows - 1, Math.trunc(p.row))),
      Math.max(0, Math.min(this.host.cols - 1, Math.trunc(p.col))),
    );
  }

  /**
   * いまの範囲（`end` は含まない）。空のとき（まだ動かしていない、捨てられた行だけ）は `undefined`。
   */
  range(): SelectionRange | undefined {
    if (!this.a || !this.cur) return undefined;
    let start: BufferPosition;
    let end: BufferPosition;
    if (this.kind === "cell") {
      const a = this.current(this.a);
      const c = this.current(this.cur);
      if (cmp(a, c) === 0) return undefined;
      const [lo, hi] = cmp(a, c) < 0 ? [a, c] : [c, a];
      start = lo;
      end = pos(hi.row, hi.col + 1);
    } else {
      const a = this.current(this.a);
      const b = this.current(this.b ?? this.a);
      const ca = this.current(this.cur);
      const cb = this.current(this.curEnd ?? this.cur);
      start = cmp(a, ca) <= 0 ? a : ca;
      end = cmp(b, cb) >= 0 ? b : cb;
    }
    // 捨てられた行にかかった分は切る
    if (end.row < 0) return undefined;
    if (start.row < 0) start = pos(0, 0);
    if (cmp(start, end) >= 0) return undefined;
    return { start, end };
  }
}
