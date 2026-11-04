//! kitty の multiple cursors protocol（`CSI > ... SP q`）。
//! <https://sw.kovidgoyal.net/kitty/multiple-cursors-protocol/>
//!
//! - `CSI > SP q`: 対応の問い合わせ。`CSI > 1;2;3;29;30;40;100;101 SP q` と答える。
//! - `CSI > 形 ; 座標の種類 : 座標... ; ... SP q`: 形は 0（消す）・1（ブロック）・2（縦棒）・
//!   3（下線）・29（主のカーソルに従う）。種類は 0（主のカーソルの位置）・2（`y:x` の組）・
//!   4（`上:左:下:右` の四角。数が無ければ画面全体）。座標は 1 始まり。画面の外は捨てる。
//!   組や四角に足りない数は捨てる。
//! - `CSI > 30 ; 色 SP q`・`CSI > 40 ; 色 SP q`: カーソルの下の字の色・カーソルの色。
//!   色は `0`（主のカーソルと同じ）・`1`（反転）・`2:r:g:b`・`5:番号`。
//! - `CSI > 100 SP q`: 置いたカーソルを `CSI > 100 ; 形:2:y:x ; ... SP q` と答える。
//! - `CSI > 101 SP q`: 色を `CSI > 101 ; 30:色 ; 40:色 SP q` と答える。
//!
//! 知らない値は捨てて、何も変えない。

use super::State;
use crate::parser::Params;
use crate::screen::{ExtraCursorColor, ExtraCursorShape};

fn shape_number(s: ExtraCursorShape) -> u32 {
    match s {
        ExtraCursorShape::Block => 1,
        ExtraCursorShape::Bar => 2,
        ExtraCursorShape::Underline => 3,
        _ => 29,
    }
}

/// 色を答えの書き方（`色の種類[:値...]`）にする。
fn color_text(c: ExtraCursorColor) -> String {
    match c {
        ExtraCursorColor::Special => "1".to_string(),
        ExtraCursorColor::Rgb(r, g, b) => format!("2:{r}:{g}:{b}"),
        ExtraCursorColor::Indexed(i) => format!("5:{i}"),
        _ => "0".to_string(),
    }
}

/// 色の指定（`色の種類 : 値...`）を読む。読めなければ `None`。
fn parse_color(spec: &[u32]) -> Option<ExtraCursorColor> {
    let byte = |v: u32| u8::try_from(v).ok();
    match spec {
        [0] => Some(ExtraCursorColor::Unset),
        [1] => Some(ExtraCursorColor::Special),
        [2, r, g, b] => Some(ExtraCursorColor::Rgb(byte(*r)?, byte(*g)?, byte(*b)?)),
        [5, i] => Some(ExtraCursorColor::Indexed(byte(*i)?)),
        _ => None,
    }
}

impl State {
    /// `CSI > ... SP q`。
    pub(super) fn multi_cursor(&mut self, params: &Params) {
        let Some(first) = params.get(0) else {
            self.reply("\x1b[>1;2;3;29;30;40;100;101 q");
            return;
        };
        match first[0] {
            shape @ (0 | 1 | 2 | 3 | 29) => {
                let shape = match shape {
                    0 => None,
                    1 => Some(ExtraCursorShape::Block),
                    2 => Some(ExtraCursorShape::Bar),
                    3 => Some(ExtraCursorShape::Underline),
                    _ => Some(ExtraCursorShape::Follow),
                };
                for group in params.iter().skip(1) {
                    self.set_extra_cursors(shape, group);
                }
            }
            which @ (30 | 40) => {
                if let Some(color) = params.get(1).and_then(parse_color) {
                    let cursors = &mut self.screen.extra_cursors;
                    if which == 30 {
                        cursors.text_color = color;
                    } else {
                        cursors.cursor_color = color;
                    }
                }
            }
            100 => {
                let mut out = String::from("\x1b[>100");
                for (x, y, shape) in self.screen.extra_cursors.iter() {
                    out.push_str(&format!(";{}:2:{}:{}", shape_number(shape), y + 1, x + 1));
                }
                out.push_str(" q");
                self.reply(&out);
            }
            101 => {
                let c = &self.screen.extra_cursors;
                let out = format!(
                    "\x1b[>101;30:{};40:{} q",
                    color_text(c.text_color),
                    color_text(c.cursor_color)
                );
                self.reply(&out);
            }
            _ => {}
        }
    }

    /// 座標の 1 つの組 `種類 : 座標...` に形を当てる。
    fn set_extra_cursors(&mut self, shape: Option<ExtraCursorShape>, group: &[u32]) {
        let (rows, cols) = (self.screen.rows(), self.screen.cols());
        let coords = &group[1..];
        match group[0] {
            0 => {
                let c = self.screen.cursor();
                self.screen.extra_cursors.set(c.x, c.y, shape);
            }
            2 => {
                for p in coords.chunks_exact(2) {
                    let (y, x) = (p[0] as usize, p[1] as usize);
                    if (1..=rows).contains(&y) && (1..=cols).contains(&x) {
                        self.screen.extra_cursors.set(x - 1, y - 1, shape);
                    }
                }
            }
            4 => {
                if coords.is_empty() {
                    self.set_rect(shape, 1, 1, rows, cols);
                }
                for r in coords.chunks_exact(4) {
                    let (top, left) = (r[0] as usize, r[1] as usize);
                    let (bottom, right) = (r[2] as usize, r[3] as usize);
                    self.set_rect(shape, top, left, bottom, right);
                }
            }
            _ => {}
        }
    }

    /// 1 始まりの四角（端を含む）のうち、画面と重なる範囲に形を当てる。
    fn set_rect(
        &mut self,
        shape: Option<ExtraCursorShape>,
        top: usize,
        left: usize,
        bottom: usize,
        right: usize,
    ) {
        let (rows, cols) = (self.screen.rows(), self.screen.cols());
        let (top, left) = (top.max(1), left.max(1));
        let (bottom, right) = (bottom.min(rows), right.min(cols));
        for y in top..=bottom {
            for x in left..=right {
                self.screen.extra_cursors.set(x - 1, y - 1, shape);
            }
        }
    }
}
