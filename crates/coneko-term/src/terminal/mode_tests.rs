//! DEC のモードと、問い合わせへの返事のテスト。

use super::*;

fn term(rows: usize, cols: usize) -> Terminal {
    Terminal::new(rows, cols)
}

fn w(t: &mut Terminal, s: &str) {
    t.write(s.as_bytes());
}

fn out(t: &mut Terminal) -> String {
    String::from_utf8(t.take_output()).unwrap()
}

fn text(t: &Terminal, y: usize) -> String {
    t.screen()
        .row(y)
        .cells()
        .iter()
        .map(|c| if c.is_empty() { " ".into() } else { c.text() })
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn pos(t: &Terminal) -> (usize, usize) {
    let c = t.screen().cursor();
    (c.x, c.y)
}

// ---- DECSET・DECRST ----

#[test]
fn decckm_and_decnkm() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b[?1h");
    assert!(t.modes().application_cursor_keys);
    w(&mut t, "\x1b[?1l");
    assert!(!t.modes().application_cursor_keys);
    w(&mut t, "\x1b[?66h");
    assert!(t.application_keypad());
    w(&mut t, "\x1b[?66l");
    assert!(!t.application_keypad());
    w(&mut t, "\x1b=");
    assert!(t.modes().application_keypad);
}

#[test]
fn several_modes_in_one_sequence() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b[?1;2004;1004h");
    assert!(t.modes().application_cursor_keys);
    assert!(t.modes().bracketed_paste);
    assert!(t.modes().send_focus);
    w(&mut t, "\x1b[?1;2004;1004l");
    assert_eq!(t.modes(), &Modes::default());
}

#[test]
fn origin_mode_positions_inside_scroll_region() {
    let mut t = term(10, 10);
    w(&mut t, "\x1b[3;6r");
    assert_eq!(pos(&t), (0, 0));
    w(&mut t, "\x1b[?6h");
    assert_eq!(pos(&t), (0, 2));
    w(&mut t, "\x1b[1;4H");
    assert_eq!(pos(&t), (3, 2));
    w(&mut t, "\x1b[99;1H");
    assert_eq!(pos(&t), (0, 5));
    w(&mut t, "\x1b[2d");
    assert_eq!(pos(&t), (0, 3));
    // DECSTBM でも範囲の左上へ。
    w(&mut t, "\x1b[5;8r");
    assert_eq!(pos(&t), (0, 4));
    w(&mut t, "\x1b[?6l");
    assert_eq!(pos(&t), (0, 0));
    w(&mut t, "\x1b[2;2H");
    assert_eq!(pos(&t), (1, 1));
}

#[test]
fn autowrap_mode() {
    let mut t = term(3, 4);
    w(&mut t, "\x1b[?7l");
    w(&mut t, "abcdef");
    assert_eq!(text(&t, 0), "abcf");
    assert_eq!(text(&t, 1), "");
    w(&mut t, "\x1b[?7h\r");
    w(&mut t, "abcdef");
    assert_eq!(text(&t, 1), "ef");
}

#[test]
fn cursor_blink_and_visibility() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b[?12h");
    assert!(t.screen().cursor().blinking);
    w(&mut t, "\x1b[?12l");
    assert!(!t.screen().cursor().blinking);
    w(&mut t, "\x1b[?25l");
    assert!(!t.screen().cursor().visible);
    w(&mut t, "\x1b[?25h");
    assert!(t.screen().cursor().visible);
}

#[test]
fn reverse_wraparound() {
    let mut t = term(3, 4);
    w(&mut t, "abcde\x08");
    // 45 が無いときは行頭で止まる。
    assert_eq!(pos(&t), (0, 1));
    w(&mut t, "\x08");
    assert_eq!(pos(&t), (0, 1));
    w(&mut t, "\x1b[?45h\x08");
    assert_eq!(pos(&t), (3, 0));
    // 折り返されていない行には戻らない。
    w(&mut t, "\r\n\r\n\x08");
    assert_eq!(pos(&t), (0, 2));
}

#[test]
fn alt_screen_47_and_1047() {
    for n in [47, 1047] {
        let mut t = term(3, 10);
        w(&mut t, "main");
        w(&mut t, &format!("\x1b[?{n}h"));
        assert!(t.screen().is_alt_screen());
        assert_eq!(text(&t, 0), "");
        assert_eq!(pos(&t), (4, 0), "the cursor is carried over");
        w(&mut t, "alt");
        w(&mut t, &format!("\x1b[?{n}l"));
        assert!(!t.screen().is_alt_screen());
        assert_eq!(text(&t, 0), "main");
        assert_eq!(pos(&t), (7, 0));
    }
}

#[test]
fn save_restore_cursor_1048() {
    let mut t = term(5, 10);
    w(&mut t, "\x1b[3;4H\x1b[?1048h\x1b[1;1H");
    assert_eq!(pos(&t), (0, 0));
    w(&mut t, "\x1b[?1048l");
    assert_eq!(pos(&t), (3, 2));
}

#[test]
fn alt_screen_1049_saves_and_restores_cursor() {
    let mut t = term(5, 10);
    w(&mut t, "hello\x1b[3;4H");
    w(&mut t, "\x1b[?1049h");
    assert!(t.screen().is_alt_screen());
    assert_eq!(text(&t, 0), "");
    w(&mut t, "\x1b[1;1Hvim");
    assert_eq!(text(&t, 0), "vim");
    w(&mut t, "\x1b[?1049l");
    assert!(!t.screen().is_alt_screen());
    assert_eq!(text(&t, 0), "hello");
    assert_eq!(pos(&t), (3, 2));
    // 代替画面に入り直すと空から始まる。
    w(&mut t, "\x1b[?1049h");
    assert_eq!(text(&t, 0), "");
}

#[test]
fn alt_screen_1049_twice_keeps_first_save() {
    let mut t = term(5, 10);
    w(
        &mut t,
        "\x1b[2;2H\x1b[?1049h\x1b[4;4H\x1b[?1049h\x1b[?1049l",
    );
    assert_eq!(pos(&t), (1, 1));
}

#[test]
fn mouse_protocols() {
    let mut t = term(3, 10);
    for (n, p) in [
        (9, MouseProtocol::X10),
        (1000, MouseProtocol::Vt200),
        (1002, MouseProtocol::Drag),
        (1003, MouseProtocol::Any),
    ] {
        w(&mut t, &format!("\x1b[?{n}h"));
        assert_eq!(t.modes().mouse_protocol, p);
        w(&mut t, &format!("\x1b[?{n}l"));
        assert_eq!(t.modes().mouse_protocol, MouseProtocol::None);
    }
    // あとから設定したものに替わる。
    w(&mut t, "\x1b[?1000h\x1b[?1003h");
    assert_eq!(t.modes().mouse_protocol, MouseProtocol::Any);
    assert!(out(&mut t).is_empty(), "no bytes are sent for mouse modes");
}

#[test]
fn mouse_encodings() {
    let mut t = term(3, 10);
    for (n, e) in [
        (1005, MouseEncoding::Utf8),
        (1006, MouseEncoding::Sgr),
        (1015, MouseEncoding::Urxvt),
        (1016, MouseEncoding::SgrPixels),
    ] {
        w(&mut t, &format!("\x1b[?{n}h"));
        assert_eq!(t.modes().mouse_encoding, e);
        w(&mut t, &format!("\x1b[?{n}l"));
        assert_eq!(t.modes().mouse_encoding, MouseEncoding::Default);
    }
}

#[test]
fn focus_reporting() {
    let mut t = term(3, 10);
    t.set_focus(true);
    assert!(out(&mut t).is_empty());
    w(&mut t, "\x1b[?1004h");
    t.set_focus(true);
    t.set_focus(false);
    assert_eq!(out(&mut t), "\x1b[I\x1b[O");
}

#[test]
fn bracketed_paste_and_synchronized_output() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b[?2004h\x1b[?2026h");
    assert!(t.modes().bracketed_paste);
    assert!(t.modes().synchronized_output);
    w(&mut t, "\x1b[?2004l\x1b[?2026l");
    assert!(!t.modes().bracketed_paste);
    assert!(!t.modes().synchronized_output);
}

#[test]
fn unknown_modes_are_ignored() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b[?9999h\x1b[9999h\x1b[?1h");
    assert!(t.modes().application_cursor_keys);
}

// ---- SM・RM ----

#[test]
fn insert_mode_pushes_cells_right() {
    let mut t = term(3, 8);
    w(&mut t, "abcd\r\x1b[4hXY");
    assert_eq!(text(&t, 0), "XYabcd");
    w(&mut t, "\x1b[4l\rZ");
    assert_eq!(text(&t, 0), "ZYabcd");
}

#[test]
fn insert_mode_at_line_end_does_not_lose_chars() {
    let mut t = term(3, 4);
    w(&mut t, "\x1b[4habcdef");
    assert_eq!(text(&t, 0), "abcd");
    assert_eq!(text(&t, 1), "ef");
}

#[test]
fn lnm_makes_lf_return_carriage() {
    let mut t = term(3, 10);
    w(&mut t, "ab\n");
    assert_eq!(pos(&t), (2, 1));
    w(&mut t, "\x1b[20h\n");
    assert_eq!(pos(&t), (0, 2));
    w(&mut t, "x\x0b");
    assert_eq!(pos(&t), (0, 2));
    w(&mut t, "\x1b[20l");
    assert!(!t.modes().linefeed_newline);
}

#[test]
fn ris_resets_modes() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b[?1h\x1b[?2004h\x1b[?1000h\x1b[4h\x1b[20h\x1bc");
    assert_eq!(t.modes(), &Modes::default());
    assert!(!t.screen().insert_mode);
}

// ---- 問い合わせ ----

#[test]
fn device_attributes() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b[c");
    w(&mut t, "\x1b[0c");
    assert_eq!(out(&mut t), "\x1b[?1;2c\x1b[?1;2c");
    w(&mut t, "\x1b[>c");
    assert_eq!(out(&mut t), "\x1b[>0;276;0c");
    // 0 以外の指定には返さない。
    w(&mut t, "\x1b[1c\x1b[>1c");
    assert!(out(&mut t).is_empty());
}

#[test]
fn device_status_report() {
    let mut t = term(5, 10);
    w(&mut t, "\x1b[5n");
    assert_eq!(out(&mut t), "\x1b[0n");
    w(&mut t, "\x1b[3;4H\x1b[6n");
    assert_eq!(out(&mut t), "\x1b[3;4R");
    w(&mut t, "\x1b[?6n");
    assert_eq!(out(&mut t), "\x1b[?3;4R");
    w(&mut t, "\x1b[7n");
    assert!(out(&mut t).is_empty());
}

#[test]
fn cursor_position_report_in_origin_mode_is_relative() {
    let mut t = term(10, 10);
    w(&mut t, "\x1b[3;6r\x1b[?6h\x1b[2;5H\x1b[6n");
    assert_eq!(out(&mut t), "\x1b[2;5R");
}

#[test]
fn decrqm_ansi() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b[4$p\x1b[20$p\x1b[99$p");
    assert_eq!(out(&mut t), "\x1b[4;2$y\x1b[20;2$y\x1b[99;0$y");
    w(&mut t, "\x1b[4h\x1b[20h\x1b[4$p\x1b[20$p");
    assert_eq!(out(&mut t), "\x1b[4;1$y\x1b[20;1$y");
}

#[test]
fn decrqm_dec() {
    let mut t = term(3, 10);
    w(
        &mut t,
        "\x1b[?1$p\x1b[?7$p\x1b[?25$p\x1b[?2004$p\x1b[?9999$p",
    );
    assert_eq!(
        out(&mut t),
        "\x1b[?1;2$y\x1b[?7;1$y\x1b[?25;1$y\x1b[?2004;2$y\x1b[?9999;0$y"
    );
    w(
        &mut t,
        "\x1b[?1h\x1b[?7l\x1b[?25l\x1b[?2004h\x1b[?1006h\x1b[?1003h\x1b[?1049h",
    );
    w(
        &mut t,
        "\x1b[?1$p\x1b[?7$p\x1b[?25$p\x1b[?2004$p\x1b[?1006$p\x1b[?1003$p",
    );
    w(&mut t, "\x1b[?1049$p\x1b[?1000$p");
    assert_eq!(
        out(&mut t),
        "\x1b[?1;1$y\x1b[?7;2$y\x1b[?25;2$y\x1b[?2004;1$y\x1b[?1006;1$y\x1b[?1003;1$y\
         \x1b[?1049;1$y\x1b[?1000;2$y"
    );
}

#[test]
fn osc_color_queries_use_theme() {
    let mut t = term(3, 10);
    t.set_theme(Theme {
        foreground: [0x11, 0x22, 0x33],
        background: [0xaa, 0xbb, 0xcc],
        cursor: [0x01, 0x80, 0xff],
        ..Theme::default()
    });
    w(&mut t, "\x1b]10;?\x1b\\\x1b]11;?\x1b\\\x1b]12;?\x1b\\");
    assert_eq!(
        out(&mut t),
        "\x1b]10;rgb:1111/2222/3333\x1b\\\x1b]11;rgb:aaaa/bbbb/cccc\x1b\\\
         \x1b]12;rgb:0101/8080/ffff\x1b\\"
    );
}

#[test]
fn osc_reply_keeps_terminator() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b]11;?\x07");
    assert_eq!(out(&mut t), "\x1b]11;rgb:0000/0000/0000\x07");
}

#[test]
fn osc_chained_color_queries() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b]10;?;?\x07");
    assert_eq!(
        out(&mut t),
        "\x1b]10;rgb:ffff/ffff/ffff\x07\x1b]11;rgb:0000/0000/0000\x07"
    );
}

#[test]
fn osc_4_palette_queries() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b]4;1;?;16;?;196;?;255;?\x07");
    assert_eq!(
        out(&mut t),
        "\x1b]4;1;rgb:cccc/0000/0000\x07\x1b]4;16;rgb:0000/0000/0000\x07\
         \x1b]4;196;rgb:ffff/0000/0000\x07\x1b]4;255;rgb:eeee/eeee/eeee\x07"
    );
}

#[test]
fn osc_color_set_is_ignored() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b]10;rgb:ff/00/00\x07\x1b]4;1;#ff0000\x07");
    assert!(out(&mut t).is_empty());
    assert_eq!(t.theme(), &Theme::default());
}

// ---- 出口 ----

#[test]
fn output_is_one_queue_in_order() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b[5n");
    t.queue_output(b"key");
    w(&mut t, "\x1b[c");
    assert_eq!(out(&mut t), "\x1b[0nkey\x1b[?1;2c");
    assert!(t.take_output().is_empty());
}

#[test]
fn replies_survive_split_writes() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b[");
    w(&mut t, "5");
    assert!(t.take_output().is_empty());
    w(&mut t, "n");
    assert_eq!(out(&mut t), "\x1b[0n");
}

#[test]
fn origin_mode_restore_cursor_stays_in_region() {
    let mut t = term(10, 10);
    w(&mut t, "\x1b7\x1b[3;6r\x1b[?6h\x1b8");
    assert_eq!(pos(&t), (0, 2));
    w(&mut t, "\x1b[6n");
    assert_eq!(out(&mut t), "\x1b[1;1R");
}

#[test]
fn reverse_wraparound_stops_at_region_top() {
    let mut t = term(5, 4);
    w(&mut t, "abcde\x1b[?45h\x1b[2;4r\x1b[2;1H\x08");
    assert_eq!(pos(&t), (0, 1));
}
