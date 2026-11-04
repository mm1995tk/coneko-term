//! SGR（`CSI ... m`）。字の属性を変える。

use crate::parser::Params;
use crate::screen::cell::{AttrFlags, Attrs, Color, UnderlineStyle};

fn palette(v: u32) -> Option<Color> {
    u8::try_from(v).ok().map(Color::Palette)
}

fn rgb(r: u32, g: u32, b: u32) -> Option<Color> {
    Some(Color::Rgb(
        u8::try_from(r).ok()?,
        u8::try_from(g).ok()?,
        u8::try_from(b).ok()?,
    ))
}

/// `38` `48` `58` の続きを読む。`p` は `38` を含むパラメーター、`rest` はその後ろのパラメーター。
/// 返すのは、色（読めなければ `None`）と、`rest` から使った個数（`;` 形式のとき）。
fn extended(p: &[u32], rest: &[&[u32]]) -> (Option<Color>, usize) {
    if p.len() > 1 {
        // `:` 形式。`38:5:n`、`38:2:r:g:b`、`38:2:色空間:r:g:b`。
        let sub = &p[1..];
        let color = match sub[0] {
            5 => sub.get(1).copied().and_then(palette),
            2 if sub.len() == 4 => rgb(sub[1], sub[2], sub[3]),
            2 if sub.len() >= 5 => rgb(sub[2], sub[3], sub[4]),
            _ => None,
        };
        return (color, 0);
    }
    // `;` 形式。`38;5;n`、`38;2;r;g;b`。
    let at = |i: usize| rest.get(i).map(|q| q[0]);
    match at(0) {
        Some(5) => (at(1).and_then(palette), 2.min(rest.len())),
        Some(2) => {
            let color = match (at(1), at(2), at(3)) {
                (Some(r), Some(g), Some(b)) => rgb(r, g, b),
                _ => None,
            };
            (color, 4.min(rest.len()))
        }
        Some(_) => (None, 1),
        None => (None, 0),
    }
}

/// SGR のパラメーターを `pen` に当てる。
pub(super) fn apply(pen: &mut Attrs, params: &Params) {
    apply_list(pen, &params.iter().collect::<Vec<_>>());
}

/// [`apply`] の本体。パラメーターを 1 つずつ（サブパラメーター込みで）並べたものを当てる。
/// 空なら `0`（全部戻す）。
pub(super) fn apply_list(pen: &mut Attrs, list: &[&[u32]]) {
    if list.is_empty() {
        *pen = Attrs::default();
        return;
    }
    let mut i = 0;
    while i < list.len() {
        let p = list[i];
        i += 1;
        let n = p[0];
        match n {
            0 => *pen = Attrs::default(),
            1 => pen.flags.insert(AttrFlags::BOLD),
            2 => pen.flags.insert(AttrFlags::DIM),
            3 => pen.flags.insert(AttrFlags::ITALIC),
            4 => {
                pen.underline.style = match p.get(1) {
                    None => UnderlineStyle::Single,
                    Some(0) => UnderlineStyle::None,
                    Some(1) => UnderlineStyle::Single,
                    Some(2) => UnderlineStyle::Double,
                    Some(3) => UnderlineStyle::Curly,
                    Some(4) => UnderlineStyle::Dotted,
                    Some(5) => UnderlineStyle::Dashed,
                    // 知らない形は 1 本線にする
                    Some(_) => UnderlineStyle::Single,
                }
            }
            5 | 6 => pen.flags.insert(AttrFlags::BLINK),
            7 => pen.flags.insert(AttrFlags::INVERSE),
            8 => pen.flags.insert(AttrFlags::INVISIBLE),
            9 => pen.flags.insert(AttrFlags::STRIKETHROUGH),
            21 => pen.underline.style = UnderlineStyle::Double,
            22 => {
                pen.flags.remove(AttrFlags::BOLD);
                pen.flags.remove(AttrFlags::DIM);
            }
            // kitty: 221 は太字だけ、222 は薄字だけを消す
            221 => pen.flags.remove(AttrFlags::BOLD),
            222 => pen.flags.remove(AttrFlags::DIM),
            23 => pen.flags.remove(AttrFlags::ITALIC),
            24 => pen.underline.style = UnderlineStyle::None,
            25 => pen.flags.remove(AttrFlags::BLINK),
            27 => pen.flags.remove(AttrFlags::INVERSE),
            28 => pen.flags.remove(AttrFlags::INVISIBLE),
            29 => pen.flags.remove(AttrFlags::STRIKETHROUGH),
            30..=37 => pen.fg = Color::Palette((n - 30) as u8),
            38 | 48 | 58 => {
                let (color, used) = extended(p, &list[i..]);
                i += used;
                if let Some(c) = color {
                    match n {
                        38 => pen.fg = c,
                        48 => pen.bg = c,
                        _ => pen.underline.color = c,
                    }
                }
            }
            39 => pen.fg = Color::Default,
            40..=47 => pen.bg = Color::Palette((n - 40) as u8),
            49 => pen.bg = Color::Default,
            59 => pen.underline.color = Color::Default,
            90..=97 => pen.fg = Color::Palette((n - 90 + 8) as u8),
            100..=107 => pen.bg = Color::Palette((n - 100 + 8) as u8),
            _ => {}
        }
    }
}
