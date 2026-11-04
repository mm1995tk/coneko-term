//! 選択の範囲の計算。範囲を字にする、ダブルクリックの単語、トリプルクリックの行。
//!
//! 位置は、スクロールバックと見えている画面をつなげた行の並び（0 がスクロールバックの
//! 一番古い行。代替画面ではスクロールバックが無いので、画面の 0 行目）での
//! `(行, 列)`。範囲は `[start, end)`（`end` は含まない）。
//!
//! 選択の状態（どこからどこまでか、ドラッグ中か）は呼ぶ側が持つ。ここは、画面の中身を読んで
//! 計算するだけ。xterm.js 6.0.0 の `SelectionService` に合わせてある。

use crate::screen::{Cell, Line, Screen};

/// 単語の区切りの既定の字（xterm.js 6.0.0 の `wordSeparator` の既定。空白も区切り）。
pub const DEFAULT_WORD_SEPARATORS: &str = " ()[]{}',\"`";

/// 行の並びの中での位置。大小は行、列の順。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos {
    /// 行。
    pub row: usize,
    /// 列（0 始まり）。
    pub col: usize,
}

impl Pos {
    /// 位置。
    pub fn new(row: usize, col: usize) -> Pos {
        Pos { row, col }
    }
}

/// 行の数（スクロールバック + 画面）。
pub fn total_rows(screen: &Screen) -> usize {
    screen.scrollback_len() + screen.rows()
}

fn line(screen: &Screen, row: usize) -> &Line {
    let sb = screen.scrollback_len();
    if row < sb {
        screen.scrollback_row(row)
    } else {
        screen.row(row - sb)
    }
}

/// `row` 行目から、折り返しでつながった行の範囲 `(最初の行, 最後の行)` を出す。
fn logical_line(screen: &Screen, row: usize) -> (usize, usize) {
    let total = total_rows(screen);
    let mut first = row;
    while first > 0 && line(screen, first - 1).is_wrapped() {
        first -= 1;
    }
    let mut last = row;
    while last + 1 < total && line(screen, last).is_wrapped() {
        last += 1;
    }
    (first, last)
}

/// 行のうち、折り返しでつながった行全体 `[start, end)`。
pub fn line_range(screen: &Screen, row: usize) -> (Pos, Pos) {
    let row = row.min(total_rows(screen) - 1);
    let (first, last) = logical_line(screen, row);
    (Pos::new(first, 0), Pos::new(last, screen.cols()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    Word,
    Separator,
}

fn classify(cell: &Cell, separators: &str) -> Class {
    match cell.text().chars().next() {
        None | Some(' ') => Class::Space,
        Some(c) if separators.contains(c) => Class::Separator,
        Some(_) => Class::Word,
    }
}

/// `(row, col)` の字を含む単語 `[start, end)`。折り返しでつながった行をまたぐ。
///
/// 区切りの字（`separators`。空白は区切りでなくても空白として扱う）でない字が続く範囲が単語。
/// 空白の上なら空白が続く範囲、区切りの字の上ならその 1 字だけ。全角の字は左右で 1 字。
pub fn word_at(screen: &Screen, row: usize, col: usize, separators: &str) -> (Pos, Pos) {
    let total = total_rows(screen);
    let row = row.min(total - 1);
    let cols = screen.cols();
    let (first, last) = logical_line(screen, row);
    // 行をつなげたセルの並び。`i` 番目は (first + i / cols, i % cols)
    let n = (last - first + 1) * cols;
    let cell_at = |i: usize| &line(screen, first + i / cols).cells()[i % cols];
    // 全角の右半分は左半分として扱う
    let head = |mut i: usize| {
        while i > 0 && (cell_at(i).is_wide_tail() || cell_at(i).is_sized_tail_right()) {
            i -= 1;
        }
        i
    };
    let pos_of = |i: usize| Pos::new(first + i / cols, i % cols);
    let class_at = |i: usize| classify(cell_at(head(i)), separators);
    let at = head((row - first) * cols + col.min(cols - 1));
    let class = class_at(at);
    let mut start = at;
    let mut end = at + 1;
    if class != Class::Separator {
        while start > 0 && class_at(start - 1) == class {
            start = head(start - 1);
        }
        while end < n && class_at(end) == class {
            end += 1;
        }
    }
    let start_pos = pos_of(start);
    let end_pos = if end % cols == 0 {
        // 行の最後まで: 次の行の頭ではなく、その行の端（列の数）にする
        Pos::new(first + end / cols - 1, cols)
    } else {
        pos_of(end)
    };
    (start_pos, end_pos)
}

/// 範囲 `[start, end)` の字。
///
/// - 折り返しでつながった行は、改行を入れずにつなげる。それ以外は `\n` で区切る。
/// - 行末の何も書かれていないセルは捨てる（折り返す行の末尾は、空白として残す）。
/// - 全角の字は 1 字。空のセルは、行の途中なら空白。
pub fn text(screen: &Screen, start: Pos, end: Pos) -> String {
    let total = total_rows(screen);
    if total == 0 || start >= end {
        return String::new();
    }
    let cols = screen.cols();
    let last_row = end.row.min(total - 1);
    let end = if end.row > last_row {
        Pos::new(last_row, cols)
    } else {
        end
    };
    let mut out = String::new();
    for row in start.row..=last_row {
        let l = line(screen, row);
        let c0 = if row == start.row {
            start.col.min(cols)
        } else {
            0
        };
        let c1 = if row == end.row {
            end.col.min(cols)
        } else {
            cols
        };
        let wraps = l.is_wrapped() && row < last_row;
        let cells = l.cells();
        // 折り返さない行は、最後に字のあるセルまで
        let mut stop = c1;
        if !wraps {
            while stop > c0 && cells[stop - 1].is_empty() {
                stop -= 1;
            }
        }
        for cell in &cells[c0.min(stop)..stop] {
            match cell.text() {
                s if s.is_empty() && !cell.is_continuation() => out.push(' '),
                s => out.push_str(&s),
            }
        }
        if !wraps && row < last_row {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(rows: usize, cols: usize, input: &str) -> Screen {
        let mut t = crate::Terminal::with_screen(Screen::with_scrollback(rows, cols, 100));
        t.write(input.as_bytes());
        t.screen().clone()
    }

    fn p(row: usize, col: usize) -> Pos {
        Pos::new(row, col)
    }

    fn word(s: &Screen, row: usize, col: usize) -> (Pos, Pos) {
        word_at(s, row, col, DEFAULT_WORD_SEPARATORS)
    }

    #[test]
    fn range_text_within_a_line() {
        let s = screen(3, 10, "hello world");
        // 1 行目 "hello worl" 2 行目 "d" （折り返し）
        assert_eq!(text(&s, p(0, 0), p(0, 5)), "hello");
        assert_eq!(text(&s, p(0, 6), p(0, 10)), "worl");
        assert_eq!(text(&s, p(0, 2), p(0, 2)), "");
    }

    #[test]
    fn wrapped_lines_are_joined_without_a_newline() {
        let s = screen(3, 10, "hello world");
        assert!(s.row(0).is_wrapped());
        assert_eq!(text(&s, p(0, 0), p(1, 1)), "hello world");
        // 折り返す行の末尾は切らない
        let s = screen(3, 5, "ab   cd");
        assert_eq!(text(&s, p(0, 0), p(1, 2)), "ab   cd");
    }

    #[test]
    fn separate_lines_get_a_newline_and_lose_trailing_blanks() {
        let s = screen(3, 10, "one\r\ntwo\r\n\r\nfour");
        assert_eq!(text(&s, p(0, 0), p(1, 10)), "one\ntwo");
        assert_eq!(text(&s, p(0, 0), p(3, 4)), "one\ntwo\n\nfour");
        // 途中までの選択
        assert_eq!(text(&s, p(0, 1), p(1, 2)), "ne\ntw");
        // 空のセルが範囲の最後でも切る
        assert_eq!(text(&s, p(0, 0), p(0, 8)), "one");
    }

    #[test]
    fn blank_cells_in_the_middle_are_spaces() {
        let s = screen(2, 10, "a\x1b[3Cb");
        assert_eq!(text(&s, p(0, 0), p(0, 10)), "a   b");
    }

    #[test]
    fn typed_spaces_at_line_end_are_kept_empty_cells_are_not() {
        // 空白を打ったセルは字なので残る
        let s = screen(2, 10, "ab  ");
        assert_eq!(text(&s, p(0, 0), p(0, 10)), "ab  ");
    }

    #[test]
    fn wide_characters_and_clusters_count_once() {
        let s = screen(2, 6, "あいx\u{65}\u{301}");
        assert_eq!(text(&s, p(0, 0), p(0, 6)), "あいxe\u{301}");
        // 全角の途中から始めても右半分は字にならない
        assert_eq!(text(&s, p(0, 1), p(0, 4)), "い");
    }

    #[test]
    fn text_spans_scrollback() {
        let s = screen(2, 8, "l1\r\nl2\r\nl3\r\nl4");
        assert_eq!(s.scrollback_len(), 2);
        assert_eq!(total_rows(&s), 4);
        assert_eq!(text(&s, p(0, 0), p(3, 8)), "l1\nl2\nl3\nl4");
        assert_eq!(text(&s, p(1, 0), p(2, 2)), "l2\nl3");
    }

    #[test]
    fn range_past_the_end_is_clamped() {
        let s = screen(2, 8, "ab");
        assert_eq!(text(&s, p(0, 0), p(50, 50)), "ab\n");
        assert_eq!(text(&s, p(5, 0), p(9, 0)), "");
    }

    #[test]
    fn word_selects_non_separator_run() {
        let s = screen(2, 30, "foo-bar (baz) 'qux'");
        assert_eq!(word(&s, 0, 1), (p(0, 0), p(0, 7)));
        assert_eq!(word(&s, 0, 6), (p(0, 0), p(0, 7)));
        // 括弧は区切りなので、その 1 字だけ
        assert_eq!(word(&s, 0, 8), (p(0, 8), p(0, 9)));
        assert_eq!(word(&s, 0, 10), (p(0, 9), p(0, 12)));
        assert_eq!(word(&s, 0, 12), (p(0, 12), p(0, 13)));
    }

    #[test]
    fn word_on_whitespace_selects_the_blank_run() {
        let s = screen(2, 30, "foo   bar");
        assert_eq!(word(&s, 0, 4), (p(0, 3), p(0, 6)));
        // 行末の空のセルも空白
        assert_eq!(word(&s, 0, 20), (p(0, 9), p(0, 30)));
    }

    #[test]
    fn word_follows_wrapped_lines() {
        let s = screen(3, 5, "abc defghijk lm");
        // "abc d" / "efghi" / "jk lm"
        assert!(s.row(0).is_wrapped() && s.row(1).is_wrapped());
        assert_eq!(word(&s, 1, 2), (p(0, 4), p(2, 2)));
        assert_eq!(word(&s, 0, 4), (p(0, 4), p(2, 2)));
        assert_eq!(text(&s, p(0, 4), p(2, 2)), "defghijk");
    }

    #[test]
    fn word_ending_at_the_last_column_ends_at_the_edge() {
        let s = screen(2, 5, "abcde\r\nfg");
        assert_eq!(word(&s, 0, 2), (p(0, 0), p(0, 5)));
    }

    #[test]
    fn word_handles_wide_characters() {
        let s = screen(2, 10, "あい う");
        assert_eq!(word(&s, 0, 1), (p(0, 0), p(0, 4)));
        assert_eq!(word(&s, 0, 2), (p(0, 0), p(0, 4)));
        assert_eq!(word(&s, 0, 4), (p(0, 4), p(0, 5)));
        assert_eq!(word(&s, 0, 5), (p(0, 5), p(0, 7)));
        assert_eq!(word(&s, 0, 6), (p(0, 5), p(0, 7)));
    }

    #[test]
    fn custom_separators() {
        let s = screen(2, 30, "a.b/c d");
        assert_eq!(word_at(&s, 0, 0, " "), (p(0, 0), p(0, 5)));
        assert_eq!(word_at(&s, 0, 0, " ./"), (p(0, 0), p(0, 1)));
    }

    #[test]
    fn line_range_covers_the_wrapped_lines() {
        let s = screen(5, 5, "abcdefghijk\r\nxyz");
        assert_eq!(line_range(&s, 0), (p(0, 0), p(2, 5)));
        assert_eq!(line_range(&s, 1), (p(0, 0), p(2, 5)));
        assert_eq!(line_range(&s, 2), (p(0, 0), p(2, 5)));
        assert_eq!(line_range(&s, 3), (p(3, 0), p(3, 5)));
        assert_eq!(text(&s, p(0, 0), p(2, 5)), "abcdefghijk");
    }
}
