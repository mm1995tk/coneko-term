//! OSC 22（mouse pointer shapes）。仕様の例どおりの動き。

use super::*;

fn term() -> Terminal {
    Terminal::new(5, 20)
}

fn w(t: &mut Terminal, s: &str) {
    t.write(s.as_bytes());
}

fn reply(t: &mut Terminal) -> String {
    String::from_utf8(t.take_output()).unwrap()
}

#[test]
fn nothing_set_means_no_shape() {
    let t = term();
    assert_eq!(t.pointer_shape(), None);
}

#[test]
fn set_with_and_without_equals() {
    let mut t = term();
    w(&mut t, "\x1b]22;pointer\x1b\\");
    assert_eq!(t.pointer_shape(), Some("pointer"));
    w(&mut t, "\x1b]22;=crosshair\x07");
    assert_eq!(t.pointer_shape(), Some("crosshair"));
}

#[test]
fn empty_set_resets_to_default() {
    let mut t = term();
    w(
        &mut t,
        "\x1b]22;>wait\x1b\\\x1b]22;>help\x1b\\\x1b]22;\x1b\\",
    );
    assert_eq!(t.pointer_shape(), None);
    // 積み重ねも空になっている
    w(&mut t, "\x1b]22;<\x1b\\");
    assert_eq!(t.pointer_shape(), None);
}

#[test]
fn set_replaces_the_top_of_the_stack() {
    let mut t = term();
    w(
        &mut t,
        "\x1b]22;>wait\x1b\\\x1b]22;>help\x1b\\\x1b]22;text\x1b\\",
    );
    assert_eq!(t.pointer_shape(), Some("text"));
    w(&mut t, "\x1b]22;<\x1b\\");
    assert_eq!(t.pointer_shape(), Some("wait"));
}

#[test]
fn push_and_pop() {
    let mut t = term();
    w(&mut t, "\x1b]22;>wait\x1b\\");
    assert_eq!(t.pointer_shape(), Some("wait"));
    w(&mut t, "\x1b]22;>pointer,move\x1b\\");
    assert_eq!(t.pointer_shape(), Some("move"));
    w(&mut t, "\x1b]22;<\x1b\\");
    assert_eq!(t.pointer_shape(), Some("pointer"));
    // 名前は読まない
    w(&mut t, "\x1b]22;<nonsense\x1b\\");
    assert_eq!(t.pointer_shape(), Some("wait"));
    w(&mut t, "\x1b]22;<\x1b\\\x1b]22;<\x1b\\");
    assert_eq!(t.pointer_shape(), None);
}

#[test]
fn full_stack_drops_the_bottom() {
    let mut t = term();
    w(&mut t, "\x1b]22;>help\x1b\\");
    for _ in 0..pointer_shape::MAX_STACK {
        w(&mut t, "\x1b]22;>wait\x1b\\");
    }
    for _ in 0..pointer_shape::MAX_STACK {
        w(&mut t, "\x1b]22;<\x1b\\");
    }
    // `help` は押し出されている
    assert_eq!(t.pointer_shape(), None);
}

#[test]
fn unknown_names_are_dropped() {
    let mut t = term();
    w(&mut t, "\x1b]22;pointer\x1b\\\x1b]22;no-such-name\x1b\\");
    assert_eq!(t.pointer_shape(), Some("pointer"));
    w(&mut t, "\x1b]22;>nope,wait,Pointer, \x1b\\");
    assert_eq!(t.pointer_shape(), Some("wait"));
    w(&mut t, "\x1b]22;<\x1b\\");
    assert_eq!(t.pointer_shape(), Some("pointer"));
    t.write(b"\x1b]22;\xff\xfe\x1b\\");
    assert_eq!(t.pointer_shape(), Some("pointer"));
}

#[test]
fn query_current() {
    let mut t = term();
    w(&mut t, "\x1b]22;?__current__\x1b\\");
    assert_eq!(reply(&mut t), "\x1b]22;0\x1b\\");
    w(&mut t, "\x1b]22;>wait\x1b\\\x1b]22;?__current__\x07");
    assert_eq!(reply(&mut t), "\x1b]22;wait\x07");
}

#[test]
fn query_support_and_specials() {
    let mut t = term();
    w(&mut t, "\x1b]22;?pointer,crosshair,no-such-name,wait\x1b\\");
    assert_eq!(reply(&mut t), "\x1b]22;1,1,0,1\x1b\\");
    w(&mut t, "\x1b]22;?__default__,__grabbed__\x1b\\");
    assert_eq!(reply(&mut t), "\x1b]22;text,default\x1b\\");
}

#[test]
fn every_spec_name_is_supported() {
    let mut t = term();
    for name in pointer_shape::SHAPES {
        w(&mut t, &format!("\x1b]22;{name}\x1b\\"));
        assert_eq!(t.pointer_shape(), Some(name));
    }
    assert_eq!(pointer_shape::SHAPES.len(), 30);
}

#[test]
fn main_and_alternate_screens_have_separate_stacks() {
    let mut t = term();
    w(&mut t, "\x1b]22;>wait\x1b\\");
    w(&mut t, "\x1b[?1049h");
    assert_eq!(t.pointer_shape(), None);
    w(&mut t, "\x1b]22;>help\x1b\\");
    assert_eq!(t.pointer_shape(), Some("help"));
    w(&mut t, "\x1b]22;?__current__\x1b\\");
    assert_eq!(reply(&mut t), "\x1b]22;help\x1b\\");
    w(&mut t, "\x1b[?1049l");
    assert_eq!(t.pointer_shape(), Some("wait"));
    w(&mut t, "\x1b[?1049h");
    assert_eq!(t.pointer_shape(), Some("help"));
}

#[test]
fn reset_empties_both_stacks() {
    let mut t = term();
    w(&mut t, "\x1b]22;>wait\x1b\\\x1b[?1049h\x1b]22;>help\x1b\\");
    w(&mut t, "\x1bc");
    assert_eq!(t.pointer_shape(), None);
    w(&mut t, "\x1b[?1049h");
    assert_eq!(t.pointer_shape(), None);
}

#[test]
fn independent_of_mouse_reporting() {
    let mut t = term();
    w(&mut t, "\x1b]22;pointer\x1b\\");
    assert!(!t.mouse_reporting());
    assert_eq!(t.pointer_shape(), Some("pointer"));
    w(&mut t, "\x1b[?1000h\x1b[?1006h\x1b]22;>wait\x1b\\");
    assert!(t.mouse_reporting());
    assert_eq!(t.pointer_shape(), Some("wait"));
    w(&mut t, "\x1b[?1000l");
    assert_eq!(t.pointer_shape(), Some("wait"));
}
