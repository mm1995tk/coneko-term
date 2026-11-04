//! OSC 66 を画面のモデルに当てたときの動き。

use super::*;
use crate::screen::{Align, Content};

fn term(rows: usize, cols: usize) -> Terminal {
    Terminal::new(rows, cols)
}

fn w(t: &mut Terminal, s: &str) {
    t.write(s.as_bytes());
}

fn pos(t: &Terminal) -> (usize, usize) {
    let c = t.screen().cursor();
    (c.x, c.y)
}

/// セル (x, y) が (dx, dy) 番目の、文字列 `text` の四角のセルか。
fn is_piece(t: &Terminal, x: usize, y: usize, text: &str, dx: u8, dy: u8) -> bool {
    matches!(&t.screen().row(y).cells()[x].content,
        Content::Sized { block, dx: a, dy: b } if &*block.text == text && *a == dx && *b == dy)
}

fn is_empty(t: &Terminal, x: usize, y: usize) -> bool {
    t.screen().row(y).cells()[x].is_empty()
}

#[test]
fn scale_two_heading_occupies_2x2_cells_per_character() {
    let mut t = term(5, 20);
    // 仕様の例: s=2 の見出し
    w(&mut t, "\x1b]66;s=2;Hi\x07");
    for (i, ch) in ["H", "i"].iter().enumerate() {
        for dy in 0..2u8 {
            for dx in 0..2u8 {
                assert!(is_piece(&t, i * 2 + dx as usize, dy as usize, ch, dx, dy));
            }
        }
    }
    let origin = &t.screen().row(0).cells()[0];
    assert_eq!(origin.width, 2);
    assert_eq!(origin.text(), "H");
    // 左上以外は幅 0 で、字を持たない
    assert_eq!(t.screen().row(1).cells()[0].width, 0);
    assert_eq!(t.screen().row(1).cells()[0].text(), "");
    // カーソルは同じ行で、四角の右
    assert_eq!(pos(&t), (4, 0));
    assert!(is_empty(&t, 4, 0));
    assert!(is_empty(&t, 0, 2));
}

#[test]
fn explicit_width_puts_all_text_in_one_block() {
    let mut t = term(5, 20);
    // w=3 の 3 セルに "ab" を収める（s=1）
    w(&mut t, "\x1b]66;w=3;ab\x07x");
    assert!(is_piece(&t, 0, 0, "ab", 0, 0));
    assert!(is_piece(&t, 2, 0, "ab", 2, 0));
    assert_eq!(t.screen().row(0).cells()[0].width, 3);
    assert_eq!(t.screen().row(0).cells()[3].text(), "x");
    // s=2:w=2 は 4 x 2
    let mut t = term(5, 20);
    w(&mut t, "\x1b]66;s=2:w=2;ab\x07");
    assert_eq!(pos(&t), (4, 0));
    assert!(is_piece(&t, 3, 1, "ab", 3, 1));
}

#[test]
fn wide_characters_take_twice_the_scale() {
    let mut t = term(5, 20);
    w(&mut t, "\x1b]66;s=2;あ\x07");
    assert_eq!(t.screen().row(0).cells()[0].width, 4);
    assert_eq!(pos(&t), (4, 0));
}

#[test]
fn fractional_scale_keeps_cell_count_and_records_alignment() {
    let mut t = term(5, 20);
    // 仕様の例: 半分の大きさの 2 字を 1 セルに
    w(&mut t, "\x1b]66;n=1:d=2:w=1:v=2:h=1;ab\x07");
    let c = &t.screen().row(0).cells()[0];
    let Content::Sized { block, .. } = &c.content else {
        panic!("not sized");
    };
    assert_eq!((c.width, block.scale, &*block.text), (1, 1, "ab"));
    assert_eq!((block.numerator, block.denominator), (1, 2));
    assert_eq!(
        (block.vertical, block.horizontal),
        (Align::Center, Align::End)
    );
    assert_eq!(pos(&t), (1, 0));
}

#[test]
fn plain_sequence_prints_normally() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b]66;;abc\x1b\\");
    assert_eq!(t.screen().row(0).cells()[1].text(), "b");
    assert!(!t.screen().row(0).cells()[0].is_sized());
}

#[test]
fn invalid_sequences_are_dropped() {
    let mut t = term(3, 10);
    w(
        &mut t,
        "\x1b]66;s=9;a\x07\x1b]66;s=2\x07\x1b]66;s=2;\x07\x1b]66;n=2:d=1;a\x07",
    );
    assert_eq!(pos(&t), (0, 0));
    assert!(t.screen().row(0).cells().iter().all(|c| c.is_empty()));
}

#[test]
fn does_not_fit_wraps_or_is_pushed_back() {
    let mut t = term(5, 5);
    w(&mut t, "abcd\x1b]66;s=2;X\x07");
    // 幅 2 が 1 セルしか残らない: 次の行へ
    assert!(is_empty(&t, 4, 0));
    assert!(is_piece(&t, 0, 1, "X", 0, 0));
    assert!(is_piece(&t, 1, 2, "X", 1, 1));
    // DECAWM オフなら、収まるところまで戻す
    let mut t = term(5, 5);
    w(&mut t, "\x1b[?7labcd\x1b]66;s=2;X\x07");
    assert!(is_piece(&t, 3, 0, "X", 0, 0));
}

#[test]
fn block_larger_than_screen_is_discarded() {
    let mut t = term(2, 20);
    w(&mut t, "\x1b]66;s=3;a\x07");
    assert!(t.screen().row(0).cells().iter().all(|c| c.is_empty()));
    let mut t = term(5, 4);
    w(&mut t, "\x1b]66;w=3:s=2;a\x07");
    assert!(t.screen().row(0).cells().iter().all(|c| c.is_empty()));
}

#[test]
fn scrolls_when_there_is_no_room_below() {
    let mut t = term(3, 10);
    w(&mut t, "a\r\nb\r\nc\x1b]66;s=2;X\x07");
    // 下に 2 行要るので 1 行スクロールする
    assert_eq!(t.screen().row(0).cells()[0].text(), "b");
    assert_eq!(t.screen().row(1).cells()[0].text(), "c");
    assert!(is_piece(&t, 1, 1, "X", 0, 0));
    assert!(is_piece(&t, 2, 2, "X", 1, 1));
    assert_eq!(pos(&t), (3, 1));
}

#[test]
fn overwriting_the_top_row_erases_the_whole_character() {
    // 左上・上の行の右・隣の字の上のどこに書いても、その字だけが全部消える
    for (seq, gone, kept) in [
        ("\x1b[1;1Hz", 0, 2),
        ("\x1b[1;2Hz", 0, 2),
        ("\x1b[1;4Hz", 2, 0),
    ] {
        let mut t = term(5, 20);
        w(&mut t, "\x1b]66;s=2;ab\x07");
        w(&mut t, seq);
        for y in 0..2 {
            for x in gone..gone + 2 {
                assert!(!t.screen().row(y).cells()[x].is_sized(), "{seq:?} {x},{y}");
            }
            assert!(t.screen().row(y).cells()[kept].is_sized(), "{seq:?} {y}");
        }
    }
}

#[test]
fn writing_over_lower_row_skips_the_block() {
    let mut t = term(5, 20);
    w(&mut t, "\x1b]66;s=2;a\x07");
    // 2 行目の左端に書くと、四角の幅だけ飛び越える
    w(&mut t, "\x1b[2;1Hz");
    assert!(is_piece(&t, 0, 1, "a", 0, 1));
    assert!(is_piece(&t, 1, 0, "a", 1, 0));
    assert_eq!(t.screen().row(1).cells()[2].text(), "z");
}

#[test]
fn erasing_part_of_a_character_erases_all_of_it() {
    // EL（2 行目の一部）
    let mut t = term(5, 20);
    w(&mut t, "\x1b]66;s=2;ab\x07\x1b[2;2H\x1b[K");
    assert!(t.screen().row(0).cells().iter().all(|c| c.is_empty()));
    assert!(t.screen().row(1).cells().iter().all(|c| c.is_empty()));
    // ECH で 1 セルだけ
    let mut t = term(5, 20);
    w(&mut t, "\x1b]66;s=2;ab\x07\x1b[1;4H\x1b[X");
    assert!(is_piece(&t, 0, 0, "a", 0, 0));
    assert!(is_piece(&t, 1, 1, "a", 1, 1));
    assert!(!t.screen().row(0).cells()[3].is_sized());
    assert!(!t.screen().row(1).cells()[2].is_sized());
    // ED 0（カーソルより後ろ）
    let mut t = term(5, 20);
    w(&mut t, "\x1b]66;s=2;ab\x07\x1b[2;4H\x1b[J");
    assert!(is_piece(&t, 0, 0, "a", 0, 0));
    assert!(!t.screen().row(0).cells()[2].is_sized());
    // ED 2
    w(&mut t, "\x1b[2J");
    assert!(t.screen().row(0).cells().iter().all(|c| c.is_empty()));
}

#[test]
fn dch_ich_and_line_operations_do_not_leave_halves() {
    // DCH
    let mut t = term(5, 20);
    w(&mut t, "\x1b]66;s=2;a\x07\x1b[1;1H\x1b[P");
    assert!(t.screen().row(1).cells().iter().all(|c| c.is_empty()));
    // ICH が右の四角を壊す
    let mut t = term(5, 20);
    w(&mut t, "\x1b]66;s=2;a\x07\x1b[1;1H\x1b[@");
    assert!(t.screen().row(1).cells().iter().all(|c| !c.is_sized()));
    assert!(t.screen().row(0).cells().iter().all(|c| !c.is_sized()));
    // DL で 2 行目だけ消すと、1 行目の半分も消える
    let mut t = term(5, 20);
    w(&mut t, "\x1b]66;s=2;a\x07\x1b[2;1H\x1b[M");
    assert!(t.screen().row(0).cells().iter().all(|c| !c.is_sized()));
    // IL で間に行を入れると 2 行目が離れるので消える
    let mut t = term(5, 20);
    w(&mut t, "\x1b]66;s=2;a\x07\x1b[2;1H\x1b[L");
    assert!(t.screen().row(0).cells().iter().all(|c| !c.is_sized()));
    assert!(t.screen().row(2).cells().iter().all(|c| !c.is_sized()));
}

#[test]
fn text_is_selectable_once() {
    let mut t = term(3, 20);
    w(&mut t, "\x1b]66;s=2;Hi\x07 ok");
    let s = crate::selection::text(
        t.screen(),
        crate::selection::Pos::new(0, 0),
        crate::selection::Pos::new(1, 20),
    );
    assert_eq!(s, "Hi ok\n");
}

#[test]
fn resizing_columns_returns_to_plain_text() {
    let mut t = term(5, 20);
    w(&mut t, "\x1b]66;s=2;ab\x07");
    t.resize(5, 10);
    let row0 = t.screen().row(0).cells();
    assert_eq!(row0[0].text(), "a");
    assert!(row0
        .iter()
        .chain(t.screen().row(1).cells())
        .all(|c| !c.is_sized()));
}

#[test]
fn resizing_rows_cuts_nothing_in_half() {
    let mut t = term(4, 10);
    w(&mut t, "\x1b[3;1H\x1b]66;s=2;a\x07");
    t.resize(3, 10);
    for y in 0..3 {
        assert!(
            t.screen().row(y).cells().iter().all(|c| !c.is_sized()),
            "{y}"
        );
    }
}
