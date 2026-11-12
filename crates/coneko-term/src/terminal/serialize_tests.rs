use super::*;
use crate::screen::Buffer;

const ROWS: usize = 6;
const COLS: usize = 20;

fn term() -> Terminal {
    Terminal::new(ROWS, COLS)
}

fn w(t: &mut Terminal, s: &str) {
    t.write(s.as_bytes());
    t.take_output();
}

fn same_buffer(a: &Buffer, b: &Buffer, what: &str) {
    assert_eq!(a.lines.len(), b.lines.len(), "{what}: line count");
    for (i, (x, y)) in a.lines.iter().zip(b.lines.iter()).enumerate() {
        assert_eq!(x.cells(), y.cells(), "{what}: cells of line {i}");
        assert_eq!(
            x.is_wrapped(),
            y.is_wrapped(),
            "{what}: wrapped of line {i}"
        );
        assert_eq!(x.is_input(), y.is_input(), "{what}: input of line {i}");
    }
    assert_eq!(a.cursor, b.cursor, "{what}: cursor");
    assert_eq!(a.saved, b.saved, "{what}: saved cursor");
    assert_eq!(
        (a.scroll_top, a.scroll_bottom),
        (b.scroll_top, b.scroll_bottom),
        "{what}: scroll region"
    );
    assert_eq!(a.tabs, b.tabs, "{what}: tabs");
}

/// `a` を書き出して新しい端末に書き、同じ状態になったかを確かめる。書いた端末を返す。
fn replay(a: &Terminal, max: usize) -> Terminal {
    let bytes = a.serialize(max);
    let mut b = Terminal::new(a.screen().rows(), a.screen().cols());
    b.write(&bytes);
    assert!(
        b.take_output().is_empty(),
        "serialized bytes must not ask anything"
    );
    assert!(
        b.take_events().is_empty(),
        "serialized bytes must not raise events"
    );
    let (sa, sb) = (a.screen(), b.screen());
    let ((pa, aa), (pb, ab)) = (sa.buffers(), sb.buffers());
    same_buffer(pa, pb, "primary");
    if sa.is_alt_screen() {
        same_buffer(aa, ab, "alt");
    }
    assert_eq!(sa.is_alt_screen(), sb.is_alt_screen());
    assert_eq!(sa.cursor(), sb.cursor());
    assert_eq!(sa.pen, sb.pen);
    assert_eq!(sa.wraparound, sb.wraparound);
    assert_eq!(sa.insert_mode, sb.insert_mode);
    assert_eq!(a.modes(), b.modes());
    assert_eq!(a.state.keyboard_flags, b.state.keyboard_flags);
    assert_eq!(a.state.colors, b.state.colors);
    // 書き出し直しても同じ。
    assert_eq!(bytes, b.serialize(max));
    b
}

#[test]
fn primary_screen_with_attributes_and_scrollback() {
    let mut t = term();
    for i in 0..30 {
        w(&mut t, &format!("\x1b[3{}mline {i}\x1b[0m\r\n", i % 8));
    }
    w(
        &mut t,
        "\x1b[1;4:3;38;2;1;2;3;48;5;100;58;5;9mstyled\x1b[0m \x1b[7;9;2mx\x1b[m plain",
    );
    w(
        &mut t,
        "\r\n\u{3042}\u{3044}e\u{301} \u{1F468}\u{200D}\u{1F469}",
    );
    assert!(t.screen().scrollback_len() > 20);
    replay(&t, 5000);
}

#[test]
fn wrapped_lines_keep_their_flag() {
    let mut t = term();
    // 画面の幅を超える行（折り返す）。全角の字が右端に入りきらずに折り返す行もある。
    w(&mut t, &"a".repeat(COLS * 2 + 5));
    w(&mut t, "\r\n");
    w(&mut t, &format!("{}\u{3042}tail", "b".repeat(COLS - 1)));
    let b = replay(&t, 5000);
    assert!(b.screen().row(0).is_wrapped());
}

#[test]
fn background_only_cells_and_gaps() {
    let mut t = term();
    w(&mut t, "\x1b[44m\x1b[2K\x1b[0m");
    w(
        &mut t,
        "\r\n\x1b[41mab\x1b[0m\x1b[5Gcd\x1b[10G\x1b[42m\x1b[3X\x1b[0m",
    );
    replay(&t, 5000);
}

#[test]
fn limits_scrollback_to_the_requested_rows() {
    let mut t = term();
    for i in 0..40 {
        w(&mut t, &format!("row {i}\r\n"));
    }
    let b = replay(&t, 5000);
    assert_eq!(b.screen().scrollback_len(), t.screen().scrollback_len());

    let bytes = t.serialize(3);
    let mut c = term();
    c.write(&bytes);
    let s = c.screen();
    assert_eq!(s.scrollback_len(), 3);
    let total = t.screen().scrollback_len();
    for i in 0..3 {
        assert_eq!(
            s.scrollback_row(i).cells(),
            t.screen().scrollback_row(total - 3 + i).cells()
        );
    }
    for y in 0..ROWS {
        assert_eq!(s.row(y).cells(), t.screen().row(y).cells());
    }

    let none = t.serialize(0);
    let mut d = term();
    d.write(&none);
    assert_eq!(d.screen().scrollback_len(), 0);
    for y in 0..ROWS {
        assert_eq!(d.screen().row(y).cells(), t.screen().row(y).cells());
    }
}

#[test]
fn fewer_scrollback_rows_than_screen_rows() {
    let mut t = term();
    for i in 0..(ROWS + 2) {
        w(&mut t, &format!("r{i}\r\n"));
    }
    assert!(t.screen().scrollback_len() < ROWS);
    replay(&t, 5000);
}

#[test]
fn alternate_screen() {
    let mut t = term();
    for i in 0..15 {
        w(&mut t, &format!("primary {i}\r\n"));
    }
    w(&mut t, "\x1b[3;5H\x1b[1mprompt\x1b[0m");
    // 1049 は主画面のカーソルを保存して、代替画面へ。
    w(&mut t, "\x1b[?1049h\x1b[2J\x1b[H");
    w(
        &mut t,
        "\x1b[44malt top\x1b[0m\x1b[4;3H\x1b[4:3munder\x1b[0m\x1b[?25l",
    );
    w(&mut t, "\x1b[2;4r\x1b[6;10H");
    let b = replay(&t, 5000);
    assert!(b.screen().is_alt_screen());
    // 主画面へ戻ると、保存したカーソルと主画面の中身が元と同じ。
    let mut a = t;
    let mut b = b;
    w(&mut a, "\x1b[?1049l");
    w(&mut b, "\x1b[?1049l");
    same_buffer(
        a.screen().buffers().0,
        b.screen().buffers().0,
        "primary after leaving",
    );
    assert_eq!(a.screen().cursor(), b.screen().cursor());
}

#[test]
fn saved_cursor_and_pen() {
    let mut t = term();
    w(&mut t, "\x1b[2;3H\x1b[31;1m\x1b7\x1b[5;7H\x1b[32mtext");
    let mut b = replay(&t, 5000);
    let mut a = t;
    w(&mut a, "\x1b8");
    w(&mut b, "\x1b8");
    assert_eq!(a.screen().cursor(), b.screen().cursor());
    assert_eq!(a.screen().pen, b.screen().pen);
}

#[test]
fn cursor_waiting_to_wrap() {
    let mut t = term();
    w(&mut t, &format!("\x1b[3;1H\x1b[33m{}", "z".repeat(COLS)));
    assert!(t.screen().cursor().pending_wrap);
    let mut b = replay(&t, 5000);
    let mut a = t;
    // 次の字は同じように折り返す。
    w(&mut a, "n");
    w(&mut b, "n");
    same_buffer(
        a.screen().buffers().0,
        b.screen().buffers().0,
        "after next char",
    );
}

#[test]
fn cursor_shape_and_visibility() {
    for style in 1..=6 {
        let mut t = term();
        w(&mut t, &format!("hi\x1b[{style} q"));
        replay(&t, 5000);
    }
    let mut t = term();
    w(&mut t, "hi\x1b[?25l\x1b[?7l\x1b[4h\x1b[20h");
    let b = replay(&t, 5000);
    assert!(!b.screen().cursor_visible);
}

#[test]
fn kitty_keyboard_flags_with_pushed_levels() {
    let mut t = term();
    w(&mut t, "\x1b[>1u\x1b[>5u\x1b[>0u\x1b[>9u");
    w(&mut t, "\x1b[?1049h\x1b[>3u\x1b[>16u");
    let b = replay(&t, 5000);
    assert_eq!(b.keyboard_flags(), 16);
    assert_eq!(b.state.keyboard_flags[0].entries(), [1, 5, 0, 9]);
    assert_eq!(b.state.keyboard_flags[1].entries(), [3, 16]);
    // 降ろすと、積んだ段が出てくる。
    let mut b = b;
    w(&mut b, "\x1b[<1u");
    assert_eq!(b.keyboard_flags(), 3);
    w(&mut b, "\x1b[?1049l");
    assert_eq!(b.keyboard_flags(), 9);
    w(&mut b, "\x1b[<1u");
    assert_eq!(b.keyboard_flags(), 0);
}

#[test]
fn mouse_and_other_dec_modes() {
    let cases = [
        "\x1b[?9h",
        "\x1b[?1000h\x1b[?1005h",
        "\x1b[?1002h\x1b[?1006h",
        "\x1b[?1003h\x1b[?1015h",
        "\x1b[?1000h\x1b[?1016h",
        "\x1b[?2004h\x1b[?1004h\x1b[?1h\x1b[?45h\x1b[?66h\x1b[?5522h",
        "\x1b[?2026h",
    ];
    for c in cases {
        let mut t = term();
        w(&mut t, c);
        let b = replay(&t, 5000);
        assert_eq!(b.modes(), t.modes(), "{c:?}");
    }
    let mut t = term();
    w(&mut t, "\x1b[?1002h\x1b[?1006h\x1b[?2004h");
    let b = replay(&t, 5000);
    assert_eq!(b.modes().mouse_protocol, MouseProtocol::Drag);
    assert_eq!(b.modes().mouse_encoding, MouseEncoding::Sgr);
    assert!(b.modes().bracketed_paste);
}

#[test]
fn origin_mode_and_scroll_region() {
    let mut t = term();
    w(&mut t, "\x1b[2;5r\x1b[?6h\x1b[2;3Hxyz");
    replay(&t, 5000);
}

#[test]
fn tab_stops() {
    let mut t = term();
    w(&mut t, "\x1b[3g\x1b[4G\x1bH\x1b[11G\x1bH");
    let b = replay(&t, 5000);
    assert_eq!(b.screen().tab_stops(), vec![3, 10]);
}

#[test]
fn changed_colors() {
    let mut t = term();
    w(
        &mut t,
        "\x1b]4;1;rgb:12/34/56\x07\x1b]10;#102030\x07\x1b]11;#405060\x07\x1b]12;#708090\x07",
    );
    replay(&t, 5000);
}

#[test]
fn text_sizing() {
    let mut t = term();
    w(
        &mut t,
        "\x1b[2;3H\x1b]66;s=2;ab\x07 \x1b]66;n=1:d=3:v=1:h=2;x\x07end",
    );
    for y in 0..3 {
        w(&mut t, &format!("\x1b[{};1Hmark", y + 5));
    }
    replay(&t, 5000);
}

#[test]
fn no_queries_in_the_bytes() {
    let mut t = term();
    // 昔の問い合わせが入った出力を書いたあとの状態。
    t.write(b"\x1b[?2026$p\x1b[?2027$p\x1b[?u\x1b[c\x1b[>c\x1b[6n\x1b]11;?\x07\x1b[#R\x1b[?7501$p");
    assert!(!t.take_output().is_empty());
    w(&mut t, "prompt$ ");
    let bytes = t.serialize(5000);
    let mut b = term();
    b.write(&bytes);
    assert!(b.take_output().is_empty());
    // 書き出した結果は、問い合わせを含む末尾の文字列を持たない。
    let s = String::from_utf8_lossy(&bytes);
    assert!(!s.contains("$p"));
    assert!(!s.contains("?u"));
    assert!(!s.contains("6n"));
}

#[test]
fn replayed_terminal_keeps_working() {
    let mut t = term();
    for i in 0..10 {
        w(&mut t, &format!("l{i}\r\n"));
    }
    let mut b = replay(&t, 5000);
    let mut a = t;
    w(&mut a, "more\r\ntext");
    w(&mut b, "more\r\ntext");
    same_buffer(
        a.screen().buffers().0,
        b.screen().buffers().0,
        "after more writes",
    );
}

#[test]
fn scrollback_wrapped_line_followed_by_tall_text() {
    let mut t = term();
    w(
        &mut t,
        &format!("{}\x1b]66;s=2;X\x07y\r\n", "a".repeat(COLS)),
    );
    for i in 0..ROWS + 1 {
        w(&mut t, &format!("r{i}\r\n"));
    }
    // 高い字は scrollback には書かないが、前の行は壊れず、行の数も変わらない。
    let mut b = term();
    b.write(&t.serialize(5000));
    let (sa, sb) = (t.screen(), b.screen());
    assert_eq!(sa.scrollback_len(), sb.scrollback_len());
    assert_eq!(sa.scrollback_row(0).cells(), sb.scrollback_row(0).cells());
    for y in 0..ROWS {
        assert_eq!(sa.row(y).cells(), sb.row(y).cells());
    }
}

#[test]
fn cursor_waiting_to_wrap_on_an_erased_cell() {
    let mut t = term();
    w(&mut t, &format!("{}\x1b[K", "a".repeat(COLS)));
    assert!(t.screen().cursor().pending_wrap);
    let mut b = replay(&t, 5000);
    let mut a = t;
    w(&mut a, "n");
    w(&mut b, "n");
    same_buffer(
        a.screen().buffers().0,
        b.screen().buffers().0,
        "after next char",
    );
}

#[test]
fn shell_input_lines_survive_a_write_back() {
    let mut t = term();
    w(
        &mut t,
        "\x1b]133;A\x07$ \x1b]133;B\x07echo https://a.b/\r\n\x1b]133;C\x07https://a.b/\r\n\x1b]133;D;0\x07\x1b]133;A\x07$ \x1b]133;B\x07ls",
    );
    assert!(t.screen().row(0).is_input());
    assert!(!t.screen().row(1).is_input());
    assert!(t.screen().row(2).is_input());
    let b = replay(&t, 100);
    assert!(b.screen().row(0).is_input());
    assert!(!b.screen().row(1).is_input());
    assert!(b.screen().row(2).is_input());
    // 書き戻したあとも、シェルの入力は続く
    let mut b = b;
    w(&mut b, "x\r\n");
    assert!(b.screen().row(2).is_input());
}

#[test]
fn a_terminal_without_shell_marks_serializes_without_them() {
    let mut t = term();
    w(&mut t, "plain\r\ntext");
    assert!(!String::from_utf8_lossy(&t.serialize(10)).contains("133"));
}
