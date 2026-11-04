//! 追加のカーソル（kitty の multiple cursors protocol）。
//! <https://sw.kovidgoyal.net/kitty/multiple-cursors-protocol/>
//!
//! 主のカーソル（`Buffer` のカーソル）とは別に、画面のセルごとに形を持つ。位置は画面の上の
//! 位置で、スクロールでは動かない（仕様どおり）。主画面と代替画面で 1 つの集まりを共有し、
//! 切り替えで空になる。色の組は、追加のカーソルすべてで 1 つ。

use std::collections::BTreeMap;

/// 追加のカーソルの形。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExtraCursorShape {
    /// ブロック。
    Block,
    /// 縦棒。
    Bar,
    /// 下線。
    Underline,
    /// 主のカーソルの形に従う。
    Follow,
}

/// 追加のカーソルの色（カーソルの色か、カーソルの下の字の色）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExtraCursorColor {
    /// 主のカーソルと同じ。
    #[default]
    Unset,
    /// 反転（カーソルの色がセルの前景色、字の色がセルの背景色になる）。
    Special,
    /// sRGB。
    Rgb(u8, u8, u8),
    /// パレットの番号。
    Indexed(u8),
}

/// 追加のカーソルの集まりと、その色の組。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtraCursors {
    /// `(y, x)`（0 始まり）から形へ。
    cells: BTreeMap<(usize, usize), ExtraCursorShape>,
    /// カーソルの下の字の色（`CSI > 30 ; ... q`）。
    pub text_color: ExtraCursorColor,
    /// カーソルの色（`CSI > 40 ; ... q`）。
    pub cursor_color: ExtraCursorColor,
}

impl ExtraCursors {
    /// 追加のカーソルを `(x, y, 形)` で、上の行から左の列の順に返す。
    pub fn iter(&self) -> impl Iterator<Item = (usize, usize, ExtraCursorShape)> + '_ {
        self.cells.iter().map(|(&(y, x), &s)| (x, y, s))
    }

    /// 追加のカーソルの数。
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// 追加のカーソルが 1 つも無いか。
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// `(x, y)` に形 `shape` のカーソルを置く。`None` なら消す。
    pub fn set(&mut self, x: usize, y: usize, shape: Option<ExtraCursorShape>) {
        match shape {
            Some(s) => {
                self.cells.insert((y, x), s);
            }
            None => {
                self.cells.remove(&(y, x));
            }
        }
    }

    /// 追加のカーソルをすべて消す。色の組は変えない。
    pub fn clear(&mut self) {
        self.cells.clear();
    }

    /// `rows` 行 `cols` 列の画面からはみ出したカーソルを消す。
    pub(crate) fn retain_within(&mut self, rows: usize, cols: usize) {
        self.cells.retain(|&(y, x), _| y < rows && x < cols);
    }
}
