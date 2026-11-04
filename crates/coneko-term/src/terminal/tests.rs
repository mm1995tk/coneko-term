use super::*;
use crate::screen::cell::{AttrFlags, Color, UnderlineStyle};
use crate::screen::{Cursor, CursorShape};

fn term(rows: usize, cols: usize) -> Terminal {
    Terminal::new(rows, cols)
}

fn w(t: &mut Terminal, s: &str) {
    t.write(s.as_bytes());
}

fn text(t: &Terminal, y: usize) -> String {
    t.screen()
        .row(y)
        .cells()
        .iter()
        .map(|c| {
            if c.is_empty() {
                " ".to_string()
            } else {
                c.text()
            }
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn lines(t: &Terminal) -> Vec<String> {
    (0..t.screen().rows()).map(|y| text(t, y)).collect()
}

fn pos(t: &Terminal) -> (usize, usize) {
    let Cursor { x, y, .. } = t.screen().cursor();
    (x, y)
}

fn pen(t: &Terminal) -> crate::screen::Attrs {
    t.screen().pen
}

// ---- C0 ----

#[test]
fn bell_becomes_event() {
    let mut t = term(3, 10);
    w(&mut t, "a\x07b");
    assert_eq!(t.take_events(), vec![Event::Bell]);
    assert!(t.take_events().is_empty());
    assert_eq!(text(&t, 0), "ab");
}

#[test]
fn bs_ht_lf_vt_ff_cr() {
    let mut t = term(5, 20);
    w(&mut t, "abc\x08X");
    assert_eq!(text(&t, 0), "abX");
    w(&mut t, "\r\t1");
    assert_eq!(text(&t, 0), "abX     1");
    // LF は列を変えない
    w(&mut t, "\x0b");
    assert_eq!(pos(&t), (9, 1));
    w(&mut t, "\x0c");
    assert_eq!(pos(&t), (9, 2));
    w(&mut t, "\n");
    assert_eq!(pos(&t), (9, 3));
    w(&mut t, "\r");
    assert_eq!(pos(&t), (0, 3));
}

#[test]
fn lf_at_bottom_scrolls() {
    let mut t = term(2, 5);
    w(&mut t, "a\r\nb\r\nc");
    assert_eq!(lines(&t), vec!["b", "c"]);
    assert_eq!(t.screen().scrollback_len(), 1);
}

#[test]
fn unknown_c0_is_ignored() {
    let mut t = term(2, 5);
    w(&mut t, "a\x00\x01\x05\x1fb");
    assert_eq!(text(&t, 0), "ab");
}

// ---- 文字集合 ----

#[test]
fn dec_line_drawing_with_designation() {
    let mut t = term(2, 10);
    w(&mut t, "\x1b(0lqk\x1b(Bq");
    assert_eq!(text(&t, 0), "┌─┐q");
}

#[test]
fn shift_out_and_in() {
    let mut t = term(2, 10);
    w(&mut t, "\x1b)0x\x0ex\x0fx");
    assert_eq!(text(&t, 0), "x│x");
}

#[test]
fn uk_charset() {
    let mut t = term(2, 10);
    w(&mut t, "\x1b(A#\x1b(B#");
    assert_eq!(text(&t, 0), "£#");
}

#[test]
fn charset_is_saved_by_decsc() {
    let mut t = term(2, 10);
    w(&mut t, "a\x1b(0\x1b7\x1b(Bq\x1b8q");
    assert_eq!(text(&t, 0), "a─");
}

// ---- ESC ----

#[test]
fn decsc_decrc_restores_position_and_attrs() {
    let mut t = term(5, 10);
    w(&mut t, "\x1b[2;3H\x1b[1;31m\x1b7\x1b[5;5H\x1b[0m\x1b8");
    assert_eq!(pos(&t), (2, 1));
    assert!(pen(&t).flags.contains(AttrFlags::BOLD));
    assert_eq!(pen(&t).fg, Color::Palette(1));
}

#[test]
fn ind_nel_ri() {
    let mut t = term(3, 10);
    w(&mut t, "ab\x1bDc");
    assert_eq!(text(&t, 1), "  c");
    w(&mut t, "\x1bEd");
    assert_eq!(text(&t, 2), "d");
    w(&mut t, "\x1bM\x1bM");
    assert_eq!(pos(&t), (1, 0));
    w(&mut t, "\x1bMX");
    // 上端での RI はスクロールダウン
    assert_eq!(lines(&t), vec![" X", "ab", "  c"]);
    assert_eq!(pos(&t), (2, 0));
}

#[test]
fn hts_sets_tab_stop() {
    let mut t = term(2, 20);
    w(&mut t, "\x1b[3g\x1b[1;5H\x1bH\r\tX");
    assert_eq!(text(&t, 0), "    X");
}

#[test]
fn ris_resets_everything() {
    let mut t = term(3, 10);
    w(
        &mut t,
        "hello\x1b[1;31m\x1b[2;3r\x1b(0\x1b[?1049h\x1b[3 q\x1bc",
    );
    assert_eq!(lines(&t), vec!["", "", ""]);
    assert_eq!(pos(&t), (0, 0));
    assert_eq!(pen(&t), crate::screen::Attrs::default());
    assert_eq!(t.screen().scroll_region(), (0, 2));
    assert_eq!(t.screen().cursor().shape, CursorShape::Block);
    w(&mut t, "q");
    assert_eq!(text(&t, 0), "q");
    assert_eq!(t.screen().tab_stops(), vec![8]);
}

#[test]
fn keypad_mode() {
    let mut t = term(2, 5);
    w(&mut t, "\x1b=");
    assert!(t.application_keypad());
    w(&mut t, "\x1b>");
    assert!(!t.application_keypad());
}

#[test]
fn decaln_fills_with_e() {
    let mut t = term(2, 3);
    w(&mut t, "\x1b[2;2r\x1b#8");
    assert_eq!(lines(&t), vec!["EEE", "EEE"]);
    assert_eq!(pos(&t), (0, 0));
    assert_eq!(t.screen().scroll_region(), (0, 1));
}

// ---- カーソル移動 ----

#[test]
fn cursor_moves() {
    let mut t = term(10, 20);
    w(&mut t, "\x1b[5;7H");
    assert_eq!(pos(&t), (6, 4));
    w(&mut t, "\x1b[2A");
    assert_eq!(pos(&t), (6, 2));
    w(&mut t, "\x1b[B");
    assert_eq!(pos(&t), (6, 3));
    w(&mut t, "\x1b[3C");
    assert_eq!(pos(&t), (9, 3));
    w(&mut t, "\x1b[2D");
    assert_eq!(pos(&t), (7, 3));
    w(&mut t, "\x1b[2E");
    assert_eq!(pos(&t), (0, 5));
    w(&mut t, "\x1b[3F");
    assert_eq!(pos(&t), (0, 2));
    w(&mut t, "\x1b[8G");
    assert_eq!(pos(&t), (7, 2));
    w(&mut t, "\x1b[5d");
    assert_eq!(pos(&t), (7, 4));
    w(&mut t, "\x1b[2e\x1b[3a");
    assert_eq!(pos(&t), (10, 6));
    w(&mut t, "\x1b[3`");
    assert_eq!(pos(&t), (2, 6));
    w(&mut t, "\x1b[2;4f");
    assert_eq!(pos(&t), (3, 1));
    w(&mut t, "\x1b[H");
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn cursor_zero_param_means_one_and_clamps() {
    let mut t = term(5, 10);
    w(&mut t, "\x1b[0;0H");
    assert_eq!(pos(&t), (0, 0));
    w(&mut t, "\x1b[0C");
    assert_eq!(pos(&t), (1, 0));
    w(&mut t, "\x1b[99;99H");
    assert_eq!(pos(&t), (9, 4));
    w(&mut t, "\x1b[99A\x1b[99D");
    assert_eq!(pos(&t), (0, 0));
    w(&mut t, "\x1b[4294967295C\x1b[4294967295B");
    assert_eq!(pos(&t), (9, 4));
}

#[test]
fn cursor_up_down_stop_at_scroll_region_margins() {
    let mut t = term(10, 10);
    w(&mut t, "\x1b[3;6r\x1b[4;1H\x1b[9A");
    assert_eq!(pos(&t), (0, 2));
    w(&mut t, "\x1b[9B");
    assert_eq!(pos(&t), (0, 5));
    // 範囲の外（下）からは、上へは上端の行まで、下へは画面の端まで
    w(&mut t, "\x1b[9;1H\x1b[9B");
    assert_eq!(pos(&t), (0, 9));
    w(&mut t, "\x1b[9A");
    assert_eq!(pos(&t), (0, 2));
}

#[test]
fn cursor_back_clears_pending_wrap() {
    let mut t = term(2, 5);
    w(&mut t, "abcde");
    assert!(t.screen().cursor().pending_wrap);
    w(&mut t, "\x1b[D");
    assert_eq!(pos(&t), (3, 0));
    assert!(!t.screen().cursor().pending_wrap);
}

// ---- 消去 ----

fn filled() -> Terminal {
    let mut t = term(3, 5);
    w(&mut t, "AAAAA\r\nBBBBB\r\nCCCCC\x1b[2;3H");
    t
}

#[test]
fn ed_modes() {
    let mut t = filled();
    w(&mut t, "\x1b[J");
    assert_eq!(lines(&t), vec!["AAAAA", "BB", ""]);
    let mut t = filled();
    w(&mut t, "\x1b[0J");
    assert_eq!(lines(&t), vec!["AAAAA", "BB", ""]);
    let mut t = filled();
    w(&mut t, "\x1b[1J");
    assert_eq!(lines(&t), vec!["", "   BB", "CCCCC"]);
    let mut t = filled();
    w(&mut t, "\x1b[2J");
    assert_eq!(lines(&t), vec!["", "", ""]);
    assert_eq!(pos(&t), (2, 1));
}

#[test]
fn ed3_clears_scrollback() {
    let mut t = term(2, 3);
    w(&mut t, "a\r\nb\r\nc\r\nd");
    assert_eq!(t.screen().scrollback_len(), 2);
    w(&mut t, "\x1b[3J");
    assert_eq!(t.screen().scrollback_len(), 0);
    assert_eq!(lines(&t), vec!["c", "d"]);
}

#[test]
fn el_modes() {
    let mut t = filled();
    w(&mut t, "\x1b[K");
    assert_eq!(text(&t, 1), "BB");
    let mut t = filled();
    w(&mut t, "\x1b[1K");
    assert_eq!(text(&t, 1), "   BB");
    let mut t = filled();
    w(&mut t, "\x1b[2K");
    assert_eq!(text(&t, 1), "");
    assert_eq!(text(&t, 0), "AAAAA");
}

#[test]
fn ech_erases_without_moving() {
    let mut t = filled();
    w(&mut t, "\x1b[2X");
    assert_eq!(text(&t, 1), "BB  B");
    assert_eq!(pos(&t), (2, 1));
    w(&mut t, "\x1b[99X");
    assert_eq!(text(&t, 1), "BB");
}

#[test]
fn erase_uses_background_color() {
    let mut t = filled();
    w(&mut t, "\x1b[41;1;32m\x1b[K");
    let cell = &t.screen().row(1).cells()[4];
    assert!(cell.is_empty());
    assert_eq!(cell.attrs.bg, Color::Palette(1));
    assert_eq!(cell.attrs.fg, Color::Default);
    assert!(!cell.attrs.flags.contains(AttrFlags::BOLD));
}

// ---- 挿入と削除 ----

#[test]
fn ich_and_dch() {
    let mut t = term(2, 8);
    w(&mut t, "abcdefgh\x1b[1;3H\x1b[2@");
    assert_eq!(text(&t, 0), "ab  cdef");
    assert_eq!(pos(&t), (2, 0));
    w(&mut t, "\x1b[3P");
    assert_eq!(text(&t, 0), "abdef");
    w(&mut t, "\x1b[99P");
    assert_eq!(text(&t, 0), "ab");
    w(&mut t, "\x1b[99@");
    assert_eq!(text(&t, 0), "ab");
}

#[test]
fn ich_dch_do_not_leave_half_wide_chars() {
    let mut t = term(2, 6);
    w(&mut t, "a\u{3042}b\u{3044}\x1b[1;3H\x1b[P");
    // 「あ」の右半分を消すと、左半分も消える
    assert_eq!(text(&t, 0), "a b\u{3044}");
    let mut t = term(2, 4);
    w(&mut t, "ab\u{3042}\x1b[1;1H\x1b[@");
    // 押し出されて右端に残った「あ」の左半分は消える
    assert_eq!(text(&t, 0), " ab");
}

#[test]
fn il_and_dl() {
    let mut t = term(4, 3);
    w(&mut t, "a\r\nb\r\nc\r\nd\x1b[2;2H\x1b[L");
    assert_eq!(lines(&t), vec!["a", "", "b", "c"]);
    assert_eq!(pos(&t), (0, 1));
    w(&mut t, "\x1b[2M");
    assert_eq!(lines(&t), vec!["a", "c", "", ""]);
    assert_eq!(t.screen().scrollback_len(), 0);
}

#[test]
fn il_dl_respect_scroll_region() {
    let mut t = term(5, 3);
    w(&mut t, "a\r\nb\r\nc\r\nd\r\ne\x1b[2;4r\x1b[2;1H\x1b[L");
    assert_eq!(lines(&t), vec!["a", "", "b", "c", "e"]);
    w(&mut t, "\x1b[M");
    assert_eq!(lines(&t), vec!["a", "b", "c", "", "e"]);
    // 範囲の外では何もしない
    w(&mut t, "\x1b[5;2H\x1b[L\x1b[M");
    assert_eq!(lines(&t), vec!["a", "b", "c", "", "e"]);
    assert_eq!(pos(&t), (1, 4));
}

#[test]
fn dl_at_top_does_not_feed_scrollback() {
    let mut t = term(3, 3);
    w(&mut t, "a\r\nb\r\nc\x1b[H\x1b[M");
    assert_eq!(lines(&t), vec!["b", "c", ""]);
    assert_eq!(t.screen().scrollback_len(), 0);
}

// ---- スクロール ----

#[test]
fn su_and_sd() {
    let mut t = term(3, 3);
    w(&mut t, "a\r\nb\r\nc\x1b[S");
    assert_eq!(lines(&t), vec!["b", "c", ""]);
    assert_eq!(t.screen().scrollback_len(), 0);
    w(&mut t, "\x1b[2T");
    assert_eq!(lines(&t), vec!["", "", "b"]);
}

#[test]
fn decstbm_limits_scrolling() {
    let mut t = term(5, 3);
    w(&mut t, "a\r\nb\r\nc\r\nd\r\ne\x1b[2;4r");
    assert_eq!(t.screen().scroll_region(), (1, 3));
    assert_eq!(pos(&t), (0, 0));
    w(&mut t, "\x1b[4;1H\n");
    assert_eq!(lines(&t), vec!["a", "c", "d", "", "e"]);
    w(&mut t, "\x1b[r");
    assert_eq!(t.screen().scroll_region(), (0, 4));
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn decstbm_invalid_is_ignored() {
    let mut t = term(5, 3);
    w(&mut t, "\x1b[3;3r");
    assert_eq!(t.screen().scroll_region(), (0, 4));
    w(&mut t, "\x1b[4;2r");
    assert_eq!(t.screen().scroll_region(), (0, 4));
    w(&mut t, "\x1b[2;99r");
    assert_eq!(t.screen().scroll_region(), (1, 4));
}

// ---- タブ ----

#[test]
fn tbc_cht_cbt() {
    let mut t = term(2, 40);
    w(&mut t, "\x1b[2I");
    assert_eq!(pos(&t), (16, 0));
    w(&mut t, "\x1b[Z");
    assert_eq!(pos(&t), (8, 0));
    w(&mut t, "\x1b[2Z");
    assert_eq!(pos(&t), (0, 0));
    w(&mut t, "\x1b[9G\x1b[g");
    assert_eq!(t.screen().tab_stops(), vec![16, 24, 32]);
    w(&mut t, "\x1b[3g");
    assert!(t.screen().tab_stops().is_empty());
    w(&mut t, "\x1b[I");
    assert_eq!(pos(&t), (39, 0));
}

// ---- REP ----

#[test]
fn rep_repeats_last_char() {
    let mut t = term(2, 10);
    w(&mut t, "ab\x1b[3b");
    assert_eq!(text(&t, 0), "abbbb");
    let mut t = term(2, 10);
    w(&mut t, "\x1b[3b");
    assert_eq!(text(&t, 0), "");
}

#[test]
fn rep_with_huge_count_is_bounded() {
    let mut t = term(2, 10);
    w(&mut t, "a\x1b[4294967295b");
    assert_eq!(text(&t, 0).len(), 10);
}

// ---- カーソルの保存 ----

#[test]
fn csi_s_and_u_save_restore() {
    let mut t = term(5, 10);
    w(&mut t, "\x1b[3;4H\x1b[s\x1b[1;1H\x1b[u");
    assert_eq!(pos(&t), (3, 2));
}

#[test]
fn csi_u_with_marker_or_params_is_not_restore() {
    let mut t = term(5, 10);
    w(&mut t, "\x1b[3;4H\x1b[s\x1b[1;1H\x1b[?u\x1b[>1u\x1b[5u");
    assert_eq!(pos(&t), (0, 0));
}

// ---- カーソルの形 ----

#[test]
fn decscusr() {
    let mut t = term(2, 5);
    let cases = [
        (0, CursorShape::Block, true),
        (1, CursorShape::Block, true),
        (2, CursorShape::Block, false),
        (3, CursorShape::Underline, true),
        (4, CursorShape::Underline, false),
        (5, CursorShape::Bar, true),
        (6, CursorShape::Bar, false),
    ];
    for (ps, shape, blinking) in cases {
        w(&mut t, &format!("\x1b[{ps} q"));
        let c = t.screen().cursor();
        assert_eq!((c.shape, c.blinking), (shape, blinking), "ps={ps}");
    }
    w(&mut t, "\x1b[ q");
    assert_eq!(t.screen().cursor().shape, CursorShape::Block);
    w(&mut t, "\x1b[4 q\x1b[9 q");
    assert_eq!(t.screen().cursor().shape, CursorShape::Underline);
}

// ---- SGR ----

#[test]
fn sgr_basic_attrs_set_and_clear() {
    let mut t = term(2, 10);
    let on = [
        (1, AttrFlags::BOLD),
        (2, AttrFlags::DIM),
        (3, AttrFlags::ITALIC),
        (5, AttrFlags::BLINK),
        (7, AttrFlags::INVERSE),
        (8, AttrFlags::INVISIBLE),
        (9, AttrFlags::STRIKETHROUGH),
    ];
    for (n, flag) in on {
        w(&mut t, &format!("\x1b[{n}m"));
        assert!(pen(&t).flags.contains(flag), "sgr {n}");
    }
    for off in [22, 23, 25, 27, 28, 29] {
        w(&mut t, &format!("\x1b[{off}m"));
    }
    assert_eq!(pen(&t).flags, AttrFlags::default());
    w(&mut t, "\x1b[1;3;4;31;42m\x1b[0m");
    assert_eq!(pen(&t), crate::screen::Attrs::default());
    w(&mut t, "\x1b[1m\x1b[m");
    assert_eq!(pen(&t), crate::screen::Attrs::default());
}

#[test]
fn sgr_22_clears_bold_and_dim() {
    let mut t = term(2, 10);
    w(&mut t, "\x1b[1;2m\x1b[22m");
    assert_eq!(pen(&t).flags, AttrFlags::default());
}

#[test]
fn sgr_underline_styles() {
    let mut t = term(2, 10);
    w(&mut t, "\x1b[4m");
    assert_eq!(pen(&t).underline.style, UnderlineStyle::Single);
    w(&mut t, "\x1b[21m");
    assert_eq!(pen(&t).underline.style, UnderlineStyle::Double);
    w(&mut t, "\x1b[24m");
    assert_eq!(pen(&t).underline.style, UnderlineStyle::None);
    let cases = [
        (0, UnderlineStyle::None),
        (1, UnderlineStyle::Single),
        (2, UnderlineStyle::Double),
        (3, UnderlineStyle::Curly),
        (4, UnderlineStyle::Dotted),
        (5, UnderlineStyle::Dashed),
    ];
    for (x, style) in cases {
        w(&mut t, &format!("\x1b[4:{x}m"));
        assert_eq!(pen(&t).underline.style, style, "4:{x}");
    }
    // 他の属性と続けても読める
    w(&mut t, "\x1b[4:3;1m");
    assert_eq!(pen(&t).underline.style, UnderlineStyle::Curly);
    assert!(pen(&t).flags.contains(AttrFlags::BOLD));
}

#[test]
fn sgr_16_colors() {
    let mut t = term(2, 10);
    w(&mut t, "\x1b[31;44m");
    assert_eq!(pen(&t).fg, Color::Palette(1));
    assert_eq!(pen(&t).bg, Color::Palette(4));
    w(&mut t, "\x1b[91;104m");
    assert_eq!(pen(&t).fg, Color::Palette(9));
    assert_eq!(pen(&t).bg, Color::Palette(12));
    w(&mut t, "\x1b[97;107m");
    assert_eq!(pen(&t).fg, Color::Palette(15));
    assert_eq!(pen(&t).bg, Color::Palette(15));
    w(&mut t, "\x1b[39;49m");
    assert_eq!(pen(&t).fg, Color::Default);
    assert_eq!(pen(&t).bg, Color::Default);
}

#[test]
fn sgr_256_and_rgb_semicolon_form() {
    let mut t = term(2, 10);
    w(&mut t, "\x1b[38;5;200;48;5;17m");
    assert_eq!(pen(&t).fg, Color::Palette(200));
    assert_eq!(pen(&t).bg, Color::Palette(17));
    w(&mut t, "\x1b[38;2;1;2;3;48;2;4;5;6m");
    assert_eq!(pen(&t).fg, Color::Rgb(1, 2, 3));
    assert_eq!(pen(&t).bg, Color::Rgb(4, 5, 6));
    // 続きのパラメーターも読める
    w(&mut t, "\x1b[38;5;1;1m");
    assert_eq!(pen(&t).fg, Color::Palette(1));
    assert!(pen(&t).flags.contains(AttrFlags::BOLD));
}

#[test]
fn sgr_256_and_rgb_colon_form() {
    let mut t = term(2, 10);
    w(&mut t, "\x1b[38:5:200m\x1b[48:5:17m");
    assert_eq!(pen(&t).fg, Color::Palette(200));
    assert_eq!(pen(&t).bg, Color::Palette(17));
    w(&mut t, "\x1b[38:2:1:2:3m\x1b[48:2:4:5:6m");
    assert_eq!(pen(&t).fg, Color::Rgb(1, 2, 3));
    assert_eq!(pen(&t).bg, Color::Rgb(4, 5, 6));
    // 色空間の欄が空
    w(&mut t, "\x1b[38:2::7:8:9m");
    assert_eq!(pen(&t).fg, Color::Rgb(7, 8, 9));
    // 色空間の欄に値
    w(&mut t, "\x1b[48:2:0:10:11:12m");
    assert_eq!(pen(&t).bg, Color::Rgb(10, 11, 12));
    // `:` 形式のあとの `;` の属性は別
    w(&mut t, "\x1b[38:5:3;1m");
    assert_eq!(pen(&t).fg, Color::Palette(3));
    assert!(pen(&t).flags.contains(AttrFlags::BOLD));
}

#[test]
fn sgr_bad_extended_colors_are_ignored() {
    let mut t = term(2, 10);
    w(&mut t, "\x1b[31m\x1b[38;5;300m");
    assert_eq!(pen(&t).fg, Color::Palette(1));
    w(&mut t, "\x1b[38;2;1;2m");
    assert_eq!(pen(&t).fg, Color::Palette(1));
    w(&mut t, "\x1b[38m\x1b[38;5m\x1b[38:7:1m");
    assert_eq!(pen(&t).fg, Color::Palette(1));
}

#[test]
fn sgr_applies_to_written_cells() {
    let mut t = term(2, 10);
    w(&mut t, "a\x1b[1;31mb\x1b[0mc");
    let cells = t.screen().row(0).cells();
    assert_eq!(cells[0].attrs, crate::screen::Attrs::default());
    assert_eq!(cells[1].attrs.fg, Color::Palette(1));
    assert!(cells[1].attrs.flags.contains(AttrFlags::BOLD));
    assert_eq!(cells[2].attrs, crate::screen::Attrs::default());
}

// ---- OSC ----

#[test]
fn osc_titles() {
    let mut t = term(2, 10);
    w(&mut t, "\x1b]2;hello\x07\x1b]1;icon\x1b\\\x1b]0;both\x07");
    assert_eq!(
        t.take_events(),
        vec![
            Event::Title("hello".into()),
            Event::IconName("icon".into()),
            Event::IconName("both".into()),
            Event::Title("both".into()),
        ]
    );
}

#[test]
fn osc_title_utf8_and_semicolons() {
    let mut t = term(2, 10);
    t.write("\x1b]2;日本語;x\x07".as_bytes());
    assert_eq!(t.take_events(), vec![Event::Title("日本語;x".into())]);
}

#[test]
fn osc_other_is_dropped() {
    let mut t = term(2, 10);
    w(
        &mut t,
        "\x1b]8;;http://example.com\x07x\x1b]8;;\x07\x1b]2\x07",
    );
    assert!(t.take_events().is_empty());
    assert_eq!(text(&t, 0), "x");
}

// ---- 知らないシーケンス ----

#[test]
fn unknown_sequences_are_dropped() {
    let mut t = term(3, 10);
    w(
        &mut t,
        "a\x1b[?25l\x1b[>c\x1b[99z\x1b[1;2;3;4;5Y\x1b%G\x1bPq...\x1b\\\x1b_Gi=1\x1b\\\x1b[=1u\x1b[0 @b",
    );
    assert_eq!(text(&t, 0), "ab");
    assert!(t.take_events().is_empty());
}

#[test]
fn garbage_does_not_panic() {
    let mut t = term(3, 4);
    let mut x: u32 = 12345;
    let mut bytes = Vec::new();
    for _ in 0..20000 {
        x = x.wrapping_mul(1103515245).wrapping_add(12345);
        bytes.push((x >> 16) as u8);
    }
    t.write(&bytes);
    // 大きな数やスクロール範囲の端
    t.write(
        b"\x1b[999999999999;999999999999H\x1b[999999999999L\x1b[1;1r\x1b[3;1r\x1b[999999999999@",
    );
    t.write(b"\x1b[999999999999P\x1b[999999999999M\x1b[999999999999S\x1b[999999999999T");
}

#[test]
fn works_with_one_cell_screen() {
    let mut t = term(1, 1);
    w(
        &mut t,
        "ab\r\n\x1b[L\x1b[M\x1b[@\x1b[P\x1b[r\x1b[1;1r\x1b[S\x1b[T\x1bM\x1bD\x1b[2J\x1b[2K",
    );
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn split_writes_continue() {
    let mut t = term(2, 10);
    w(&mut t, "a\x1b[3");
    w(&mut t, "1mb");
    assert_eq!(text(&t, 0), "ab");
    assert_eq!(pen(&t).fg, Color::Palette(1));
}

#[test]
fn dirty_rows_after_write() {
    let mut t = term(3, 5);
    t.screen_mut().take_dirty_rows();
    w(&mut t, "\x1b[2;1Hx");
    assert_eq!(t.screen_mut().take_dirty_rows(), vec![1]);
}
