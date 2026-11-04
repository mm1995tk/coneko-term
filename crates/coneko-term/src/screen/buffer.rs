//! 1 枚分の画面（行の並びとカーソルなど）。主画面と代替画面で 1 つずつ持つ。

use std::collections::VecDeque;

use unicode_width::UnicodeWidthStr;

use super::cell::{Attrs, Cell, Content};

/// 行。
#[derive(Clone, Debug)]
pub struct Line {
    pub(crate) cells: Vec<Cell>,
    pub(crate) wrapped: bool,
    pub(crate) dirty: bool,
}

impl Line {
    pub(crate) fn blank(cols: usize, attrs: Attrs) -> Line {
        Line {
            cells: vec![Cell::blank(attrs); cols],
            wrapped: false,
            dirty: true,
        }
    }

    /// 行のセル。列数と同じ数ある。
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// この行が、次の行へ折り返している（次の行に続く）か。
    pub fn is_wrapped(&self) -> bool {
        self.wrapped
    }

    /// 前に描いてから変わったか。
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// 全角の字の半分が残らないよう、`x` のセルを上書きする前に相方を消す。
    fn clear_overlap(&mut self, x: usize) {
        if self.cells[x].is_wide_tail() && x > 0 {
            self.cells[x - 1] = Cell::default();
        }
        if self.cells[x].width == 2 && x + 1 < self.cells.len() {
            self.cells[x + 1] = Cell::default();
        }
    }

    /// `x` が全角の字の右半分なら、その字を左右とも消す。
    fn split_wide_at(&mut self, x: usize) {
        if self.cells[x].is_wide_tail() && x > 0 {
            self.cells[x - 1] = Cell::default();
            self.cells[x] = Cell::default();
        }
    }

    /// 相方のいない全角の字の半分を消す。
    fn fix_wide(&mut self) {
        let n = self.cells.len();
        for i in 0..n {
            let c = &self.cells[i];
            let orphan = if c.width == 2 {
                !(i + 1 < n && self.cells[i + 1].is_wide_tail())
            } else if c.is_wide_tail() {
                !(i > 0 && self.cells[i - 1].width == 2)
            } else {
                false
            };
            if orphan {
                self.cells[i] = Cell::default();
            }
        }
    }

    fn resize(&mut self, cols: usize) {
        self.cells.resize(cols, Cell::default());
        // 切り詰めて全角の字の右半分だけが消えたら、左半分も消す
        if let Some(last) = self.cells.last_mut() {
            if last.width == 2 {
                *last = Cell::default();
            }
        }
        self.dirty = true;
    }
}

/// カーソルの位置。見た目の形は `Screen` が持つ。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CursorPos {
    pub x: usize,
    pub y: usize,
    /// 行末まで書いて、次の字で折り返す状態。
    pub pending_wrap: bool,
}

/// 保存したカーソル（DECSC）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SavedCursor {
    /// 列（0 始まり）。
    pub x: usize,
    /// 行（0 始まり）。
    pub y: usize,
    /// 折り返し待ちだったか。
    pub pending_wrap: bool,
    /// そのときの字の属性。
    pub attrs: Attrs,
}

#[derive(Clone, Debug)]
pub(crate) struct Buffer {
    /// 先頭がスクロールバックの一番古い行。末尾の `rows` 行が見えている画面。
    pub lines: VecDeque<Line>,
    pub rows: usize,
    pub cols: usize,
    pub max_scrollback: usize,
    pub cursor: CursorPos,
    pub saved: Option<SavedCursor>,
    pub scroll_top: usize,
    pub scroll_bottom: usize,
    pub tabs: Vec<bool>,
}

fn default_tabs(cols: usize) -> Vec<bool> {
    (0..cols).map(|x| x % 8 == 0 && x != 0).collect()
}

impl Buffer {
    pub fn new(rows: usize, cols: usize, max_scrollback: usize) -> Buffer {
        Buffer {
            lines: (0..rows)
                .map(|_| Line::blank(cols, Attrs::default()))
                .collect(),
            rows,
            cols,
            max_scrollback,
            cursor: CursorPos::default(),
            saved: None,
            scroll_top: 0,
            scroll_bottom: rows - 1,
            tabs: default_tabs(cols),
        }
    }

    pub fn scrollback_len(&self) -> usize {
        self.lines.len() - self.rows
    }

    fn idx(&self, y: usize) -> usize {
        self.lines.len() - self.rows + y
    }

    pub fn line(&self, y: usize) -> &Line {
        &self.lines[self.idx(y)]
    }

    pub fn line_mut(&mut self, y: usize) -> &mut Line {
        let i = self.idx(y);
        let line = &mut self.lines[i];
        line.dirty = true;
        line
    }

    pub fn mark_all_dirty(&mut self) {
        for line in &mut self.lines {
            line.dirty = true;
        }
    }

    fn erase_attrs(pen: &Attrs) -> Attrs {
        Attrs {
            bg: pen.bg,
            ..Attrs::default()
        }
    }

    /// 範囲 `top..=bottom` を `n` 行ぶん上へ送る。`top == 0` なら、押し出された行は
    /// スクロールバックに残る（xterm.js と同じ。下端が画面の下端でなくてもよい）。
    pub fn scroll_up(&mut self, top: usize, bottom: usize, n: usize, pen: &Attrs) {
        self.scroll_up_impl(top, bottom, n, pen, top == 0);
    }

    /// `scroll_up` と同じだが、押し出された行はスクロールバックに残さず捨てる（SU、DL）。
    pub fn scroll_up_discard(&mut self, top: usize, bottom: usize, n: usize, pen: &Attrs) {
        self.scroll_up_impl(top, bottom, n, pen, false);
    }

    fn scroll_up_impl(&mut self, top: usize, bottom: usize, n: usize, pen: &Attrs, history: bool) {
        let n = n.min(bottom - top + 1);
        let erase = Self::erase_attrs(pen);
        for _ in 0..n {
            if history && self.max_scrollback > 0 {
                let at = self.idx(bottom) + 1;
                self.lines.insert(at, Line::blank(self.cols, erase));
                while self.lines.len() > self.rows + self.max_scrollback {
                    self.lines.pop_front();
                }
            } else {
                let (t, b) = (self.idx(top), self.idx(bottom));
                self.lines.remove(t);
                self.lines.insert(b, Line::blank(self.cols, erase));
            }
        }
        for y in top..=bottom {
            self.line_mut(y);
        }
    }

    /// 範囲 `top..=bottom` を `n` 行ぶん下へ送る。
    pub fn scroll_down(&mut self, top: usize, bottom: usize, n: usize, pen: &Attrs) {
        let n = n.min(bottom - top + 1);
        let erase = Self::erase_attrs(pen);
        for _ in 0..n {
            let (t, b) = (self.idx(top), self.idx(bottom));
            self.lines.remove(b);
            self.lines.insert(t, Line::blank(self.cols, erase));
        }
        for y in top..=bottom {
            self.line_mut(y);
        }
    }

    /// 1 行下へ（LF）。範囲の下端ならスクロールする。
    pub fn index(&mut self, pen: &Attrs) {
        self.cursor.pending_wrap = false;
        if self.cursor.y == self.scroll_bottom {
            self.scroll_up(self.scroll_top, self.scroll_bottom, 1, pen);
        } else if self.cursor.y + 1 < self.rows {
            self.cursor.y += 1;
        }
    }

    /// 1 行上へ（RI）。範囲の上端ならスクロールする。
    pub fn reverse_index(&mut self, pen: &Attrs) {
        self.cursor.pending_wrap = false;
        if self.cursor.y == self.scroll_top {
            self.scroll_down(self.scroll_top, self.scroll_bottom, 1, pen);
        } else if self.cursor.y > 0 {
            self.cursor.y -= 1;
        }
    }

    fn wrap_to_next_line(&mut self, pen: &Attrs) {
        let y = self.cursor.y;
        self.line_mut(y).wrapped = true;
        self.cursor.x = 0;
        self.index(pen);
    }

    /// 結合文字をつける先のセルの x。
    fn combine_target(&self) -> Option<usize> {
        let x = if self.cursor.pending_wrap {
            self.cursor.x
        } else {
            self.cursor.x.checked_sub(1)?
        };
        let line = self.line(self.cursor.y);
        let x = if line.cells[x].is_wide_tail() {
            x.checked_sub(1)?
        } else {
            x
        };
        (!line.cells[x].is_empty()).then_some(x)
    }

    fn append_to_previous(&mut self, c: char) {
        let Some(tx) = self.combine_target() else {
            return;
        };
        let (y, cols) = (self.cursor.y, self.cols);
        let line = self.line_mut(y);
        line.cells[tx].push_char(c);
        // 絵文字の異体字選択子などで、字が 2 セルぶんになるときは右へ広げる
        let cell = &line.cells[tx];
        if cell.width == 1 && tx + 1 < cols && UnicodeWidthStr::width(cell.text().as_str()) == 2 {
            let attrs = cell.attrs;
            line.clear_overlap(tx + 1);
            line.cells[tx].width = 2;
            line.cells[tx + 1] = Cell::wide_tail(attrs);
            if !self.cursor.pending_wrap {
                if tx + 2 >= self.cols {
                    self.cursor.x = self.cols - 1;
                    self.cursor.pending_wrap = true;
                } else {
                    self.cursor.x = tx + 2;
                }
            }
        }
    }

    fn previous_ends_with_zwj(&self) -> bool {
        let Some(tx) = self.combine_target() else {
            return false;
        };
        matches!(&self.line(self.cursor.y).cells[tx].content,
            Content::Cluster(s) if s.ends_with('\u{200D}'))
    }

    /// 字を 1 つ書く。`width` は字の幅（0 は結合文字）。
    pub fn print(&mut self, c: char, width: usize, pen: &Attrs, wraparound: bool) {
        if width == 0 || self.previous_ends_with_zwj() {
            self.append_to_previous(c);
            return;
        }
        if self.cursor.pending_wrap {
            if wraparound {
                self.wrap_to_next_line(pen);
            } else {
                self.cursor.pending_wrap = false;
            }
        }
        if width == 2 && self.cursor.x + 1 >= self.cols {
            // 行末に全角の字が入りきらない（xterm.js と同じ）
            if self.cols < 2 {
                return;
            }
            if wraparound {
                let (x, y) = (self.cursor.x, self.cursor.y);
                let line = self.line_mut(y);
                line.clear_overlap(x);
                line.cells[x] = Cell::default();
                self.wrap_to_next_line(pen);
            } else {
                self.cursor.x = self.cols - 2;
            }
        }
        let (x, y) = (self.cursor.x, self.cursor.y);
        let line = self.line_mut(y);
        line.clear_overlap(x);
        if width == 2 {
            line.clear_overlap(x + 1);
        }
        line.cells[x] = Cell {
            content: Content::Char(c),
            width: width as u8,
            attrs: *pen,
        };
        if width == 2 {
            line.cells[x + 1] = Cell::wide_tail(*pen);
        }
        if x + width >= self.cols {
            self.cursor.x = self.cols - 1;
            self.cursor.pending_wrap = true;
        } else {
            self.cursor.x = x + width;
        }
    }

    /// 行 `y` の `x0..x1` のセルを消す。背景色は `pen` のものを使う。
    pub fn erase_cells(&mut self, y: usize, x0: usize, x1: usize, pen: &Attrs) {
        let x1 = x1.min(self.cols);
        if x0 >= x1 {
            return;
        }
        let erase = Self::erase_attrs(pen);
        let line = self.line_mut(y);
        line.clear_overlap(x0);
        line.clear_overlap(x1 - 1);
        for cell in &mut line.cells[x0..x1] {
            *cell = Cell::blank(erase);
        }
        if x0 == 0 && x1 == line.cells.len() {
            line.wrapped = false;
        }
    }

    /// カーソルの位置に空のセルを `n` 個入れ、右のセルを押し出す（ICH）。
    pub fn insert_cells(&mut self, n: usize, pen: &Attrs) {
        let (x, y, cols) = (self.cursor.x, self.cursor.y, self.cols);
        let n = n.min(cols - x);
        let erase = Self::erase_attrs(pen);
        self.cursor.pending_wrap = false;
        let line = self.line_mut(y);
        line.split_wide_at(x);
        line.cells.truncate(cols - n);
        for _ in 0..n {
            line.cells.insert(x, Cell::blank(erase));
        }
        line.fix_wide();
    }

    /// カーソルの位置から `n` 個のセルを消し、右のセルを詰める（DCH）。
    pub fn delete_cells(&mut self, n: usize, pen: &Attrs) {
        let (x, y, cols) = (self.cursor.x, self.cursor.y, self.cols);
        let n = n.min(cols - x);
        let erase = Self::erase_attrs(pen);
        self.cursor.pending_wrap = false;
        let line = self.line_mut(y);
        line.split_wide_at(x);
        line.cells.drain(x..x + n);
        line.cells.resize(cols, Cell::blank(erase));
        line.fix_wide();
    }

    /// 全部のセルを `c` で埋める（DECALN）。
    pub fn fill(&mut self, c: char) {
        for y in 0..self.rows {
            let line = self.line_mut(y);
            for cell in &mut line.cells {
                *cell = Cell {
                    content: Content::Char(c),
                    width: 1,
                    attrs: Attrs::default(),
                };
            }
            line.wrapped = false;
        }
    }

    /// スクロールバックを捨てる。
    pub fn clear_scrollback(&mut self) {
        let n = self.scrollback_len();
        self.lines.drain(..n);
    }

    pub fn resize(&mut self, rows: usize, cols: usize) {
        let mut shifted = 0;
        if cols != self.cols {
            for line in &mut self.lines {
                line.resize(cols);
            }
            self.cols = cols;
            // 広げた分には既定のタブ位置を足す
            let old = self.tabs.len();
            let defaults = default_tabs(cols);
            self.tabs.resize(cols, false);
            if old < cols {
                self.tabs[old..].copy_from_slice(&defaults[old..]);
            }
        }
        if rows < self.rows {
            let remove = self.rows - rows;
            // カーソルより下の行は捨て、足りなければ上の行をスクロールバックへ送る
            let below = self.rows - 1 - self.cursor.y;
            let drop_bottom = remove.min(below);
            for _ in 0..drop_bottom {
                self.lines.pop_back();
            }
            shifted = remove - drop_bottom;
            self.cursor.y -= shifted;
            self.rows = rows;
            while self.lines.len() > self.rows + self.max_scrollback {
                self.lines.pop_front();
            }
        } else if rows > self.rows {
            for _ in self.rows..rows {
                self.lines.push_back(Line::blank(cols, Attrs::default()));
            }
            self.rows = rows;
        }
        self.scroll_top = 0;
        self.scroll_bottom = rows - 1;
        self.cursor.y = self.cursor.y.min(rows - 1);
        self.cursor.x = self.cursor.x.min(cols - 1);
        self.cursor.pending_wrap = false;
        if let Some(s) = &mut self.saved {
            s.y = s.y.saturating_sub(shifted).min(rows - 1);
            s.x = s.x.min(cols - 1);
        }
        self.mark_all_dirty();
    }
}
