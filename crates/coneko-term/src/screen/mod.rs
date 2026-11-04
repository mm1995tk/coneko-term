//! 画面のモデル。セル・カーソル・スクロールバック・代替画面を持つ。
//!
//! シーケンスの解釈（#4）はここに含まない。ここにあるのは、解釈した結果から呼ぶ操作。
//!
//! あとで足すもの（セルに付く画像、複数のセルにまたがる字、複数のカーソル）のために、
//! `Content` などの enum は `#[non_exhaustive]`、カーソルは `Buffer` の 1 つの値にまとめてある。
//!
//! 左右のマージン（DECSLRM）は持たない。xterm.js 6.0.0 が対応していないため。

mod buffer;
pub mod cell;
#[cfg(test)]
mod tests;

use unicode_width::UnicodeWidthChar;

use buffer::Buffer;
pub use buffer::{Line, SavedCursor};
pub use cell::{Attrs, Cell, Color, Content, Underline, UnderlineStyle};

/// スクロールバックの既定の行数。
pub const DEFAULT_SCROLLBACK: usize = 5000;

/// カーソルの見た目の形。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CursorShape {
    #[default]
    /// ブロック。
    Block,
    /// 下線。
    Underline,
    /// 縦棒。
    Bar,
}

/// カーソルの状態。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    /// 列（0 始まり）。
    pub x: usize,
    /// 行（0 始まり）。
    pub y: usize,
    /// 行末まで書いて、次の字で折り返す状態。
    pub pending_wrap: bool,
    /// 見た目の形。
    pub shape: CursorShape,
    /// 点滅するか。
    pub blinking: bool,
    /// 表示するか。
    pub visible: bool,
}

#[derive(Clone, Debug)]
/// 画面。主画面と代替画面を持ち、字の書き込み・カーソル・スクロール・タブ・大きさの変更を受ける。
pub struct Screen {
    primary: Buffer,
    alt: Buffer,
    alt_active: bool,
    /// 字を書くときに使う属性。
    pub pen: Attrs,
    /// 行末で折り返すか（DECAWM）。
    pub wraparound: bool,
    /// カーソルの形。
    pub cursor_shape: CursorShape,
    /// カーソルが点滅するか。
    pub cursor_blinking: bool,
    /// カーソルを表示するか。
    pub cursor_visible: bool,
    /// 挿入モード（IRM）。字を書くとき、右のセルを押し出す。
    pub insert_mode: bool,
}

impl Screen {
    /// `rows` 行 `cols` 列の画面。スクロールバックは既定の行数。
    pub fn new(rows: usize, cols: usize) -> Screen {
        Screen::with_scrollback(rows, cols, DEFAULT_SCROLLBACK)
    }

    /// スクロールバックの行数を指定する。行数と列数は 1 以上。
    pub fn with_scrollback(rows: usize, cols: usize, scrollback: usize) -> Screen {
        assert!(rows >= 1 && cols >= 1, "rows and cols must be at least 1");
        Screen {
            primary: Buffer::new(rows, cols, scrollback),
            alt: Buffer::new(rows, cols, 0),
            alt_active: false,
            pen: Attrs::default(),
            wraparound: true,
            cursor_shape: CursorShape::default(),
            cursor_blinking: false,
            cursor_visible: true,
            insert_mode: false,
        }
    }

    fn buf(&self) -> &Buffer {
        if self.alt_active {
            &self.alt
        } else {
            &self.primary
        }
    }

    fn buf_mut(&mut self) -> &mut Buffer {
        if self.alt_active {
            &mut self.alt
        } else {
            &mut self.primary
        }
    }

    /// 行数。
    pub fn rows(&self) -> usize {
        self.buf().rows
    }

    /// 列数。
    pub fn cols(&self) -> usize {
        self.buf().cols
    }

    // ---- 読む ----

    /// 見えている画面の `y` 行目（0 始まり）。
    pub fn row(&self, y: usize) -> &Line {
        self.buf().line(y)
    }

    /// スクロールバックの行数。代替画面では常に 0。
    pub fn scrollback_len(&self) -> usize {
        self.buf().scrollback_len()
    }

    /// スクロールバックの `i` 行目。0 が一番古い行。
    pub fn scrollback_row(&self, i: usize) -> &Line {
        assert!(i < self.scrollback_len());
        &self.buf().lines[i]
    }

    /// カーソルの状態。
    pub fn cursor(&self) -> Cursor {
        let c = self.buf().cursor;
        Cursor {
            x: c.x,
            y: c.y,
            pending_wrap: c.pending_wrap,
            shape: self.cursor_shape,
            blinking: self.cursor_blinking,
            visible: self.cursor_visible,
        }
    }

    /// 保存したカーソル（DECSC）。
    pub fn saved_cursor(&self) -> Option<SavedCursor> {
        self.buf().saved
    }

    /// スクロール範囲の上端と下端（どちらも含む、0 始まり）。
    pub fn scroll_region(&self) -> (usize, usize) {
        (self.buf().scroll_top, self.buf().scroll_bottom)
    }

    /// 代替画面に切り替わっているか。
    pub fn is_alt_screen(&self) -> bool {
        self.alt_active
    }

    /// タブ位置の列（昇順）。
    pub fn tab_stops(&self) -> Vec<usize> {
        let tabs = &self.buf().tabs;
        (0..tabs.len()).filter(|&x| tabs[x]).collect()
    }

    /// 変わった行（見えている画面の行番号）。
    pub fn dirty_rows(&self) -> Vec<usize> {
        (0..self.rows())
            .filter(|&y| self.row(y).is_dirty())
            .collect()
    }

    /// 変わった行を返し、「変わった」の印を消す。スクロールバックの行の印も消す。
    pub fn take_dirty_rows(&mut self) -> Vec<usize> {
        let rows = self.dirty_rows();
        for line in &mut self.buf_mut().lines {
            line.dirty = false;
        }
        rows
    }

    // ---- 書く ----

    /// 字を 1 つ、カーソルの位置に書く。結合文字は直前の字につく。制御文字は無視する。
    pub fn print(&mut self, c: char) {
        let Some(width) = c.width() else { return };
        // 折り返し待ちなら、字は次の行（空）に書かれるので、押し出すものは無い。
        if self.insert_mode && width > 0 && !self.cursor().pending_wrap {
            self.insert_cells(width);
        }
        let (pen, wrap) = (self.pen, self.wraparound);
        self.buf_mut().print(c, width, &pen, wrap);
    }

    /// 文字列を 1 字ずつ `print` する。
    pub fn print_str(&mut self, s: &str) {
        for c in s.chars() {
            self.print(c);
        }
    }

    /// 行 `y` の `x0..x1` を消す。背景色は `pen` のものを使う。
    pub fn erase_cells(&mut self, y: usize, x0: usize, x1: usize) {
        let pen = self.pen;
        self.buf_mut().erase_cells(y, x0, x1, &pen);
    }

    // ---- カーソル ----

    /// カーソルを動かす。画面の中に収める。
    pub fn set_cursor(&mut self, x: usize, y: usize) {
        let (rows, cols) = (self.rows(), self.cols());
        let c = &mut self.buf_mut().cursor;
        c.x = x.min(cols - 1);
        c.y = y.min(rows - 1);
        c.pending_wrap = false;
    }

    /// カーソルを左端へ（CR）。
    pub fn carriage_return(&mut self) {
        let c = &mut self.buf_mut().cursor;
        c.x = 0;
        c.pending_wrap = false;
    }

    /// カーソルを 1 つ左へ（BS）。左端では動かない。
    pub fn backspace(&mut self) {
        let c = &mut self.buf_mut().cursor;
        c.x = c.x.saturating_sub(1);
        c.pending_wrap = false;
    }

    /// LF / IND。範囲の下端ならスクロールする。
    pub fn index(&mut self) {
        let pen = self.pen;
        self.buf_mut().index(&pen);
    }

    /// RI。範囲の上端ならスクロールする。
    pub fn reverse_index(&mut self) {
        let pen = self.pen;
        self.buf_mut().reverse_index(&pen);
    }

    /// DECSC。位置・折り返し待ち・属性を保存する。
    pub fn save_cursor(&mut self) {
        let attrs = self.pen;
        let b = self.buf_mut();
        b.saved = Some(SavedCursor {
            x: b.cursor.x,
            y: b.cursor.y,
            pending_wrap: b.cursor.pending_wrap,
            attrs,
        });
    }

    /// DECRC。保存が無ければ左上と既定の属性に戻す。
    pub fn restore_cursor(&mut self) {
        let saved = self.buf().saved.unwrap_or(SavedCursor {
            x: 0,
            y: 0,
            pending_wrap: false,
            attrs: Attrs::default(),
        });
        let b = self.buf_mut();
        b.cursor.x = saved.x;
        b.cursor.y = saved.y;
        b.cursor.pending_wrap = saved.pending_wrap;
        self.pen = saved.attrs;
    }

    // ---- スクロール ----

    /// スクロール範囲を決める（どちらも含む）。不正なら何もしない。成功したらカーソルは左上へ。
    pub fn set_scroll_region(&mut self, top: usize, bottom: usize) -> bool {
        let rows = self.rows();
        if top >= bottom || bottom >= rows {
            return false;
        }
        let b = self.buf_mut();
        b.scroll_top = top;
        b.scroll_bottom = bottom;
        b.cursor.x = 0;
        b.cursor.y = 0;
        b.cursor.pending_wrap = false;
        true
    }

    /// スクロール範囲を画面全体に戻す。
    pub fn reset_scroll_region(&mut self) {
        let rows = self.rows();
        let b = self.buf_mut();
        b.scroll_top = 0;
        b.scroll_bottom = rows - 1;
    }

    /// スクロール範囲を `n` 行ぶん上へ送る。
    pub fn scroll_up(&mut self, n: usize) {
        let (pen, (t, b)) = (self.pen, self.scroll_region());
        self.buf_mut().scroll_up(t, b, n, &pen);
    }

    /// スクロール範囲を `n` 行ぶん下へ送る。
    pub fn scroll_down(&mut self, n: usize) {
        let (pen, (t, b)) = (self.pen, self.scroll_region());
        self.buf_mut().scroll_down(t, b, n, &pen);
    }

    /// スクロール範囲を `n` 行ぶん上へ送る。`scroll_up` と違い、押し出した行はスクロールバックに
    /// 残さない（SU。xterm.js と同じ）。
    pub fn scroll_up_discard(&mut self, n: usize) {
        let (pen, (t, b)) = (self.pen, self.scroll_region());
        self.buf_mut().scroll_up_discard(t, b, n, &pen);
    }

    /// カーソルの行に空の行を `n` 行入れる（IL）。カーソルがスクロール範囲の外なら何もしない。
    /// 成功したらカーソルは左端へ。
    pub fn insert_lines(&mut self, n: usize) {
        let (pen, (t, b)) = (self.pen, self.scroll_region());
        let buf = self.buf_mut();
        let y = buf.cursor.y;
        if y < t || y > b {
            return;
        }
        buf.scroll_down(y, b, n, &pen);
        buf.cursor.x = 0;
        buf.cursor.pending_wrap = false;
    }

    /// カーソルの行から `n` 行を消す（DL）。カーソルがスクロール範囲の外なら何もしない。
    /// 成功したらカーソルは左端へ。消した行はスクロールバックに残さない。
    pub fn delete_lines(&mut self, n: usize) {
        let (pen, (t, b)) = (self.pen, self.scroll_region());
        let buf = self.buf_mut();
        let y = buf.cursor.y;
        if y < t || y > b {
            return;
        }
        buf.scroll_up_discard(y, b, n, &pen);
        buf.cursor.x = 0;
        buf.cursor.pending_wrap = false;
    }

    // ---- セルの挿入と削除 ----

    /// カーソルの位置に空のセルを `n` 個入れる（ICH）。右のセルは押し出される。カーソルは動かない。
    pub fn insert_cells(&mut self, n: usize) {
        let pen = self.pen;
        self.buf_mut().insert_cells(n, &pen);
    }

    /// カーソルの位置から `n` 個のセルを消して詰める（DCH）。カーソルは動かない。
    pub fn delete_cells(&mut self, n: usize) {
        let pen = self.pen;
        self.buf_mut().delete_cells(n, &pen);
    }

    /// 全部のセルを `c` で埋め、スクロール範囲を戻してカーソルを左上へ置く（DECALN）。
    pub fn fill_with(&mut self, c: char) {
        self.reset_scroll_region();
        let b = self.buf_mut();
        b.fill(c);
        b.cursor.x = 0;
        b.cursor.y = 0;
        b.cursor.pending_wrap = false;
    }

    /// スクロールバックを捨てる（ED 3）。
    pub fn clear_scrollback(&mut self) {
        self.buf_mut().clear_scrollback();
    }

    /// 全部を初期の状態に戻す（RIS）。大きさとスクロールバックの上限は変えない。
    pub fn reset(&mut self) {
        let (rows, cols, max) = (
            self.primary.rows,
            self.primary.cols,
            self.primary.max_scrollback,
        );
        *self = Screen::with_scrollback(rows, cols, max);
    }

    // ---- タブ ----

    /// カーソルの列にタブ位置を置く（HTS）。
    pub fn set_tab_stop(&mut self) {
        let b = self.buf_mut();
        b.tabs[b.cursor.x] = true;
    }

    /// カーソルの列のタブ位置を消す。
    pub fn clear_tab_stop(&mut self) {
        let b = self.buf_mut();
        b.tabs[b.cursor.x] = false;
    }

    /// タブ位置をすべて消す。
    pub fn clear_all_tab_stops(&mut self) {
        self.buf_mut().tabs.fill(false);
    }

    /// 次のタブ位置へ（HT）。無ければ右端。
    pub fn tab(&mut self) {
        let b = self.buf_mut();
        let x = (b.cursor.x + 1..b.cols)
            .find(|&x| b.tabs[x])
            .unwrap_or(b.cols - 1);
        b.cursor.x = x;
        b.cursor.pending_wrap = false;
    }

    /// 前のタブ位置へ（CBT）。無ければ左端。
    pub fn back_tab(&mut self) {
        let b = self.buf_mut();
        let x = (0..b.cursor.x).rev().find(|&x| b.tabs[x]).unwrap_or(0);
        b.cursor.x = x;
        b.cursor.pending_wrap = false;
    }

    // ---- 代替画面 ----

    /// 代替画面へ切り替える。代替画面は空の状態から始まり、スクロールバックを持たない。
    pub fn enter_alt_screen(&mut self) {
        if self.alt_active {
            return;
        }
        let (rows, cols) = (self.primary.rows, self.primary.cols);
        self.alt = Buffer::new(rows, cols, 0);
        self.alt_active = true;
    }

    /// 主画面へ戻る。代替画面の中身は捨てる。
    pub fn leave_alt_screen(&mut self) {
        if !self.alt_active {
            return;
        }
        self.alt_active = false;
        self.primary.mark_all_dirty();
    }

    // ---- 大きさ ----

    /// 大きさを変える。折り返し直しはしない（#6）。はみ出した列は切り詰める。
    /// 行が減るときは、カーソルより下の行を捨て、足りなければ上の行をスクロールバックへ送る。
    pub fn resize(&mut self, rows: usize, cols: usize) {
        assert!(rows >= 1 && cols >= 1, "rows and cols must be at least 1");
        self.primary.resize(rows, cols);
        self.alt.resize(rows, cols);
    }
}
