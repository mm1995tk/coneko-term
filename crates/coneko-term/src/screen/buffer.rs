//! 1 枚分の画面（行の並びとカーソルなど）。主画面と代替画面で 1 つずつ持つ。

use std::collections::VecDeque;
use std::sync::Arc;

use unicode_width::UnicodeWidthStr;

use super::cell::{Attrs, Cell, Content, SizedBlock, MAX_SCALE};
use crate::graphics::Placement;

mod reflow;

/// 行。
#[derive(Clone, Debug)]
pub struct Line {
    pub(crate) cells: Vec<Cell>,
    pub(crate) wrapped: bool,
    pub(crate) dirty: bool,
    /// この行に左上のセルがある画像の配置。
    pub(crate) placements: Vec<Placement>,
    /// 複数のセルにまたがる字のセルがあるかもしれない（無いときは偽が確か）。
    pub(crate) sized: bool,
}

impl Line {
    pub(crate) fn blank(cols: usize, attrs: Attrs) -> Line {
        Line {
            cells: vec![Cell::blank(attrs); cols],
            wrapped: false,
            dirty: true,
            placements: Vec::new(),
            sized: false,
        }
    }

    /// 何も書かれていない行か。
    pub(crate) fn is_blank(&self) -> bool {
        self.cells.iter().all(Cell::is_empty)
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
        if self.cells[x].is_wide() && x + 1 < self.cells.len() {
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
            let orphan = if c.is_wide() {
                !(i + 1 < n && self.cells[i + 1].is_wide_tail())
            } else if c.is_wide_tail() {
                !(i > 0 && self.cells[i - 1].is_wide())
            } else {
                false
            };
            if orphan {
                self.cells[i] = Cell::default();
            }
        }
    }

    fn resize(&mut self, cols: usize) {
        self.placements.retain(|p| p.col < cols);
        self.cells.resize(cols, Cell::default());
        // 切り詰めて全角の字の右半分だけが消えたら、左半分も消す
        if let Some(last) = self.cells.last_mut() {
            if last.is_wide() {
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
    /// 列の数が変わったとき、折り返した行を組み直すか（主画面だけ）。
    pub reflow_enabled: bool,
    /// スクロールバックの上から捨てた行の数の合計。行の「通し番号」（位置 + この値）が
    /// 変わらないので、選択の位置を覚えておくのに使う。
    pub trimmed: u64,
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
            reflow_enabled: true,
            trimmed: 0,
        }
    }

    /// 代替画面のバッファ。スクロールバックを持たず、組み直しもしない。
    pub fn new_alt(rows: usize, cols: usize) -> Buffer {
        Buffer {
            reflow_enabled: false,
            ..Buffer::new(rows, cols, 0)
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

    /// 配置を足す。同じ画像の同じ（0 でない）配置の番号があれば、置き換える。
    pub fn add_placement(&mut self, y: usize, p: Placement) {
        if p.placement_id != 0 {
            self.retain_placements(|_, q| {
                !(q.image_id == p.image_id && q.placement_id == p.placement_id)
            });
        }
        self.line_mut(y).placements.push(p);
    }

    /// `keep` が偽を返した配置を捨てる。`keep` には、見えている画面の一番上を 0 とした行と配置を渡す。
    /// 捨てた配置の画像の ID を返す。
    pub fn retain_placements(&mut self, mut keep: impl FnMut(i64, &Placement) -> bool) -> Vec<u32> {
        let base = self.scrollback_len() as i64;
        let mut removed = Vec::new();
        for (i, line) in self.lines.iter_mut().enumerate() {
            if line.placements.is_empty() {
                continue;
            }
            let row = i as i64 - base;
            let before = line.placements.len();
            line.placements.retain(|p| {
                let k = keep(row, p);
                if !k {
                    removed.push(p.image_id);
                }
                k
            });
            if line.placements.len() != before {
                line.dirty = true;
            }
        }
        removed
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
                    self.trimmed += 1;
                }
                self.repair_sized_top();
            } else {
                let (t, b) = (self.idx(top), self.idx(bottom));
                self.lines.remove(t);
                self.lines.insert(b, Line::blank(self.cols, erase));
            }
        }
        for y in top..=bottom {
            self.line_mut(y);
        }
        self.repair_sized(top, bottom);
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
        self.repair_sized(top, bottom);
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
        if self.line(y).cells[tx].is_sized() {
            return;
        }
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
        self.skip_block_tails(pen);
        if width == 2 && self.cursor.x + 1 >= self.cols {
            // 行末に全角の字が入りきらない（xterm.js と同じ）
            if self.cols < 2 {
                return;
            }
            if wraparound {
                let (x, y) = (self.cursor.x, self.cursor.y);
                self.prepare_overwrite(y, x);
                self.line_mut(y).cells[x] = Cell::default();
                self.wrap_to_next_line(pen);
            } else {
                self.cursor.x = self.cols - 2;
            }
        }
        let (x, y) = (self.cursor.x, self.cursor.y);
        self.prepare_overwrite(y, x);
        if width == 2 {
            self.prepare_overwrite(y, x + 1);
        }
        let line = self.line_mut(y);
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

    // ---- 複数のセルにまたがる字（OSC 66） ----

    /// 行 `y` の `x` のセルを上書きする前の後始末。複数のセルにまたがる字なら全体を消し、
    /// 全角の字なら相方を消す。
    fn prepare_overwrite(&mut self, y: usize, x: usize) {
        if self.line(y).sized {
            self.clear_sized_at(y, x);
        }
        self.line_mut(y).clear_overlap(x);
    }

    /// 行 `y` の `x` 列から右に、複数のセルにまたがる字があれば全体を消す。
    fn clear_sized_from(&mut self, y: usize, x: usize) {
        if self.line(y).sized {
            for x in x..self.cols {
                let tall = matches!(&self.line(y).cells[x].content,
                    Content::Sized { block, .. } if block.scale > 1);
                if tall {
                    self.clear_sized_at(y, x);
                }
            }
        }
    }

    /// 行 `y` の `x` のセルが複数のセルにまたがる字の一部なら、その字を全部のセルから消す。
    fn clear_sized_at(&mut self, y: usize, x: usize) {
        let i = self.idx(y);
        self.clear_sized_abs(i, x);
    }

    /// `lines` の `i` 番目の行の `x` のセルについて `clear_sized_at` と同じことをする。
    fn clear_sized_abs(&mut self, i: usize, x: usize) {
        let Content::Sized { block, dx, dy } = &self.lines[i].cells[x].content else {
            return;
        };
        let block = Arc::clone(block);
        let oi = i as isize - isize::from(*dy);
        let ox = x as isize - isize::from(*dx);
        for r in 0..isize::from(block.scale) {
            for c in 0..isize::from(block.cols) {
                let (li, cx) = (oi + r, ox + c);
                if li < 0 || cx < 0 || li as usize >= self.lines.len() {
                    continue;
                }
                let line = &mut self.lines[li as usize];
                let Some(cell) = line.cells.get_mut(cx as usize) else {
                    continue;
                };
                if matches!(&cell.content, Content::Sized { block: b, .. } if Arc::ptr_eq(b, &block))
                {
                    *cell = Cell::default();
                    line.dirty = true;
                }
            }
        }
    }

    /// カーソルが、複数のセルにまたがる字の 2 行目より下の行の上にあるとき、その行のぶんを
    /// 飛び越える（OSC 66 の仕様。DECAWM に関わらず、行末まで飛び越えたら折り返す）。
    fn skip_block_tails(&mut self, pen: &Attrs) {
        // 1 回ごとに右へ進むか次の行へ移るので終わる。念のための上限。
        for _ in 0..self.cols * usize::from(MAX_SCALE) {
            let (x, y) = (self.cursor.x, self.cursor.y);
            if self.cursor.pending_wrap || !self.line(y).sized {
                return;
            }
            let Content::Sized { block, dx, dy } = &self.line(y).cells[x].content else {
                return;
            };
            if *dy == 0 {
                return;
            }
            let nx = x + usize::from(block.cols - dx);
            if nx < self.cols {
                self.cursor.x = nx;
            } else {
                self.cursor.x = self.cols - 1;
                self.cursor.pending_wrap = true;
                self.wrap_to_next_line(pen);
            }
        }
    }

    /// 複数のセルにまたがる字を、カーソルの位置に書く。入りきらない字は捨てる。
    /// 書いたあと、カーソルは同じ行の右へ動く。
    pub fn print_sized(&mut self, block: SizedBlock, pen: &Attrs, wraparound: bool) {
        let (cols, rows) = (usize::from(block.cols), usize::from(block.scale));
        if cols == 0 || cols > self.cols || rows > self.rows {
            return;
        }
        if self.cursor.pending_wrap {
            if wraparound {
                self.wrap_to_next_line(pen);
            } else {
                self.cursor.pending_wrap = false;
            }
        }
        self.skip_block_tails(pen);
        if self.cursor.x + cols > self.cols {
            if wraparound {
                self.wrap_to_next_line(pen);
                self.skip_block_tails(pen);
            }
            self.cursor.x = self.cursor.x.min(self.cols - cols);
        }
        // 下に収まらなければ、範囲をスクロールして場所を作る
        let y = self.cursor.y;
        let (top, bottom) = if (self.scroll_top..=self.scroll_bottom).contains(&y) {
            (self.scroll_top, self.scroll_bottom)
        } else {
            (0, self.rows - 1)
        };
        if y + rows - 1 > bottom {
            if rows > bottom - top + 1 {
                return;
            }
            let n = y + rows - 1 - bottom;
            self.scroll_up(top, bottom, n, pen);
            self.cursor.y -= n;
        }
        let (x0, y0) = (self.cursor.x, self.cursor.y);
        let block = Arc::new(block);
        for r in 0..rows {
            for c in 0..cols {
                self.prepare_overwrite(y0 + r, x0 + c);
            }
            let line = self.line_mut(y0 + r);
            line.sized = true;
            for c in 0..cols {
                line.cells[x0 + c] = Cell {
                    content: Content::Sized {
                        block: Arc::clone(&block),
                        dx: c as u8,
                        dy: r as u8,
                    },
                    width: if r == 0 && c == 0 { cols as u8 } else { 0 },
                    attrs: *pen,
                };
            }
        }
        if x0 + cols >= self.cols {
            self.cursor.x = self.cols - 1;
            self.cursor.pending_wrap = true;
        } else {
            self.cursor.x = x0 + cols;
        }
    }

    /// `top..=bottom` の行（見えている画面の行番号）とその近くで、欠けた複数のセルにまたがる字を
    /// 消す。スクロールや行の挿入・削除などで四角が切れたときに呼ぶ。
    fn repair_sized(&mut self, top: usize, bottom: usize) {
        let lo = self.idx(top).saturating_sub(usize::from(MAX_SCALE));
        let hi = (self.idx(bottom) + usize::from(MAX_SCALE)).min(self.lines.len() - 1);
        self.repair_sized_abs(lo, hi);
    }

    /// 行 `y` の、切れた 1 行の四角を消す（ICH・DCH のあと）。
    fn repair_line_sized(&mut self, y: usize) {
        let i = self.idx(y);
        self.repair_sized_abs(i, i);
    }

    /// 先頭の行を捨てて、四角の上の行だけがなくなったかもしれないとき、残りの欠片を消す。
    fn repair_sized_top(&mut self) {
        let hi = usize::from(MAX_SCALE).min(self.lines.len() - 1);
        self.repair_sized_abs(0, hi);
    }

    fn repair_sized_abs(&mut self, lo: usize, hi: usize) {
        for i in lo..=hi {
            if !self.lines[i].sized {
                continue;
            }
            let mut any = false;
            for x in 0..self.cols.min(self.lines[i].cells.len()) {
                if self.lines[i].cells[x].is_sized() {
                    if self.sized_intact(i, x) {
                        any = true;
                    } else {
                        self.clear_sized_abs(i, x);
                        // 同じ行の隣のセルも消えているかもしれない
                    }
                }
            }
            self.lines[i].sized = any;
        }
    }

    /// `lines` の `i` 番目の行の `x` のセルが、欠けていない四角の一部か。
    fn sized_intact(&self, i: usize, x: usize) -> bool {
        let Content::Sized { block, dx, dy } = &self.lines[i].cells[x].content else {
            return true;
        };
        let (oi, ox) = (i as isize - isize::from(*dy), x as isize - isize::from(*dx));
        let at = |r: isize, c: isize| -> Option<&Content> {
            let (li, cx) = (oi + r, ox + c);
            if li < 0 || cx < 0 {
                return None;
            }
            self.lines
                .get(li as usize)?
                .cells
                .get(cx as usize)
                .map(|cell| &cell.content)
        };
        let ok = |r: isize, c: isize| {
            matches!(at(r, c), Some(Content::Sized { block: b, dx, dy })
                if Arc::ptr_eq(b, block) && isize::from(*dx) == c && isize::from(*dy) == r)
        };
        if *dx == 0 && *dy == 0 {
            (0..isize::from(block.scale)).all(|r| (0..isize::from(block.cols)).all(|c| ok(r, c)))
        } else {
            ok(0, 0)
        }
    }

    /// 複数のセルにまたがる字を、ふつうの大きさの字に戻す。列の数を変えるときに、四角が組み直しで
    /// 崩れないよう先に呼ぶ。字は左上のセルに残し、ほかのセルは空にする。
    fn degrade_sized(&mut self) {
        for line in &mut self.lines {
            if !line.sized {
                continue;
            }
            line.sized = false;
            for x in 0..line.cells.len() {
                let Content::Sized { block, dx, dy } = line.cells[x].content.clone() else {
                    continue;
                };
                let attrs = line.cells[x].attrs;
                if dx != 0 || dy != 0 {
                    line.cells[x] = Cell::blank(attrs);
                    continue;
                }
                let width = UnicodeWidthStr::width(&*block.text)
                    .clamp(1, 2)
                    .min(usize::from(block.cols));
                let mut chars = block.text.chars();
                let content = match (chars.next(), chars.next()) {
                    (Some(c), None) => Content::Char(c),
                    _ => Content::Cluster(block.text.clone()),
                };
                line.cells[x] = Cell {
                    content,
                    width: width as u8,
                    attrs,
                };
                if width == 2 && x + 1 < line.cells.len() {
                    line.cells[x + 1] = Cell::wide_tail(attrs);
                }
            }
            line.dirty = true;
        }
    }

    /// 行 `y` の `x0..x1` のセルを消す。背景色は `pen` のものを使う。
    pub fn erase_cells(&mut self, y: usize, x0: usize, x1: usize, pen: &Attrs) {
        let x1 = x1.min(self.cols);
        if x0 >= x1 {
            return;
        }
        let erase = Self::erase_attrs(pen);
        if self.line(y).sized {
            for x in x0..x1 {
                self.clear_sized_at(y, x);
            }
        }
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
        self.clear_sized_from(y, x);
        let line = self.line_mut(y);
        line.split_wide_at(x);
        line.cells.truncate(cols - n);
        for _ in 0..n {
            line.cells.insert(x, Cell::blank(erase));
        }
        line.fix_wide();
        self.repair_line_sized(y);
    }

    /// カーソルの位置から `n` 個のセルを消し、右のセルを詰める（DCH）。
    pub fn delete_cells(&mut self, n: usize, pen: &Attrs) {
        let (x, y, cols) = (self.cursor.x, self.cursor.y, self.cols);
        let n = n.min(cols - x);
        let erase = Self::erase_attrs(pen);
        self.cursor.pending_wrap = false;
        self.clear_sized_from(y, x);
        let line = self.line_mut(y);
        line.split_wide_at(x);
        line.cells.drain(x..x + n);
        line.cells.resize(cols, Cell::blank(erase));
        line.fix_wide();
        self.repair_line_sized(y);
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
        self.repair_sized(0, self.rows - 1);
    }

    /// スクロールバックを捨てる。
    pub fn clear_scrollback(&mut self) {
        let n = self.scrollback_len();
        self.lines.drain(..n);
        self.trimmed += n as u64;
        self.repair_sized_top();
    }

    /// 大きさを変える。主画面は折り返した行を組み直す（`reflow`）。代替画面は組み直さず、はみ出した
    /// 列を切り詰める（xterm.js と同じ）。
    pub fn resize(&mut self, rows: usize, cols: usize) {
        self.resize_rows(rows);
        let mut reflowed = false;
        if cols != self.cols {
            self.degrade_sized();
            if self.reflow_enabled {
                self.reflow(cols);
                reflowed = true;
            } else {
                for line in &mut self.lines {
                    line.resize(cols);
                }
                self.cols = cols;
            }
            // 広げた分には既定のタブ位置を足す
            let old = self.tabs.len();
            let defaults = default_tabs(cols);
            self.tabs.resize(cols, false);
            if old < cols {
                self.tabs[old..].copy_from_slice(&defaults[old..]);
            }
        }
        self.repair_sized_abs(0, self.lines.len() - 1);
        self.scroll_top = 0;
        self.scroll_bottom = rows - 1;
        self.cursor.y = self.cursor.y.min(rows - 1);
        self.cursor.x = self.cursor.x.min(cols - 1);
        if !reflowed {
            self.cursor.pending_wrap = false;
        }
        if let Some(s) = &mut self.saved {
            s.y = s.y.min(rows - 1);
            s.x = s.x.min(cols - 1);
        }
        self.mark_all_dirty();
    }

    /// 行の数だけを変える（xterm.js 6.0.0 と同じ）。
    /// 減るとき: カーソルより下の行を捨て、足りなければ上の行をスクロールバックへ送る。
    /// 増えるとき: カーソルが最下行でスクロールバックがあれば、そこから行を引き戻す。
    /// そうでなければ下に空行を足す。
    fn resize_rows(&mut self, rows: usize) {
        if rows < self.rows {
            let remove = self.rows - rows;
            let below = self.rows - 1 - self.cursor.y;
            let drop_bottom = remove.min(below);
            for _ in 0..drop_bottom {
                self.lines.pop_back();
            }
            let shifted = remove - drop_bottom;
            self.cursor.y -= shifted;
            if let Some(s) = &mut self.saved {
                s.y = s.y.saturating_sub(shifted);
            }
            self.rows = rows;
            while self.lines.len() > self.rows + self.max_scrollback {
                self.lines.pop_front();
                self.trimmed += 1;
            }
        } else if rows > self.rows {
            let add = rows - self.rows;
            let pulled = if self.cursor.y == self.rows - 1 {
                add.min(self.scrollback_len())
            } else {
                0
            };
            for _ in pulled..add {
                self.lines
                    .push_back(Line::blank(self.cols, Attrs::default()));
            }
            self.cursor.y += pulled;
            if let Some(s) = &mut self.saved {
                s.y += pulled;
            }
            self.rows = rows;
        }
    }
}
