//! 列の数が変わったときの、折り返した行の組み直し（reflow）。

use std::collections::VecDeque;

use super::{Buffer, Line};
use crate::screen::cell::Cell;

/// 組み直しのあとも追いかける位置（カーソルなど）。
struct Tracked {
    /// 組み直す前の、バッファ全体での行番号（スクロールバックの先頭が 0）。
    abs: usize,
    x: usize,
    /// 折り返し待ちか。
    pending: bool,
    /// 組み直したあとの（行番号, 列, 折り返し待ち）。
    result: Option<(usize, usize, bool)>,
}

/// 折り返しでつながった行を 1 本にしたときの長さ（セルの数）。
fn trimmed_len(cells: &[Cell]) -> usize {
    cells
        .iter()
        .rposition(|c| !c.is_empty())
        .map_or(0, |i| i + 1)
}

/// 1 本に戻した行を `cols` 列で折り返したときの各行の（始まり, 長さ）。
/// 行末に全角の字が入りきらないときは、その行を 1 セル短くして字を次の行へ送る。
fn layout(flat: &[Cell], cols: usize) -> Vec<(usize, usize)> {
    let total = flat.len();
    let mut rows = Vec::new();
    let mut pos = 0;
    while pos < total || rows.is_empty() {
        let mut take = cols.min(total - pos);
        if take == cols && pos + take < total && flat[pos + take - 1].width == 2 {
            take -= 1;
        }
        rows.push((pos, take));
        pos += take;
    }
    rows
}

/// 1 本に戻した行の `offset` 番目の字が、`layout` のどこに来るか（行, 列, 折り返し待ち）。
/// 内容の外（行末より右）は、最後の行の右に空きがあれば列をそのまま、
/// ちょうど端なら折り返し待ち、はみ出すなら端に寄せる。
fn locate(
    layout: &[(usize, usize)],
    total: usize,
    cols: usize,
    offset: usize,
) -> (usize, usize, bool) {
    if offset < total {
        let r = layout
            .iter()
            .rposition(|&(s, _)| s <= offset)
            .expect("layout starts at 0");
        return (r, offset - layout[r].0, false);
    }
    let r = layout.len() - 1;
    let col = offset - layout[r].0;
    match col.cmp(&cols) {
        std::cmp::Ordering::Less => (r, col, false),
        std::cmp::Ordering::Equal => (r, cols - 1, true),
        std::cmp::Ordering::Greater => (r, cols - 1, false),
    }
}

impl Buffer {
    /// 列の数を `cols` にして、折り返しでつながった行を 1 本に戻してから折り直す
    /// （スクロールバックも）。行の数は変えない。カーソルと保存したカーソルは、
    /// 組み直したあとも同じ字の上に来る。ただし見える範囲より上（スクロールバック）
    /// に出てしまうときは、列はそのままで一番上の行に寄せる。
    pub(super) fn reflow(&mut self, cols: usize) {
        let rows = self.rows;
        let old: Vec<Line> = std::mem::take(&mut self.lines).into();
        let n = old.len();
        let view_start = n - rows;

        let mut tracked = vec![Tracked {
            abs: view_start + self.cursor.y.min(rows - 1),
            x: self.cursor.x,
            pending: self.cursor.pending_wrap,
            result: None,
        }];
        if let Some(s) = &self.saved {
            tracked.push(Tracked {
                abs: view_start + s.y.min(rows - 1),
                x: s.x,
                pending: s.pending_wrap,
                result: None,
            });
        }

        let mut out: VecDeque<Line> = VecDeque::with_capacity(n);
        let mut old = old;
        let mut i = 0;
        while i < n {
            let mut j = i;
            while j + 1 < n && old[j].wrapped {
                j += 1;
            }
            rewrap_group(&mut old[i..=j], i, cols, &mut out, &mut tracked);
            i = j + 1;
        }

        // 行が足りなければ下に空行を足す
        while out.len() < rows {
            out.push_back(Line::blank(cols, Default::default()));
        }
        let cursor = tracked[0].result.expect("cursor is tracked");
        // 行が増えて画面に収まらないときは、カーソルより下の空行から先に捨てる
        // （xterm.js と同じく、上の行をスクロールバックへ送るのはそのあと）
        while out.len() > rows && out.len() - 1 > cursor.0 && out[out.len() - 1].is_blank() {
            out.pop_back();
        }
        // 見える範囲は常に末尾の `rows` 行。中身のある行は捨てない
        let start0 = out.len() - rows;
        // スクロールバックの上限を超えた古い行を捨てる
        let excess = start0.saturating_sub(self.max_scrollback);
        for _ in 0..excess {
            out.pop_front();
        }

        self.lines = out;
        self.cols = cols;
        self.cursor.x = cursor.1;
        self.cursor.y = cursor.0.saturating_sub(start0).min(rows - 1);
        self.cursor.pending_wrap = cursor.2;
        if let Some(s) = &mut self.saved {
            let (abs, x, pending) = tracked[1].result.expect("saved cursor is tracked");
            s.y = abs.saturating_sub(start0).min(rows - 1);
            s.x = x;
            s.pending_wrap = pending;
        }
    }
}

/// 折り返しでつながった行 `group`（バッファの `first_abs` 行目から）を `cols` 列で組み直し、
/// `out` の末尾に足す。`tracked` のうちこの行たちの上にあるものは、新しい位置を決める。
fn rewrap_group(
    group: &mut [Line],
    first_abs: usize,
    cols: usize,
    out: &mut VecDeque<Line>,
    tracked: &mut [Tracked],
) {
    let last_wrapped = group[group.len() - 1].wrapped;
    let out_start = out.len();
    let last = group.len() - 1;
    let single = group.len() == 1;

    // 1 本に戻す。折り返した行の最後の空のセルは、全角の字が入りきらなかった余りなので捨てる。
    // 最後の行は、右端の空のセルを捨てる。
    let mut starts = Vec::with_capacity(group.len());
    let mut flat: Vec<Cell> = Vec::new();
    for k in 0..group.len() {
        starts.push(flat.len());
        // 次の行が全角の字で始まるなら、この行の最後の空のセルは入りきらなかった余り
        let next_wide = group.get(k + 1).is_some_and(|l| l.cells[0].width == 2);
        let mut cells = std::mem::take(&mut group[k].cells);
        let keep = if single && trimmed_len(&cells) <= cols {
            // 組み直す必要が無い行は、右端の空のセルの属性も残す
            cells.len().min(cols)
        } else if k == last {
            trimmed_len(&cells)
        } else if next_wide && cells.last().is_some_and(|c| c.is_empty()) {
            cells.len() - 1
        } else {
            cells.len()
        };
        cells.truncate(keep);
        flat.append(&mut cells);
    }
    if cols < 2 {
        // 1 列には全角の字が入らない
        for c in &mut flat {
            if c.width == 2 || c.is_wide_tail() {
                *c = Cell::default();
            }
        }
    }

    let lay = layout(&flat, cols);
    let total = flat.len();

    for t in tracked.iter_mut() {
        if t.abs >= first_abs && t.abs <= first_abs + last {
            let offset = starts[t.abs - first_abs] + t.x + usize::from(t.pending);
            let (r, x, p) = locate(&lay, total, cols, offset);
            t.result = Some((out_start + r, x, p));
        }
    }

    let nrows = lay.len();
    let mut cells = flat.into_iter();
    for (r, &(_, len)) in lay.iter().enumerate() {
        let mut row: Vec<Cell> = cells.by_ref().take(len).collect();
        row.resize(cols, Cell::default());
        out.push_back(Line {
            cells: row,
            wrapped: if r + 1 < nrows { true } else { last_wrapped },
            dirty: true,
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::screen::{Cell, Screen};

    fn text(cells: &[Cell]) -> String {
        cells
            .iter()
            .map(|c| if c.is_empty() { " ".into() } else { c.text() })
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    fn row(s: &Screen, y: usize) -> String {
        text(s.row(y).cells())
    }

    fn back(s: &Screen, i: usize) -> String {
        text(s.scrollback_row(i).cells())
    }

    /// 見えている行を、折り返しの印つき（`+` が付くと次の行へ続く）で並べる。
    fn rows(s: &Screen) -> Vec<String> {
        (0..s.rows())
            .map(|y| {
                format!(
                    "{}{}",
                    row(s, y),
                    if s.row(y).is_wrapped() { "+" } else { "" }
                )
            })
            .collect()
    }

    #[test]
    fn narrowing_wraps_a_long_line() {
        let mut s = Screen::new(4, 10);
        s.print_str("abcdefghij");
        s.resize(4, 4);
        assert_eq!(rows(&s), ["abcd+", "efgh+", "ij", ""]);
    }

    #[test]
    fn widening_joins_wrapped_lines() {
        let mut s = Screen::new(4, 4);
        s.print_str("abcdefghij");
        assert_eq!(rows(&s), ["abcd+", "efgh+", "ij", ""]);
        s.resize(4, 10);
        assert_eq!(rows(&s), ["abcdefghij", "", "", ""]);
        assert!(!s.row(0).is_wrapped());
    }

    #[test]
    fn hard_line_breaks_are_kept() {
        let mut s = Screen::new(4, 6);
        s.print_str("abc");
        s.carriage_return();
        s.index();
        s.print_str("de");
        s.resize(4, 2);
        assert_eq!(rows(&s), ["ab+", "c", "de", ""]);
        s.resize(4, 8);
        assert_eq!(rows(&s), ["abc", "de", "", ""]);
    }

    #[test]
    fn narrowing_then_widening_restores_the_line() {
        let mut s = Screen::new(5, 12);
        s.print_str("hello world!");
        s.resize(5, 5);
        s.resize(5, 12);
        assert_eq!(row(&s, 0), "hello world!");
        assert_eq!(row(&s, 1), "");
    }

    #[test]
    fn wide_char_at_the_edge_moves_to_the_next_line() {
        let mut s = Screen::new(3, 6);
        s.print_str("abあい");
        s.resize(3, 3);
        // 3 列目に「あ」の左半分だけは置けない
        assert_eq!(rows(&s), ["ab+", "あ+", "い"]);
        for y in 0..2 {
            let w = s.row(y).cells();
            assert!(w.iter().all(|c| !c.is_wide_tail() || w[0].width == 2));
        }
        assert_eq!(s.row(1).cells()[0].width, 2);
        assert!(s.row(1).cells()[1].is_wide_tail());
        // 戻すと余りの空きは消えて 1 本に戻る
        s.resize(3, 6);
        assert_eq!(rows(&s), ["abあい", "", ""]);
    }

    #[test]
    fn wide_chars_split_across_wrapped_lines_join_without_gaps() {
        let mut s = Screen::new(3, 5);
        // 5 列目に全角が入りきらず、4 列目の後ろに空きが残る
        s.print_str("abcdあ");
        assert_eq!(rows(&s), ["abcd+", "あ", ""]);
        s.resize(3, 8);
        assert_eq!(rows(&s), ["abcdあ", "", ""]);
    }

    #[test]
    fn one_column_drops_wide_chars() {
        let mut s = Screen::new(6, 4);
        s.print_str("aあb");
        s.resize(6, 1);
        assert_eq!(s.cols(), 1);
        assert_eq!(rows(&s)[..4], ["a+", "+", "+", "b"]);
    }

    #[test]
    fn wrapped_lines_in_scrollback_are_reflowed() {
        let mut s = Screen::new(2, 4);
        s.print_str("abcdefghijkl"); // 3 行ぶん。先頭の 1 行はスクロールバックへ
        s.print_str("m");
        assert_eq!(s.scrollback_len(), 2);
        s.resize(2, 13);
        assert_eq!(s.scrollback_len(), 0);
        assert_eq!(row(&s, 0), "abcdefghijklm");
    }

    #[test]
    fn line_spanning_scrollback_and_screen_is_reflowed() {
        let mut s = Screen::new(2, 4);
        s.print_str("abcdefghij"); // abcd+ / efgh+ / ij
        assert_eq!(s.scrollback_len(), 1);
        assert_eq!(back(&s, 0), "abcd");
        s.resize(2, 10);
        assert_eq!(s.scrollback_len(), 0);
        assert_eq!(rows(&s), ["abcdefghij", ""]);
        s.resize(2, 3);
        assert_eq!(s.scrollback_len(), 2);
        assert_eq!(back(&s, 0), "abc");
        assert_eq!(back(&s, 1), "def");
        assert_eq!(rows(&s), ["ghi+", "j"]);
    }

    #[test]
    fn scrollback_limit_is_applied_after_reflow() {
        let mut s = Screen::with_scrollback(2, 10, 2);
        for _ in 0..3 {
            s.print_str("0123456789");
            s.index();
            s.carriage_return();
        }
        s.resize(2, 2);
        assert_eq!(s.scrollback_len(), 2);
    }

    #[test]
    fn cursor_stays_on_the_same_char() {
        let mut s = Screen::new(5, 10);
        s.print_str("abcdefghij");
        s.carriage_return();
        s.index();
        s.print_str("xyz");
        s.set_cursor(7, 0); // 'h'
        s.resize(5, 4);
        // abcd+ / efgh+ / ij / xyz
        let c = s.cursor();
        assert_eq!((c.x, c.y, c.pending_wrap), (3, 1, false));
        assert_eq!(s.row(c.y).cells()[c.x].text(), "h");
        s.resize(5, 10);
        let c = s.cursor();
        assert_eq!((c.x, c.y), (7, 0));
        assert_eq!(s.row(0).cells()[7].text(), "h");
    }

    #[test]
    fn cursor_after_the_text_follows_it() {
        let mut s = Screen::new(4, 10);
        s.print_str("$ ls -la");
        s.resize(4, 5);
        // "$ ls " / "-la"
        let c = s.cursor();
        assert_eq!((c.x, c.y), (3, 1));
        s.print('X');
        assert_eq!(rows(&s), ["$ ls+", "-laX", "", ""]);
    }

    #[test]
    fn cursor_at_the_end_of_a_full_line_waits_to_wrap() {
        let mut s = Screen::new(3, 4);
        s.print_str("abc");
        s.resize(3, 3);
        let c = s.cursor();
        assert_eq!((c.x, c.y, c.pending_wrap), (2, 0, true));
        s.print('d');
        assert_eq!(rows(&s), ["abc+", "d", ""]);
    }

    #[test]
    fn cursor_moves_up_when_lines_are_joined() {
        let mut s = Screen::new(5, 4);
        s.print_str("abcdefghij");
        assert_eq!(s.cursor().y, 2);
        s.resize(5, 10);
        let c = s.cursor();
        assert_eq!((c.x, c.y, c.pending_wrap), (9, 0, true));
    }

    #[test]
    fn content_below_cursor_is_not_dropped_when_narrowing() {
        let mut s = Screen::new(3, 12);
        s.print_str("abcdefghijkl");
        s.carriage_return();
        s.index();
        s.print_str("second line");
        s.set_cursor(0, 0);
        s.resize(3, 3);
        // 下の行は捨てずに、見える範囲は末尾の 3 行（"second line" が残る）。カーソルは一番上に寄る
        assert_eq!(rows(&s), ["ond+", " li+", "ne"]);
        assert_eq!(s.cursor().y, 0);
        assert!(s.scrollback_len() > 0);
    }

    #[test]
    fn saved_cursor_follows_the_reflow() {
        let mut s = Screen::new(5, 10);
        s.print_str("abcdefghij");
        s.set_cursor(5, 0);
        s.save_cursor();
        s.set_cursor(0, 1);
        s.resize(5, 4);
        let saved = s.saved_cursor().unwrap();
        assert_eq!((saved.x, saved.y), (1, 1));
        assert_eq!(s.row(saved.y).cells()[saved.x].text(), "f");
    }

    #[test]
    fn alt_screen_is_not_reflowed() {
        let mut s = Screen::new(3, 6);
        s.enter_alt_screen();
        s.print_str("abcdefgh");
        assert_eq!(rows(&s)[..2], ["abcdef+", "gh"]);
        s.resize(3, 3);
        assert_eq!(rows(&s)[..2], ["abc+", "gh"]);
        // 主画面のほうは組み直される
        s.leave_alt_screen();
        s.print_str("abcdef");
        assert_eq!(s.cols(), 3);
        assert_eq!(rows(&s)[..2], ["abc+", "def"]);
    }

    #[test]
    fn primary_is_reflowed_while_alt_screen_is_active() {
        let mut s = Screen::new(3, 6);
        s.print_str("abcdefgh");
        s.enter_alt_screen();
        s.resize(3, 4);
        s.leave_alt_screen();
        assert_eq!(rows(&s), ["abcd+", "efgh", ""]);
    }

    #[test]
    fn growing_rows_pulls_lines_back_from_scrollback() {
        let mut s = Screen::new(2, 5);
        for w in ["a", "b", "c", "d"] {
            s.print_str(w);
            s.carriage_return();
            s.index();
        }
        // a, b, c は押し出されて、画面は "d" と空行。カーソルは最下行
        assert_eq!(s.scrollback_len(), 3);
        s.resize(4, 5);
        assert_eq!(s.scrollback_len(), 1);
        assert_eq!(back(&s, 0), "a");
        assert_eq!(rows(&s), ["b", "c", "d", ""]);
        assert_eq!(s.cursor().y, 3);
    }

    #[test]
    fn growing_rows_adds_blank_lines_when_cursor_is_not_at_the_bottom() {
        let mut s = Screen::new(3, 5);
        for w in ["a", "b", "c", "d"] {
            s.print_str(w);
            s.carriage_return();
            s.index();
        }
        s.set_cursor(0, 0);
        let before = s.scrollback_len();
        s.resize(5, 5);
        assert_eq!(s.scrollback_len(), before);
        assert_eq!(s.cursor().y, 0);
    }

    #[test]
    fn shrinking_rows_sends_top_lines_to_scrollback() {
        let mut s = Screen::new(4, 5);
        for w in ["a", "b", "c", "d"] {
            s.print_str(w);
            s.carriage_return();
            s.index();
        }
        s.set_cursor(0, 3);
        s.resize(2, 5);
        assert_eq!(s.scrollback_len(), 3);
        assert_eq!(back(&s, 1), "b");
        assert_eq!(s.cursor().y, 1);
    }

    #[test]
    fn rows_and_columns_change_together() {
        let mut s = Screen::new(3, 8);
        s.print_str("abcdefgh");
        s.resize(5, 4);
        assert_eq!(s.rows(), 5);
        assert_eq!(rows(&s)[..2], ["abcd+", "efgh"]);
    }

    #[test]
    fn tab_stops_follow_the_new_width() {
        let mut s = Screen::new(2, 8);
        s.resize(2, 20);
        assert_eq!(s.tab_stops(), vec![8, 16]);
    }

    #[test]
    fn keeps_background_of_blank_cells_when_nothing_wraps() {
        let mut s = Screen::new(2, 6);
        s.pen.bg = crate::screen::Color::Palette(1);
        s.print_str("ab");
        s.erase_cells(0, 2, 6);
        s.resize(2, 4);
        assert_eq!(
            s.row(0).cells()[3].attrs.bg,
            crate::screen::Color::Palette(1)
        );
    }

    #[test]
    fn lines_are_marked_dirty() {
        let mut s = Screen::new(2, 6);
        s.print_str("ab");
        s.take_dirty_rows();
        s.resize(2, 3);
        assert_eq!(s.dirty_rows(), vec![0, 1]);
    }
}
