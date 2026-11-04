//! kitty の unscrolling（`CSI n + T`）の動き。

use super::*;
use crate::screen::Content;

fn term(rows: usize, cols: usize) -> Terminal {
    Terminal::new(rows, cols)
}

fn w(t: &mut Terminal, s: &str) {
    t.write(s.as_bytes());
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

fn rows(t: &Terminal) -> Vec<String> {
    (0..t.screen().rows()).map(|y| row_text(t, y)).collect()
}

/// 3 行の画面に 1..=6 を書く。画面は 4 5 6、スクロールバックは 1 2 3。
fn filled() -> Terminal {
    let mut t = term(3, 5);
    w(&mut t, "1\r\n2\r\n3\r\n4\r\n5\r\n6");
    assert_eq!(t.screen().scrollback_len(), 3);
    t
}

#[test]
fn restores_lines_from_scrollback() {
    let mut t = filled();
    w(&mut t, "\x1b[2+T");
    assert_eq!(rows(&t), ["2", "3", "4"]);
    assert_eq!(t.screen().scrollback_len(), 1);
}

#[test]
fn default_is_one_line() {
    let mut t = filled();
    w(&mut t, "\x1b[+T");
    assert_eq!(rows(&t), ["3", "4", "5"]);
    assert_eq!(t.screen().scrollback_len(), 2);
}

#[test]
fn short_scrollback_leaves_blank_lines() {
    let mut t = term(3, 5);
    w(&mut t, "1\r\n2\r\n3\r\n4");
    assert_eq!(t.screen().scrollback_len(), 1);
    w(&mut t, "\x1b[3+T");
    assert_eq!(rows(&t), ["", "", "1"].map(String::from));
    // 2 行目（元は 2）も 3 行ぶん下がって消える
    assert_eq!(t.screen().scrollback_len(), 0);
}

#[test]
fn count_is_capped_to_one_screen() {
    let mut t = filled();
    w(&mut t, "\x1b[100+T");
    assert_eq!(rows(&t), ["1", "2", "3"]);
    assert_eq!(t.screen().scrollback_len(), 0);
}

#[test]
fn keeps_attributes() {
    let mut t = term(2, 5);
    w(&mut t, "\x1b[1;31mA\x1b[0m\r\nB\r\nC");
    assert_eq!(t.screen().scrollback_len(), 1);
    let before = t.screen().scrollback_row(0).cells()[0].attrs;
    assert_ne!(before, crate::screen::Attrs::default());
    w(&mut t, "\x1b[+T");
    let cell = t.screen().row(0).cells()[0].clone();
    assert_eq!(cell.content, Content::Char('A'));
    assert_eq!(cell.attrs, before);
}

#[test]
fn alt_screen_is_plain_scroll_down() {
    let mut t = filled();
    w(&mut t, "\x1b[?1049h");
    w(&mut t, "\x1b[Hx\r\ny\r\nz");
    w(&mut t, "\x1b[2+T");
    assert_eq!(rows(&t), ["", "", "x"].map(String::from));
    w(&mut t, "\x1b[?1049l");
    // 主画面のスクロールバックは減らない
    assert_eq!(t.screen().scrollback_len(), 3);
    assert_eq!(rows(&t), ["4", "5", "6"]);
}

#[test]
fn within_scroll_region() {
    let mut t = filled();
    // 画面の 2 行目から 3 行目だけをスクロール範囲にする
    w(&mut t, "\x1b[2;3r\x1b[+T");
    assert_eq!(rows(&t), ["4", "3", "5"]);
    assert_eq!(t.screen().scrollback_len(), 2);
}

#[test]
fn cursor_does_not_move() {
    let mut t = filled();
    w(&mut t, "\x1b[2;3H\x1b[+T");
    let c = t.screen().cursor();
    assert_eq!((c.x, c.y), (2, 1));
}

#[test]
fn other_intermediates_and_extra_params_are_ignored() {
    let mut t = filled();
    w(&mut t, "\x1b[1;2+T\x1b[1$T\x1b[1+S");
    assert_eq!(rows(&t), ["4", "5", "6"]);
    assert_eq!(t.screen().scrollback_len(), 3);
}
