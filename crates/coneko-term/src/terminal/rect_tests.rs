//! DECCARA・DECRARA・DECSACE の動き。

use super::*;
use crate::screen::cell::{AttrFlags, Color, Content, UnderlineStyle};

fn term(rows: usize, cols: usize) -> Terminal {
    Terminal::new(rows, cols)
}

fn w(t: &mut Terminal, s: &str) {
    t.write(s.as_bytes());
}

/// `(x, y)`（0 始まり）のセルの属性。
fn at(t: &Terminal, x: usize, y: usize) -> crate::screen::cell::Attrs {
    t.screen().row(y).cells()[x].attrs
}

/// 背景が `Palette(n)` のセルの位置を、行ごとの文字列にする。`#` が該当、`.` が該当しない。
fn bg_map(t: &Terminal, n: u8) -> Vec<String> {
    (0..t.screen().rows())
        .map(|y| {
            (0..t.screen().cols())
                .map(|x| {
                    if at(t, x, y).bg == Color::Palette(n) {
                        '#'
                    } else {
                        '.'
                    }
                })
                .collect()
        })
        .collect()
}

#[test]
fn kitty_example_sets_background_in_a_rect() {
    // 仕様の例。(3, 4) から (10, 11) の矩形の背景を青にする。
    let mut t = term(12, 12);
    w(&mut t, "\x1b[2*x\x1b[4;3;11;10;44$r\x1b[*x");
    let map = bg_map(&t, 4);
    for (y, row) in map.iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            let inside = (2..=9).contains(&x) && (3..=10).contains(&y);
            assert_eq!(c == '#', inside, "x={x} y={y}");
        }
    }
}

#[test]
fn stream_extent_follows_the_flow_of_lines() {
    // 既定は行の流れ。2 行目の 3 列目から 4 行目の 2 列目まで。
    let mut t = term(5, 6);
    w(&mut t, "\x1b[2;3;4;2;42$r");
    assert_eq!(
        bg_map(&t, 2),
        ["......", "..####", "######", "##....", "......"]
    );
}

#[test]
fn decsace_selects_and_resets() {
    let mut t = term(4, 6);
    w(&mut t, "\x1b[2*x\x1b[1;2;3;3;41$r");
    assert_eq!(bg_map(&t, 1), [".##...", ".##...", ".##...", "......"]);
    // 0 と 1 は行の流れ、省略も同じ。
    w(&mut t, "\x1b[0m\x1b[0*x\x1b[1;2;3;3;0$r\x1b[1;2;3;3;42$r");
    assert_eq!(bg_map(&t, 2), [".#####", "######", "###...", "......"]);
    w(&mut t, "\x1b[2*x\x1b[1*x\x1b[4;1;4;2;43$r");
    assert_eq!(bg_map(&t, 3), ["......", "......", "......", "##...."]);
    // 知らない値は捨てて、今のままにする。
    w(&mut t, "\x1b[2*x\x1b[9*x\x1b[1;1;2;2;44$r");
    assert_eq!(bg_map(&t, 4), ["##....", "##....", "......", "......"]);
}

#[test]
fn defaults_cover_the_whole_screen() {
    let mut t = term(3, 4);
    w(&mut t, "\x1b[2*x\x1b[;;;;41$r");
    assert_eq!(bg_map(&t, 1), ["####", "####", "####"]);
}

#[test]
fn out_of_range_edges_are_clamped_and_inverted_rects_ignored() {
    let mut t = term(3, 4);
    w(&mut t, "\x1b[2*x\x1b[2;3;99;99;41$r");
    assert_eq!(bg_map(&t, 1), ["....", "..##", "..##"]);
    let mut t = term(3, 4);
    w(
        &mut t,
        "\x1b[2*x\x1b[3;1;2;4;41$r\x1b[1;4;3;2;41$r\x1b[1;9;3;9;41$r",
    );
    assert_eq!(bg_map(&t, 1), ["....", "....", "...."]);
}

#[test]
fn sgr_attributes_colors_and_underline() {
    let mut t = term(2, 4);
    w(
        &mut t,
        "\x1b[2*x\x1b[1;1;1;1;1;3;4:3;38;2;1;2;3;58;5;9;48:5:200$r",
    );
    let a = at(&t, 0, 0);
    assert!(a.flags.contains(AttrFlags::BOLD));
    assert!(a.flags.contains(AttrFlags::ITALIC));
    assert_eq!(a.underline.style, UnderlineStyle::Curly);
    assert_eq!(a.underline.color, Color::Palette(9));
    assert_eq!(a.fg, Color::Rgb(1, 2, 3));
    assert_eq!(a.bg, Color::Palette(200));
    assert_eq!(at(&t, 1, 0), Default::default());
    // 同じ領域の 22・24・39 で戻る。
    w(&mut t, "\x1b[1;1;1;1;22;23;24;39;49$r");
    let a = at(&t, 0, 0);
    assert_eq!(a.flags, AttrFlags::default());
    assert_eq!(a.underline.style, UnderlineStyle::None);
    assert_eq!((a.fg, a.bg), (Color::Default, Color::Default));
}

#[test]
fn empty_sgr_resets_the_region() {
    let mut t = term(2, 4);
    w(&mut t, "\x1b[41;1m");
    w(&mut t, "ab\x1b[0m\x1b[1;1;1;2$r");
    assert_eq!(at(&t, 0, 0), Default::default());
    assert_eq!(at(&t, 1, 0), Default::default());
}

#[test]
fn text_and_pen_are_untouched() {
    let mut t = term(2, 4);
    w(&mut t, "abcd\x1b[31m\x1b[1;1;2;4;44$r");
    assert_eq!(t.screen().row(0).cells()[1].content, Content::Char('b'));
    assert_eq!(t.screen().pen.fg, Color::Palette(1));
    assert_eq!(t.screen().pen.bg, Color::Default);
    assert_eq!(t.screen().cursor().x, 3);
}

#[test]
fn decrara_toggles() {
    let mut t = term(1, 4);
    w(&mut t, "\x1b[1m a\x1b[0m");
    w(&mut t, "\x1b[2*x\x1b[1;1;1;3;1;7$t");
    // 太字だった 1 つ目は太字が消え、ほかは太字になる。全部、反転が付く。
    assert!(!at(&t, 0, 0).flags.contains(AttrFlags::BOLD));
    assert!(at(&t, 2, 0).flags.contains(AttrFlags::BOLD));
    assert!(at(&t, 0, 0).flags.contains(AttrFlags::INVERSE));
    assert!(!at(&t, 3, 0).flags.contains(AttrFlags::INVERSE));
    w(&mut t, "\x1b[1;1;1;3;1;7$t");
    for x in 0..4 {
        assert_eq!(at(&t, x, 0).flags.contains(AttrFlags::BOLD), x < 2, "x={x}");
        assert!(!at(&t, x, 0).flags.contains(AttrFlags::INVERSE));
    }
}

#[test]
fn decrara_zero_and_empty_toggle_the_dec_four() {
    let mut t = term(1, 2);
    w(&mut t, "\x1b[3;31m x\x1b[0m");
    w(&mut t, "\x1b[1;1;1;1$t");
    let a = at(&t, 0, 0);
    assert!(a.flags.contains(AttrFlags::BOLD));
    assert!(a.flags.contains(AttrFlags::BLINK));
    assert!(a.flags.contains(AttrFlags::INVERSE));
    assert_eq!(a.underline.style, UnderlineStyle::Single);
    assert!(
        a.flags.contains(AttrFlags::ITALIC),
        "italic is not in the four"
    );
    assert_eq!(a.fg, Color::Palette(1), "colors are kept");
    w(&mut t, "\x1b[1;1;1;1;0$t");
    assert!(!at(&t, 0, 0).flags.contains(AttrFlags::BOLD));
    assert_eq!(at(&t, 0, 0).underline.style, UnderlineStyle::None);
}

#[test]
fn decrara_ignores_colors_and_unknown() {
    let mut t = term(1, 2);
    w(&mut t, "\x1b[1;1;1;2;31;48;5;4;99$t");
    assert_eq!(at(&t, 0, 0), Default::default());
}

#[test]
fn origin_mode_is_relative_to_the_scroll_region() {
    let mut t = term(6, 4);
    w(&mut t, "\x1b[2;5r\x1b[?6h\x1b[2*x\x1b[1;1;2;2;41$r");
    assert_eq!(
        bg_map(&t, 1),
        ["....", "##..", "##..", "....", "....", "...."]
    );
    // 領域より下は、領域の最後の行に収める。
    w(&mut t, "\x1b[0m\x1b[3;1;99;4;42$r");
    assert_eq!(
        bg_map(&t, 2)[3..],
        ["####", "####", "...."].map(String::from)
    );
    // 省略した下は領域の最後の行。
    w(&mut t, "\x1b[1;1;;1;43$r");
    assert_eq!(
        bg_map(&t, 3),
        ["....", "#...", "#...", "#...", "#...", "...."]
    );
}

#[test]
fn without_origin_mode_the_scroll_region_is_ignored() {
    let mut t = term(6, 4);
    w(&mut t, "\x1b[2;5r\x1b[2*x\x1b[1;1;6;1;41$r");
    assert_eq!(
        bg_map(&t, 1),
        ["#...", "#...", "#...", "#...", "#...", "#..."]
    );
}

#[test]
fn reset_returns_to_stream() {
    let mut t = term(2, 3);
    w(&mut t, "\x1b[2*x\x1bc\x1b[1;2;2;2;41$r");
    assert_eq!(bg_map(&t, 1), [".##", "##."]);
}

#[test]
fn rows_are_marked_dirty() {
    let mut t = term(3, 3);
    t.screen_mut().take_dirty_rows();
    w(&mut t, "\x1b[2;1;2;3;41$r");
    assert_eq!(t.screen().dirty_rows(), vec![1]);
}
