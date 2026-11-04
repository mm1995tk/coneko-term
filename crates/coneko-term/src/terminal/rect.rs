//! 矩形の領域への属性（kitty の「Setting text styles/colors in arbitrary regions of the screen」）。
//! DECCARA（`CSI Pt;Pl;Pb;Pr;Ps... $ r`）で属性を付け、DECRARA（`CSI Pt;Pl;Pb;Pr;Ps... $ t`）で
//! 属性を反転し、DECSACE（`CSI Ps * x`）で範囲が矩形か行の流れかを選ぶ。

use super::{arg, raw, sgr, State};
use crate::parser::Params;
use crate::screen::cell::{AttrFlags, Attrs, UnderlineStyle};

impl State {
    /// DECSACE。`2` が矩形、`0`・`1`・省略が行の流れ。ほかの値は捨てる。
    pub(super) fn decsace(&mut self, params: &Params) {
        match raw(params, 0).unwrap_or(0) {
            0 | 1 => self.rect_extent = false,
            2 => self.rect_extent = true,
            _ => {}
        }
    }

    /// DECCARA（`reverse` が偽）と DECRARA（真）。先頭の 4 つが上・左・下・右（1 から数える。
    /// 原点モードでは、スクロール範囲の左上からの位置）、残りが SGR のパラメーター。
    pub(super) fn deccara(&mut self, params: &Params, reverse: bool) {
        let rows = self.screen.rows();
        let cols = self.screen.cols();
        let (first, last) = if self.modes.origin {
            self.screen.scroll_region()
        } else {
            (0, rows - 1)
        };
        // 行は 0 始まりの画面の行。範囲より下は、範囲の最後の行に収める。
        let top = first.saturating_add(arg(params, 0, 1) - 1);
        let bottom = match raw(params, 2) {
            None | Some(0) => last,
            Some(v) => first.saturating_add(v as usize - 1).min(last),
        };
        let left = arg(params, 1, 1) - 1;
        let right = match raw(params, 3) {
            None | Some(0) => cols - 1,
            Some(v) => (v as usize - 1).min(cols - 1),
        };
        if top > bottom || left >= cols {
            return;
        }
        let rect = self.rect_extent;
        if (rect || top == bottom) && left > right {
            return;
        }
        let list: Vec<&[u32]> = params.iter().skip(4).collect();
        for y in top..=bottom {
            let (x0, x1) = if rect || top == bottom {
                (left, right + 1)
            } else {
                // 行の流れ: 最初の行は左から行末、最後の行は行頭から右まで、間は行全体。
                (
                    if y == top { left } else { 0 },
                    if y == bottom { right + 1 } else { cols },
                )
            };
            if reverse {
                self.screen
                    .update_attrs(y, x0, x1, |a| reverse_attrs(a, &list));
            } else {
                self.screen
                    .update_attrs(y, x0, x1, |a| sgr::apply_list(a, &list));
            }
        }
    }
}

/// DECRARA。`list` の属性を反転する。`0`（と省略）は、DEC の決まりの太字・下線・点滅・反転。
/// 色は反転しない。
fn reverse_attrs(attrs: &mut Attrs, list: &[&[u32]]) {
    if list.is_empty() {
        toggle_dec(attrs);
        return;
    }
    let mut i = 0;
    while i < list.len() {
        let p = list[i];
        i += 1;
        match p[0] {
            // 色は反転しない。`;` 形式の続き（`5;n`、`2;r;g;b`）は読み飛ばす。
            38 | 48 | 58 if p.len() == 1 => {
                i += match list.get(i).map(|q| q[0]) {
                    Some(5) => 2,
                    Some(2) => 4,
                    _ => 0,
                };
            }
            0 => toggle_dec(attrs),
            1 => toggle(attrs, AttrFlags::BOLD),
            2 => toggle(attrs, AttrFlags::DIM),
            3 => toggle(attrs, AttrFlags::ITALIC),
            4 => toggle_underline(attrs, p.get(1).copied()),
            5 | 6 => toggle(attrs, AttrFlags::BLINK),
            7 => toggle(attrs, AttrFlags::INVERSE),
            8 => toggle(attrs, AttrFlags::INVISIBLE),
            9 => toggle(attrs, AttrFlags::STRIKETHROUGH),
            21 => toggle_underline(attrs, Some(2)),
            _ => {}
        }
    }
}

fn toggle_dec(attrs: &mut Attrs) {
    toggle(attrs, AttrFlags::BOLD);
    toggle_underline(attrs, None);
    toggle(attrs, AttrFlags::BLINK);
    toggle(attrs, AttrFlags::INVERSE);
}

fn toggle(attrs: &mut Attrs, flag: AttrFlags) {
    if attrs.flags.contains(flag) {
        attrs.flags.remove(flag);
    } else {
        attrs.flags.insert(flag);
    }
}

/// 下線があれば消し、無ければ `style`（`4:n` の `n`。省略は 1 本線）の下線を付ける。
fn toggle_underline(attrs: &mut Attrs, style: Option<u32>) {
    if attrs.underline.style != UnderlineStyle::None {
        attrs.underline.style = UnderlineStyle::None;
        return;
    }
    attrs.underline.style = match style {
        Some(2) => UnderlineStyle::Double,
        Some(3) => UnderlineStyle::Curly,
        Some(4) => UnderlineStyle::Dotted,
        Some(5) => UnderlineStyle::Dashed,
        _ => UnderlineStyle::Single,
    };
}
