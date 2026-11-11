//! kitty の miscellaneous protocol extensions のうち、小さな拡張のテスト。
//! 仕様: <https://sw.kovidgoyal.net/kitty/misc-protocol/>

use super::*;
use crate::screen::cell::AttrFlags;
use crate::screen::Content;

fn term(rows: usize, cols: usize) -> Terminal {
    Terminal::new(rows, cols)
}

fn w(t: &mut Terminal, s: &str) {
    t.write(s.as_bytes());
}

fn out(t: &mut Terminal) -> String {
    String::from_utf8(t.take_output()).unwrap()
}

fn row_text(t: &Terminal, y: usize) -> String {
    let s: String = t
        .screen()
        .row(y)
        .cells()
        .iter()
        .map(|c| match &c.content {
            Content::Char(ch) => *ch,
            _ => ' ',
        })
        .collect();
    s.trim_end().to_string()
}

// ---- すべてのモードの保存と復元 ----

#[test]
fn save_and_restore_all_modes() {
    let mut t = term(3, 10);
    w(
        &mut t,
        "\x1b[?1h\x1b[?2004h\x1b[?1004h\x1b[?25l\x1b[?1003h\x1b[?1006h",
    );
    w(&mut t, "\x1b[?s");
    w(
        &mut t,
        "\x1b[?1l\x1b[?2004l\x1b[?1004l\x1b[?25h\x1b[?1003l\x1b[?1006l\x1b[?7l\x1b[?1000h",
    );
    w(&mut t, "\x1b[?r");
    let m = t.modes();
    assert!(m.application_cursor_keys);
    assert!(m.bracketed_paste);
    assert!(m.send_focus);
    assert!(!t.screen().cursor_visible);
    // マウスの種類は 1003 に戻り、1000 は残らない
    assert_eq!(m.mouse_protocol, MouseProtocol::Any);
    assert_eq!(m.mouse_encoding, MouseEncoding::Sgr);
    // 保存したときは設定中だった自動折り返しも戻る
    assert!(t.screen().wraparound);
}

#[test]
fn restore_without_save_changes_nothing() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b[?1h\x1b[?r");
    assert!(t.modes().application_cursor_keys);
}

#[test]
fn modes_with_side_effects_are_not_saved_without_a_list() {
    let mut t = term(5, 10);
    w(&mut t, "\x1b[?s");
    // DECOM と代替画面は、保存の対象ではない
    w(&mut t, "\x1b[?6h\x1b[?1049h");
    w(&mut t, "\x1b[3;3H\x1b[?r");
    assert!(t.modes().origin);
    assert!(t.screen().is_alt_screen());
    assert_eq!((t.screen().cursor().x, t.screen().cursor().y), (2, 2));
}

#[test]
fn xtsave_with_a_list_saves_only_those() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b[?2004h\x1b[?1004h\x1b[?2004;1004s");
    w(&mut t, "\x1b[?2004l\x1b[?1004l");
    w(&mut t, "\x1b[?2004r");
    assert!(t.modes().bracketed_paste);
    assert!(!t.modes().send_focus);
    w(&mut t, "\x1b[?1004r");
    assert!(t.modes().send_focus);
}

#[test]
fn xtsave_ignores_screen_switching_and_unknown_modes() {
    let mut t = term(3, 10);
    w(
        &mut t,
        "\x1b[?1049;47;1047;1048;9999s\x1b[?1049h\x1b[?1049;9999r",
    );
    assert!(t.screen().is_alt_screen());
}

#[test]
fn saving_again_replaces_the_saved_value() {
    let mut t = term(3, 10);
    w(
        &mut t,
        "\x1b[?2004h\x1b[?s\x1b[?2004l\x1b[?s\x1b[?2004h\x1b[?r",
    );
    assert!(!t.modes().bracketed_paste);
}

#[test]
fn ris_forgets_saved_modes() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b[?2004h\x1b[?s\x1bc\x1b[?r");
    assert!(!t.modes().bracketed_paste);
}

// ---- 太字と薄字を別々に消す SGR ----

#[test]
fn sgr_221_resets_only_bold() {
    let mut t = term(2, 10);
    w(&mut t, "\x1b[1;2m\x1b[221mA");
    let f = t.screen().row(0).cells()[0].attrs.flags;
    assert!(!f.contains(AttrFlags::BOLD));
    assert!(f.contains(AttrFlags::DIM));
}

#[test]
fn sgr_222_resets_only_faint() {
    let mut t = term(2, 10);
    w(&mut t, "\x1b[1;2m\x1b[222mA");
    let f = t.screen().row(0).cells()[0].attrs.flags;
    assert!(f.contains(AttrFlags::BOLD));
    assert!(!f.contains(AttrFlags::DIM));
}

#[test]
fn sgr_22_still_resets_both() {
    let mut t = term(2, 10);
    w(&mut t, "\x1b[1;2m\x1b[22mA");
    let f = t.screen().row(0).cells()[0].attrs.flags;
    assert!(!f.contains(AttrFlags::BOLD) && !f.contains(AttrFlags::DIM));
}

#[test]
fn sgr_221_mixes_with_other_params() {
    let mut t = term(2, 10);
    w(&mut t, "\x1b[1;2;31;222;3mA");
    let c = &t.screen().row(0).cells()[0].attrs;
    assert!(c.flags.contains(AttrFlags::BOLD));
    assert!(!c.flags.contains(AttrFlags::DIM));
    assert!(c.flags.contains(AttrFlags::ITALIC));
}

// ---- マウスが窓から出たこと ----

#[test]
fn mouse_leave_is_reported_with_sgr_pixels() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b[?1003h\x1b[?1016h");
    assert!(t.report_mouse_leave());
    // 動き（32）、押していない（3）、窓から出た（128）
    assert_eq!(out(&mut t), "\x1b[<163;0;0M");
}

#[test]
fn mouse_leave_is_not_reported_in_other_modes() {
    let mut t = term(3, 10);
    assert!(!t.report_mouse_leave());
    w(&mut t, "\x1b[?1003h");
    assert!(!t.report_mouse_leave());
    w(&mut t, "\x1b[?1006h");
    assert!(!t.report_mouse_leave());
    w(&mut t, "\x1b[?1000h\x1b[?1016h");
    assert!(!t.report_mouse_leave());
    assert_eq!(out(&mut t), "");
}

#[test]
fn mouse_move_is_reported_again_after_leaving() {
    let mut t = term(3, 10);
    w(&mut t, "\x1b[?1003h\x1b[?1016h");
    let mut e = MouseEvent::new(MouseButton::NoButton, MouseAction::Move, 1, 1);
    e.x = 12;
    e.y = 20;
    assert!(t.report_mouse(&e));
    assert!(!t.report_mouse(&e));
    assert!(t.report_mouse_leave());
    assert!(t.report_mouse(&e));
}

// ---- 画面の内容をスクロールバックへ移す ----

#[test]
fn csi_22_j_moves_the_screen_into_the_scrollback() {
    let mut t = term(4, 6);
    w(&mut t, "one\r\ntwo\r\nthree");
    assert_eq!(t.screen().scrollback_len(), 0);
    w(&mut t, "\x1b[22J");
    assert_eq!(t.screen().scrollback_len(), 3);
    let sb: Vec<String> = (0..3)
        .map(|i| {
            t.screen()
                .scrollback_row(i)
                .cells()
                .iter()
                .map(|c| c.text())
                .collect::<String>()
                .trim()
                .to_string()
        })
        .collect();
    assert_eq!(sb, ["one", "two", "three"]);
    // 画面は CSI 2 J のあとと同じ空。カーソルは動かない
    for y in 0..4 {
        assert_eq!(row_text(&t, y), "");
    }
    assert_eq!((t.screen().cursor().x, t.screen().cursor().y), (5, 2));
}

#[test]
fn csi_22_j_leaves_the_same_screen_as_csi_2_j() {
    let mut a = term(4, 6);
    let mut b = term(4, 6);
    for t in [&mut a, &mut b] {
        w(t, "\x1b[41mx\r\ny\x1b[2;3H");
    }
    w(&mut a, "\x1b[2J");
    w(&mut b, "\x1b[22J");
    assert_eq!(a.screen().cursor(), b.screen().cursor());
    for y in 0..4 {
        assert_eq!(a.screen().row(y).cells(), b.screen().row(y).cells());
    }
}

#[test]
fn csi_22_j_on_an_empty_screen_adds_nothing() {
    let mut t = term(4, 6);
    w(&mut t, "\x1b[22J");
    assert_eq!(t.screen().scrollback_len(), 0);
}

#[test]
fn csi_22_j_on_the_alt_screen_just_clears() {
    let mut t = term(3, 6);
    w(&mut t, "main\x1b[?1049hhello\x1b[22J");
    assert_eq!(row_text(&t, 0), "");
    assert_eq!(t.screen().scrollback_len(), 0);
    w(&mut t, "\x1b[?1049l");
    assert_eq!(row_text(&t, 0), "main");
    assert_eq!(t.screen().scrollback_len(), 0);
}

#[test]
fn csi_22_j_moves_images_too() {
    let mut t = term(4, 10);
    w(&mut t, "\x1b_Ga=T,f=24,s=1,v=1,i=1,c=2,r=2;AAAA\x1b\\");
    out(&mut t);
    assert_eq!(t.placements().len(), 1);
    w(&mut t, "\x1b[22J");
    let ps = t.placements();
    assert_eq!(ps.len(), 1, "画像はスクロールバックの側に残る");
    assert!(
        ps[0].row + 2 <= 0,
        "画像は画面の外へ出ている: {}",
        ps[0].row
    );
}

#[test]
fn csi_22_j_still_removes_extra_cursors() {
    let mut t = term(3, 6);
    w(&mut t, "\x1b[>1;2:3 q");
    w(&mut t, "\x1b[22J");
    assert!(t.screen().extra_cursors.is_empty());
}

#[test]
fn osc_133_marks_the_lines_the_shell_takes_input_on() {
    let mut t = term(5, 10);
    w(
        &mut t,
        "\x1b]133;A\x07$ ls\r\n\x1b]133;C\x07out\r\n\x1b]133;D;0\x07\x1b]133;A\x07$ ",
    );
    let s = t.screen();
    assert!(s.row(0).is_input());
    assert!(!s.row(1).is_input());
    assert!(s.row(2).is_input());
    assert!(!s.row(3).is_input());
    // 消した行の印は消える
    w(&mut t, "\x1b[2K");
    assert!(!t.screen().row(2).is_input());
}

#[test]
fn no_osc_133_means_no_input_lines() {
    let mut t = term(3, 10);
    w(&mut t, "$ ls\r\n");
    assert!(!t.screen().row(0).is_input());
}

#[test]
fn osc_133_is_tracked_when_the_host_registers_it() {
    let mut t = term(3, 10);
    t.register_osc(133);
    w(&mut t, "\x1b]133;A\x07$");
    assert!(t.screen().row(0).is_input());
}

// ---- 幅が変わったときの、シェルのプロンプトの扱い ----

/// スクロールバックと画面を、折り返しでつながった行を 1 本にして並べる（末尾の空行は除く）。
fn logical_lines(t: &Terminal) -> Vec<String> {
    let s = t.screen();
    let rows: Vec<&crate::screen::Line> = (0..s.scrollback_len())
        .map(|i| s.scrollback_row(i))
        .chain((0..s.rows()).map(|y| s.row(y)))
        .collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for l in rows {
        let text: String = l
            .cells()
            .iter()
            .map(|c| if c.is_empty() { " ".into() } else { c.text() })
            .collect();
        cur.push_str(if l.is_wrapped() {
            &text
        } else {
            text.trim_end()
        });
        if !l.is_wrapped() {
            out.push(std::mem::take(&mut cur));
        }
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out
}

const PROMPT1_LEN: usize = 120;

fn prompt1() -> String {
    format!("~/work/{}", "d".repeat(PROMPT1_LEN - 7))
}

fn ls_lines() -> Vec<String> {
    (0..5)
        .map(|i| format!("ls{i}:{}", "x".repeat(130)))
        .collect()
}

/// `ls` の長い行 5 本のあとに、2 行のプロンプト（1 行目は 100 列より長い）を書いた端末。
/// `marks` が真なら、OSC 133 の `A`・`B` を付ける。
fn shell_term(marks: bool) -> Terminal {
    let mut t = term(24, 100);
    // 前のコマンドの出力の始まり。シェルが `C` を出すと分かってから、プロンプトの始まりの印をつける
    w(&mut t, "\x1b]133;C\x07");
    for l in ls_lines() {
        w(&mut t, &format!("{l}\r\n"));
    }
    w(&mut t, &prompt(marks));
    t
}

fn prompt(marks: bool) -> String {
    let (a, b) = if marks {
        ("\x1b]133;A\x07", "\x1b]133;B\x07")
    } else {
        ("", "")
    };
    format!("{a}{}\r\n% {b}", prompt1())
}

/// zsh が SIGWINCH で描き直す出力。`up` は、シェルが覚えている、プロンプトの先頭へ戻る段数。
fn zsh_redraw(up: usize, marks: bool) -> String {
    format!("\r{}\x1b[0m\x1b[J{}", "\x1b[A".repeat(up), prompt(marks))
}

fn expected_after_redraw() -> Vec<String> {
    let mut v = ls_lines();
    v.push(prompt1());
    v.push("%".to_string());
    v
}

#[test]
fn prompt_is_redrawn_by_the_shell_instead_of_reflowed() {
    let mut t = shell_term(true);
    t.resize(24, 60);
    // 組み直さず、プロンプトの始まりから下は空。上の `ls` は組み直される
    assert_eq!(logical_lines(&t), ls_lines());
    // カーソルは、始まりから同じ段目（2 段目）に残る
    let c = t.screen().cursor();
    assert_eq!(c.y, 5 * 3 + 2);
    // シェルが描き直すと、プロンプトの 1 行目は 1 回だけ出る
    w(&mut t, &zsh_redraw(2, true));
    assert_eq!(logical_lines(&t), expected_after_redraw());
    assert_eq!(t.screen().scrollback_len(), 0);
}

#[test]
fn prompt_does_not_pile_up_when_narrowed_step_by_step() {
    let mut t = shell_term(true);
    let mut width = 100;
    while width > 60 {
        let up = PROMPT1_LEN.div_ceil(width);
        width -= 3;
        t.resize(24, width);
        w(&mut t, &zsh_redraw(up, true));
        assert_eq!(logical_lines(&t), expected_after_redraw(), "{width} 列");
    }
}

#[test]
fn only_the_lines_from_the_prompt_start_are_cleared() {
    let mut t = term(8, 20);
    w(
        &mut t,
        "\x1b]133;C\x07out\r\n\x1b]133;A\x07$ \x1b]133;B\x07",
    );
    t.resize(8, 10);
    assert_eq!(logical_lines(&t), ["out"]);
    assert_eq!(t.screen().cursor().y, 1);
}

#[test]
fn cursor_outside_the_prompt_reflows_as_before() {
    let mut t = term(8, 20);
    w(&mut t, "\x1b]133;A\x07$ ls\r\n\x1b]133;C\x07");
    w(&mut t, &"y".repeat(30));
    w(&mut t, "\r\n\x1b]133;D;0\x07");
    t.resize(8, 10);
    assert_eq!(logical_lines(&t), ["$ ls", &"y".repeat(30)]);
}

#[test]
fn shell_without_osc_133_is_reflowed_as_before() {
    let mut t = shell_term(false);
    t.resize(24, 60);
    // プロンプトも組み直される
    let mut want = ls_lines();
    want.push(prompt1());
    want.push("%".to_string());
    assert_eq!(logical_lines(&t), want);
    let c = t.screen().cursor();
    assert_eq!(c.y, 5 * 3 + 2);
    assert_eq!(c.x, 2);
}

#[test]
fn shell_that_never_sends_osc_133_c_is_reflowed_as_before() {
    // `A` だけのシェルは、実行中のコマンドの出力の行にも入力の印がつくので、空にすると出力が消える
    let mut t = term(8, 20);
    w(&mut t, "\x1b]133;A\x07$ make\r\n");
    w(&mut t, "line1\r\nline2\r\nline3");
    t.resize(8, 10);
    assert_eq!(logical_lines(&t), ["$ make", "line1", "line2", "line3"]);
}

#[test]
fn alt_screen_does_not_change_the_primary_prompt_rule() {
    // 全画面のプログラムを動かしている間も、主画面のプロンプトは空にされる（シェルが描き直す前提）
    let mut t = shell_term(true);
    w(&mut t, "\x1b[?1049h");
    t.resize(24, 60);
    w(&mut t, "\x1b[?1049l");
    assert_eq!(logical_lines(&t), ls_lines());
}
