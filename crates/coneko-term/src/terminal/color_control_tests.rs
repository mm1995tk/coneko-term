//! OSC 21（kitty の color control）と色の積み重ねのテスト。

use super::*;

fn w(t: &mut Terminal, s: &str) {
    t.write(s.as_bytes());
}

fn out(t: &mut Terminal) -> String {
    String::from_utf8(t.take_output()).unwrap()
}

fn term() -> Terminal {
    Terminal::new(3, 10)
}

#[test]
fn spec_example_query_and_unknown() {
    let mut t = term();
    w(
        &mut t,
        "\x1b]21;foreground=red;foreground=?;cursor=?;nonsense=?\x1b\\",
    );
    // cursor の既定は白（動的ではない）。
    assert_eq!(
        out(&mut t),
        "\x1b]21;foreground=rgb:ff/00/00;cursor=rgb:ff/ff/ff;unknown=bm9uc2Vuc2U\x1b\\"
    );
}

#[test]
fn undefined_color_answers_empty() {
    let mut t = term();
    w(
        &mut t,
        "\x1b]21;selection_background=?;cursor_text=?;transparent_background_color3=?\x07",
    );
    assert_eq!(
        out(&mut t),
        "\x1b]21;selection_background=;cursor_text=;transparent_background_color3=\x07"
    );
}

#[test]
fn set_then_reset_and_dynamic() {
    let mut t = term();
    w(
        &mut t,
        "\x1b]21;selection_foreground=#102030;7=rgb:1/2/3\x07",
    );
    assert_eq!(t.colors().selection_foreground, Some([0x10, 0x20, 0x30]));
    assert_eq!(t.colors().palette[7], [0x11, 0x22, 0x33]);
    assert!(t.take_colors_changed());
    assert!(!t.take_colors_changed());
    // 空の値は動的。キーだけを送ると既定に戻る。
    w(&mut t, "\x1b]21;selection_foreground=;7\x07");
    assert_eq!(t.colors().selection_foreground, None);
    assert_eq!(t.colors().palette[7], [0xd3, 0xd7, 0xcf]);
    // 動的にできない色への空の値は無視する。
    w(&mut t, "\x1b]21;background=\x07");
    assert_eq!(t.colors().background, [0, 0, 0]);
    assert!(out(&mut t).is_empty());
}

#[test]
fn bad_values_and_keys_are_dropped() {
    let mut t = term();
    w(
        &mut t,
        "\x1b]21;foreground=nonsense;256=#fff;-1=?;background=rgb:1/2\x07",
    );
    assert_eq!(t.colors(), &Colors::from_theme(&Theme::default()));
    // 範囲外の番号と知らないキーは unknown で返す。
    assert_eq!(out(&mut t), "\x1b]21;unknown=MjU2;unknown=LTE\x07");
}

#[test]
fn all_keys_round_trip() {
    let mut t = term();
    let keys = [
        "foreground",
        "background",
        "selection_background",
        "selection_foreground",
        "cursor",
        "cursor_text",
        "visual_bell",
        "transparent_background_color1",
        "transparent_background_color7",
        "0",
        "255",
    ];
    for k in keys {
        w(&mut t, &format!("\x1b]21;{k}=#0a0b0c\x07"));
        w(&mut t, &format!("\x1b]21;{k}=?\x07"));
        assert_eq!(out(&mut t), format!("\x1b]21;{k}=rgb:0a/0b/0c\x07"));
    }
}

#[test]
fn color_formats() {
    use super::color_control::parse_color;
    assert_eq!(parse_color("rgb:ff/80/00"), Some([255, 128, 0]));
    assert_eq!(parse_color("rgb:ffff/0000/8080"), Some([255, 0, 128]));
    assert_eq!(parse_color("#abc"), Some([0xa0, 0xb0, 0xc0]));
    assert_eq!(parse_color("#aabbcc"), Some([0xaa, 0xbb, 0xcc]));
    assert_eq!(parse_color("#aaabbbccc"), Some([0xaa, 0xbb, 0xcc]));
    assert_eq!(parse_color("#3a7"), parse_color("#3000a0007000"));
    assert_eq!(parse_color("#3a7"), parse_color("#3000a0007000"));
    assert_eq!(parse_color("rgbi:1.0/0.5/0"), Some([255, 128, 0]));
    assert_eq!(parse_color("RED@0.5"), Some([255, 0, 0]));
    assert_eq!(parse_color("#abcd"), None);
    assert_eq!(parse_color("rgb:1/2/3/4"), None);
    assert_eq!(parse_color("rgbi:2/-1/0"), Some([255, 0, 0]));
    assert_eq!(parse_color(""), None);
}

#[test]
fn osc_4_10_11_12_share_the_table() {
    let mut t = term();
    w(&mut t, "\x1b]21;1=#112233;foreground=#445566\x07");
    w(&mut t, "\x1b]4;1;?\x07\x1b]10;?\x07");
    assert_eq!(
        out(&mut t),
        "\x1b]4;1;rgb:1111/2222/3333\x07\x1b]10;rgb:4444/5555/6666\x07"
    );
    w(
        &mut t,
        "\x1b]4;2;rgb:ff/00/00\x07\x1b]11;#010203\x07\x1b]12;#040506\x07",
    );
    w(&mut t, "\x1b]21;2=?;background=?;cursor=?\x07");
    assert_eq!(
        out(&mut t),
        "\x1b]21;2=rgb:ff/00/00;background=rgb:01/02/03;cursor=rgb:04/05/06\x07"
    );
    // 104・110・111・112 で戻す。
    w(&mut t, "\x1b]104;2\x07\x1b]110\x07\x1b]111\x07\x1b]112\x07");
    w(&mut t, "\x1b]104;1\x07");
    w(&mut t, "\x1b]11;#010203\x07\x1b]111;\x07");
    assert_eq!(t.colors(), &Colors::from_theme(&Theme::default()));
}

#[test]
fn push_pop_report() {
    let mut t = term();
    w(&mut t, "\x1b[#R");
    assert_eq!(out(&mut t), "\x1b[0;0#Q");
    w(&mut t, "\x1b]21;foreground=#010101\x07\x1b[#P");
    w(&mut t, "\x1b]21;foreground=#020202\x07\x1b[#P");
    w(&mut t, "\x1b]21;foreground=#030303\x07\x1b[#R");
    assert_eq!(out(&mut t), "\x1b[2;2#Q");
    w(&mut t, "\x1b[#Q");
    assert_eq!(t.colors().foreground, [2, 2, 2]);
    w(&mut t, "\x1b[#Q");
    assert_eq!(t.colors().foreground, [1, 1, 1]);
    // 空の積み重ねからの pop は何もしない。
    w(&mut t, "\x1b[#Q");
    assert_eq!(t.colors().foreground, [1, 1, 1]);
    w(&mut t, "\x1b[#R");
    assert_eq!(out(&mut t), "\x1b[0;0#Q");
}

#[test]
fn push_pop_numbered_slots() {
    let mut t = term();
    w(&mut t, "\x1b]21;5=#0000aa\x07\x1b[3#P\x1b]21;5=#0000bb\x07");
    w(&mut t, "\x1b[3#Q");
    assert_eq!(t.colors().palette[5], [0, 0, 0xaa]);
    // 段の指定は段を残す。
    w(&mut t, "\x1b]21;5=#0000cc\x07\x1b[3#Q");
    assert_eq!(t.colors().palette[5], [0, 0, 0xaa]);
}

#[test]
fn stack_is_bounded_and_osc_30001() {
    let mut t = term();
    for i in 0..12u8 {
        w(
            &mut t,
            &format!("\x1b]21;foreground=#{i:02x}0000\x07\x1b]30001\x07"),
        );
    }
    w(&mut t, "\x1b[#R");
    assert_eq!(out(&mut t), "\x1b[10;10#Q");
    w(&mut t, "\x1b]30101\x07");
    assert_eq!(t.colors().foreground, [11, 0, 0]);
}

#[test]
fn ris_restores_colors_and_clears_stack() {
    let mut t = term();
    w(&mut t, "\x1b]21;foreground=red\x07\x1b[#P\x1bc");
    assert_eq!(t.colors(), &Colors::from_theme(&Theme::default()));
    w(&mut t, "\x1b[#R");
    assert_eq!(out(&mut t), "\x1b[0;0#Q");
}

#[test]
fn set_theme_resets_colors() {
    let mut t = term();
    w(&mut t, "\x1b]21;foreground=red\x07");
    t.take_colors_changed();
    t.set_theme(Theme {
        foreground: [1, 2, 3],
        ..Theme::default()
    });
    assert!(t.take_colors_changed());
    assert_eq!(t.colors().foreground, [1, 2, 3]);
}
