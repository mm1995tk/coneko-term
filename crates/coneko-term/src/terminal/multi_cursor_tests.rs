//! kitty の multiple cursors protocol の動き。

use super::*;
use crate::screen::{ExtraCursorColor, ExtraCursorShape};

fn term(rows: usize, cols: usize) -> Terminal {
    Terminal::new(rows, cols)
}

fn w(t: &mut Terminal, s: &str) {
    t.write(s.as_bytes());
}

fn out(t: &mut Terminal) -> String {
    String::from_utf8(t.take_output()).unwrap()
}

/// 追加のカーソルを `(x, y, 形)`（0 始まり）で。
fn cursors(t: &Terminal) -> Vec<(usize, usize, ExtraCursorShape)> {
    t.screen().extra_cursors.iter().collect()
}

use ExtraCursorShape::{Bar, Block, Follow, Underline};

#[test]
fn support_query() {
    let mut t = term(5, 10);
    w(&mut t, "\x1b[> q");
    assert_eq!(out(&mut t), "\x1b[>1;2;3;29;30;40;100;101 q");
}

#[test]
fn spec_example_follow_main_shape() {
    let mut t = term(10, 20);
    // y=4, x=5
    w(&mut t, "\x1b[>29;2:4:5 q");
    assert_eq!(cursors(&t), vec![(4, 3, Follow)]);
}

#[test]
fn spec_example_various_shapes_on_one_line() {
    let mut t = term(10, 20);
    w(
        &mut t,
        "\x1b[>1;2:7:1 q\x1b[>2;2:7:3 q\x1b[>3;2:7:5;2:7:7 q",
    );
    assert_eq!(
        cursors(&t),
        vec![
            (0, 6, Block),
            (2, 6, Bar),
            (4, 6, Underline),
            (6, 6, Underline)
        ]
    );
}

#[test]
fn spec_example_point_and_rectangle() {
    let mut t = term(10, 20);
    // 点 (x, y) = (4, 3) と、四角 (5, 6) から (7, 8)
    w(&mut t, "\x1b[>1;2:3:4;4:5:6:7:8 q");
    let got = cursors(&t);
    assert_eq!(got.len(), 1 + 3 * 3);
    assert!(got.contains(&(3, 2, Block)));
    assert!(got.contains(&(5, 4, Block)));
    assert!(got.contains(&(7, 6, Block)));
    assert!(!got.contains(&(4, 2, Block)));
}

#[test]
fn main_cursor_position_type_zero() {
    let mut t = term(5, 10);
    w(&mut t, "\x1b[3;4H\x1b[>1;0 q");
    assert_eq!(cursors(&t), vec![(3, 2, Block)]);
    // 主のカーソルを動かしても、置いたカーソルは動かない
    w(&mut t, "\x1b[1;1H");
    assert_eq!(cursors(&t), vec![(3, 2, Block)]);
}

#[test]
fn setting_again_replaces_the_shape() {
    let mut t = term(5, 10);
    w(&mut t, "\x1b[>1;2:2:2 q\x1b[>3;2:2:2 q");
    assert_eq!(cursors(&t), vec![(1, 1, Underline)]);
}

#[test]
fn shape_zero_clears_cells_and_rectangle_clears_all() {
    let mut t = term(5, 10);
    w(&mut t, "\x1b[>1;2:1:1;2:1:2;2:2:2 q");
    w(&mut t, "\x1b[>0;2:1:2 q");
    assert_eq!(cursors(&t), vec![(0, 0, Block), (1, 1, Block)]);
    w(&mut t, "\x1b[>0;4 q");
    assert!(cursors(&t).is_empty());
}

#[test]
fn full_screen_rectangle_sets_every_cell() {
    let mut t = term(3, 4);
    w(&mut t, "\x1b[>2;4 q");
    assert_eq!(t.screen().extra_cursors.len(), 12);
}

#[test]
fn cells_outside_the_screen_are_ignored() {
    let mut t = term(3, 4);
    w(&mut t, "\x1b[>1;2:0:1;2:1:0;2:4:1;2:1:5;2:99999999999:1 q");
    assert!(cursors(&t).is_empty());
    // 四角は、画面と重なるところだけ
    w(&mut t, "\x1b[>1;4:0:3:2:100 q");
    assert_eq!(
        cursors(&t),
        vec![(2, 0, Block), (3, 0, Block), (2, 1, Block), (3, 1, Block)]
    );
    // 画面と重ならない四角
    w(&mut t, "\x1b[>0;4 q\x1b[>1;4:10:10:20:20 q");
    assert!(cursors(&t).is_empty());
}

#[test]
fn extra_coordinates_are_ignored() {
    let mut t = term(5, 10);
    // 組の数が奇数: 最後の数を捨てる
    w(&mut t, "\x1b[>1;2:1:1:3 q");
    assert_eq!(cursors(&t), vec![(0, 0, Block)]);
    // 四角の数が 4 の倍数でない: 最後の 1 から 3 個を捨てる
    w(&mut t, "\x1b[>0;4 q\x1b[>1;4:2:2:2:2:5:5 q");
    assert_eq!(cursors(&t), vec![(1, 1, Block)]);
    w(&mut t, "\x1b[>0;4 q\x1b[>1;4:2:2:3 q");
    assert!(cursors(&t).is_empty());
}

#[test]
fn unknown_values_are_dropped() {
    let mut t = term(5, 10);
    w(
        &mut t,
        "\x1b[>7;2:1:1 q\x1b[>1;9:1:1 q\x1b[>1;2:1:1;9:2:2 q",
    );
    assert_eq!(cursors(&t), vec![(0, 0, Block)]);
    let _ = out(&mut t);
    // 未知の形は何も答えない
    w(&mut t, "\x1b[>50 q");
    assert_eq!(out(&mut t), "");
}

#[test]
fn query_cursors() {
    let mut t = term(10, 20);
    w(&mut t, "\x1b[>100 q");
    assert_eq!(out(&mut t), "\x1b[>100 q");
    w(&mut t, "\x1b[>2;2:7:3 q\x1b[>1;2:7:1 q\x1b[>29;2:4:5 q");
    w(&mut t, "\x1b[>100 q");
    assert_eq!(out(&mut t), "\x1b[>100;29:2:4:5;1:2:7:1;2:2:7:3 q");
}

#[test]
fn colors_default_set_query_and_reject() {
    let mut t = term(5, 10);
    w(&mut t, "\x1b[>101 q");
    assert_eq!(out(&mut t), "\x1b[>101;30:0;40:0 q");
    w(&mut t, "\x1b[>40;2:255:128:0 q\x1b[>30;5:7 q");
    assert_eq!(
        t.screen().extra_cursors.cursor_color,
        ExtraCursorColor::Rgb(255, 128, 0)
    );
    assert_eq!(
        t.screen().extra_cursors.text_color,
        ExtraCursorColor::Indexed(7)
    );
    w(&mut t, "\x1b[>101 q");
    assert_eq!(out(&mut t), "\x1b[>101;30:5:7;40:2:255:128:0 q");
    w(&mut t, "\x1b[>40;1 q");
    w(&mut t, "\x1b[>101 q");
    assert_eq!(out(&mut t), "\x1b[>101;30:5:7;40:1 q");
    // 範囲の外・足りない・知らない種類は何も変えない
    w(
        &mut t,
        "\x1b[>40;2:256:0:0 q\x1b[>40;2:1:2 q\x1b[>40;5:256 q\x1b[>40;9 q\x1b[>40 q",
    );
    w(&mut t, "\x1b[>101 q");
    assert_eq!(out(&mut t), "\x1b[>101;30:5:7;40:1 q");
    // 0 で主のカーソルと同じに戻す
    w(&mut t, "\x1b[>40;0 q\x1b[>30;0 q\x1b[>101 q");
    assert_eq!(out(&mut t), "\x1b[>101;30:0;40:0 q");
}

#[test]
fn main_cursor_is_untouched() {
    let mut t = term(5, 10);
    w(&mut t, "\x1b[3 q\x1b[?25l\x1b[2;3H");
    let before = t.screen().cursor();
    w(&mut t, "\x1b[>1;2:1:1 q\x1b[>40;1 q\x1b[>100 q\x1b[> q");
    assert_eq!(t.screen().cursor(), before);
    // 主のカーソルが見えなくても、置いたカーソルは残る
    assert_eq!(cursors(&t), vec![(0, 0, Block)]);
    // DECSCUSR は追加のカーソルに触れない
    w(&mut t, "\x1b[5 q");
    assert_eq!(cursors(&t), vec![(0, 0, Block)]);
}

#[test]
fn erase_display_2_and_3_and_22_remove_cursors() {
    for ed in ["2", "3", "22"] {
        let mut t = term(5, 10);
        w(&mut t, "\x1b[>1;2:1:1 q");
        w(&mut t, &format!("\x1b[{ed}J"));
        assert!(cursors(&t).is_empty(), "ED {ed}");
    }
    for ed in ["0", "1"] {
        let mut t = term(5, 10);
        w(&mut t, "\x1b[>1;2:1:1 q");
        w(&mut t, &format!("\x1b[{ed}J\x1b[2K"));
        assert_eq!(cursors(&t).len(), 1, "ED {ed}");
    }
}

#[test]
fn erase_display_keeps_the_color_pair() {
    let mut t = term(5, 10);
    w(&mut t, "\x1b[>40;1 q\x1b[>1;2:1:1 q\x1b[2J");
    assert_eq!(
        t.screen().extra_cursors.cursor_color,
        ExtraCursorColor::Special
    );
}

#[test]
fn reset_removes_cursors_and_colors() {
    let mut t = term(5, 10);
    w(&mut t, "\x1b[>1;2:1:1 q\x1b[>40;1 q\x1bc");
    assert!(cursors(&t).is_empty());
    assert_eq!(
        t.screen().extra_cursors.cursor_color,
        ExtraCursorColor::Unset
    );
    w(&mut t, "\x1b[>1;2:1:1 q");
    t.reset();
    assert!(cursors(&t).is_empty());
}

#[test]
fn switching_screens_removes_cursors() {
    let mut t = term(5, 10);
    w(&mut t, "\x1b[>1;2:1:1 q\x1b[?1049h");
    assert!(cursors(&t).is_empty());
    w(&mut t, "\x1b[>2;2:2:2 q\x1b[?1049l");
    assert!(cursors(&t).is_empty());
    // すでにその画面にいるなら、切り替えではない
    w(&mut t, "\x1b[>2;2:2:2 q\x1b[?1049l");
    assert_eq!(cursors(&t), vec![(1, 1, Bar)]);
}

#[test]
fn scrolling_does_not_move_cursors() {
    let mut t = term(3, 10);
    w(
        &mut t,
        "\x1b[>1;2:2:2 q\x1b[3;1H\n\n\x1bM\x1b[S\x1b[T\x1b[L\x1b[M",
    );
    assert_eq!(cursors(&t), vec![(1, 1, Block)]);
}

#[test]
fn resize_drops_cursors_outside_the_new_screen() {
    let mut t = term(5, 10);
    w(&mut t, "\x1b[>1;2:1:1;2:5:10;2:1:10;2:5:1 q");
    t.resize(3, 6);
    assert_eq!(cursors(&t), vec![(0, 0, Block)]);
    t.resize(8, 20);
    assert_eq!(cursors(&t), vec![(0, 0, Block)]);
}
