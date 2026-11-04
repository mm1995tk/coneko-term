//! マウスの報告。押した・離した・動かした・ホイールを、PTY へ送るバイト列にする。
//!
//! どのイベントを送るか（[`MouseProtocol`]）と、バイト列の書き方（[`MouseEncoding`]）は、
//! xterm.js 6.0.0 の `CoreMouseService` に合わせてある。
//!
//! - 書き方は既定（`CSI M` に 3 バイト）、SGR（1006）、ピクセル単位の SGR（1016）の 3 つ。
//!   xterm.js が対応しない UTF-8（1005）と urxvt（1015）は、既定の書き方として扱う。
//! - 既定の書き方で 1 バイトに収まらない位置（列か行が 223 を超える）は送らない。

use crate::terminal::{MouseEncoding, MouseProtocol};

/// マウスのボタン。ホイールもボタンの一種として扱う（xterm.js と同じ）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MouseButton {
    /// 左。
    Left,
    /// 真ん中。
    Middle,
    /// 右。
    Right,
    /// ボタンを押していない（動きだけ）。
    NoButton,
    /// 戻る（ボタン 8）。
    Back,
    /// 進む（ボタン 9）。
    Forward,
    /// ホイールを上へ。
    WheelUp,
    /// ホイールを下へ。
    WheelDown,
    /// ホイールを左へ。
    WheelLeft,
    /// ホイールを右へ。
    WheelRight,
}

impl MouseButton {
    fn is_wheel(self) -> bool {
        matches!(
            self,
            MouseButton::WheelUp
                | MouseButton::WheelDown
                | MouseButton::WheelLeft
                | MouseButton::WheelRight
        )
    }
}

/// マウスで起きたこと。ホイールは `Press` で表す。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MouseAction {
    /// 押した（ホイールを回した）。
    Press,
    /// 離した。
    Release,
    /// 動かした。
    Move,
}

/// マウスのイベント 1 つ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseEvent {
    /// ボタン。
    pub button: MouseButton,
    /// 起きたこと。
    pub action: MouseAction,
    /// 列（0 始まり）。
    pub col: usize,
    /// 行（0 始まり）。
    pub row: usize,
    /// 画面の左端からのピクセル（ピクセル単位の SGR だけが使う）。
    pub x: u32,
    /// 画面の上端からのピクセル（ピクセル単位の SGR だけが使う）。
    pub y: u32,
    /// Shift を押している。
    pub shift: bool,
    /// Alt（Option）を押している。
    pub alt: bool,
    /// Ctrl を押している。
    pub ctrl: bool,
}

impl MouseEvent {
    /// 位置と修飾キーが無いイベント。
    pub fn new(button: MouseButton, action: MouseAction, col: usize, row: usize) -> MouseEvent {
        MouseEvent {
            button,
            action,
            col,
            row,
            x: 0,
            y: 0,
            shift: false,
            alt: false,
            ctrl: false,
        }
    }
}

/// `protocol` が、このイベントを報告するか。X10 は修飾キーを報告しないので、
/// 報告するときは修飾キーを外したイベントを返す。
fn restrict(protocol: MouseProtocol, mut e: MouseEvent) -> Option<MouseEvent> {
    match protocol {
        MouseProtocol::None => None,
        MouseProtocol::X10 => {
            // 押したときだけ。ホイールも送らない。修飾キーは付けない
            if e.button.is_wheel() || e.action != MouseAction::Press {
                return None;
            }
            e.shift = false;
            e.alt = false;
            e.ctrl = false;
            Some(e)
        }
        // 動きは送らない
        MouseProtocol::Vt200 => (e.action != MouseAction::Move).then_some(e),
        // ボタンを押していない動きは送らない
        MouseProtocol::Drag => {
            (!(e.action == MouseAction::Move && e.button == MouseButton::NoButton)).then_some(e)
        }
        MouseProtocol::Any => Some(e),
    }
}

fn event_code(e: &MouseEvent, sgr: bool) -> u32 {
    let mut code = ((e.ctrl as u32) * 16) | ((e.shift as u32) * 4) | ((e.alt as u32) * 8);
    match e.button {
        MouseButton::WheelUp => code |= 64,
        MouseButton::WheelDown => code |= 64 | 1,
        MouseButton::WheelLeft => code |= 64 | 2,
        MouseButton::WheelRight => code |= 64 | 3,
        button => {
            code |= match button {
                MouseButton::Left => 0,
                MouseButton::Middle => 1,
                MouseButton::Right => 2,
                MouseButton::NoButton => 3,
                MouseButton::Back => 128,
                MouseButton::Forward => 128 | 1,
                _ => 0,
            };
            if e.action == MouseAction::Move {
                code |= 32;
            } else if e.action == MouseAction::Release && !sgr {
                // 既定の書き方では、離したときはどのボタンかを伝えない
                code |= 3;
            }
        }
    }
    code
}

/// イベントを PTY へ送るバイト列にする。送らないイベントと、既定の書き方で表せない位置は `None`。
pub fn encode(
    protocol: MouseProtocol,
    encoding: MouseEncoding,
    event: &MouseEvent,
) -> Option<Vec<u8>> {
    let e = restrict(protocol, *event)?;
    let (col, row) = (e.col as u64 + 1, e.row as u64 + 1);
    match encoding {
        MouseEncoding::Sgr | MouseEncoding::SgrPixels => {
            let (a, b) = if encoding == MouseEncoding::SgrPixels {
                (e.x as u64, e.y as u64)
            } else {
                (col, row)
            };
            // ホイールは離したことを持たない。離したときだけ最後が `m`
            let last = if e.action == MouseAction::Release && !e.button.is_wheel() {
                'm'
            } else {
                'M'
            };
            Some(format!("\x1b[<{};{};{}{}", event_code(&e, true), a, b, last).into_bytes())
        }
        // xterm.js が対応しない UTF-8（1005）と urxvt（1015）も、既定の書き方にする
        _ => {
            let code = event_code(&e, false) as u64 + 32;
            let (c, r) = (col + 32, row + 32);
            if code > 255 || c > 255 || r > 255 {
                return None;
            }
            Some(vec![0x1b, b'[', b'M', code as u8, c as u8, r as u8])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use MouseAction::*;
    use MouseButton::*;

    fn ev(button: MouseButton, action: MouseAction, col: usize, row: usize) -> MouseEvent {
        MouseEvent::new(button, action, col, row)
    }

    fn enc(p: MouseProtocol, e: MouseEncoding, event: MouseEvent) -> Option<String> {
        encode(p, e, &event).map(|v| v.iter().map(|&b| b as char).collect())
    }

    use MouseEncoding as E;
    use MouseProtocol as P;

    #[test]
    fn none_reports_nothing() {
        assert_eq!(enc(P::None, E::Default, ev(Left, Press, 0, 0)), None);
        assert_eq!(enc(P::None, E::Sgr, ev(WheelUp, Press, 0, 0)), None);
    }

    #[test]
    fn default_encoding_is_csi_m_with_offsets() {
        let s = enc(P::Vt200, E::Default, ev(Left, Press, 0, 0)).unwrap();
        assert_eq!(s, "\x1b[M !!");
        let s = enc(P::Vt200, E::Default, ev(Right, Press, 9, 4)).unwrap();
        assert_eq!(s, "\x1b[M\"*%");
        // 離したときはどのボタンでも 3
        let s = enc(P::Vt200, E::Default, ev(Left, Release, 0, 0)).unwrap();
        assert_eq!(s, "\x1b[M#!!");
        let s = enc(P::Vt200, E::Default, ev(Right, Release, 0, 0)).unwrap();
        assert_eq!(s, "\x1b[M#!!");
    }

    #[test]
    fn default_encoding_drops_positions_that_do_not_fit_in_a_byte() {
        // 列 223（1 始まり）が最後に送れる位置
        assert!(enc(P::Vt200, E::Default, ev(Left, Press, 222, 0)).is_some());
        assert_eq!(enc(P::Vt200, E::Default, ev(Left, Press, 223, 0)), None);
        assert_eq!(enc(P::Vt200, E::Default, ev(Left, Press, 0, 223)), None);
        let s = encode(P::Vt200, E::Default, &ev(Left, Press, 222, 222)).unwrap();
        assert_eq!(s, vec![0x1b, b'[', b'M', 32, 255, 255]);
        // SGR は大きな位置も送れる
        assert_eq!(
            enc(P::Vt200, E::Sgr, ev(Left, Press, 299, 0)).unwrap(),
            "\x1b[<0;300;1M"
        );
    }

    #[test]
    fn utf8_and_urxvt_fall_back_to_the_default_encoding() {
        let e = ev(Left, Press, 1, 2);
        let d = enc(P::Vt200, E::Default, e);
        assert_eq!(enc(P::Vt200, E::Utf8, e), d);
        assert_eq!(enc(P::Vt200, E::Urxvt, e), d);
    }

    #[test]
    fn sgr_encoding_uses_m_for_release_and_keeps_the_button() {
        assert_eq!(
            enc(P::Vt200, E::Sgr, ev(Left, Press, 4, 2)).unwrap(),
            "\x1b[<0;5;3M"
        );
        assert_eq!(
            enc(P::Vt200, E::Sgr, ev(Left, Release, 4, 2)).unwrap(),
            "\x1b[<0;5;3m"
        );
        assert_eq!(
            enc(P::Vt200, E::Sgr, ev(Right, Release, 4, 2)).unwrap(),
            "\x1b[<2;5;3m"
        );
        assert_eq!(
            enc(P::Vt200, E::Sgr, ev(Middle, Press, 0, 0)).unwrap(),
            "\x1b[<1;1;1M"
        );
    }

    #[test]
    fn sgr_pixels_reports_pixel_positions() {
        let mut e = ev(Left, Press, 4, 2);
        e.x = 123;
        e.y = 45;
        assert_eq!(enc(P::Vt200, E::SgrPixels, e).unwrap(), "\x1b[<0;123;45M");
        e.action = Release;
        assert_eq!(enc(P::Vt200, E::SgrPixels, e).unwrap(), "\x1b[<0;123;45m");
    }

    #[test]
    fn modifiers_are_added_to_the_button_code() {
        let mut e = ev(Left, Press, 0, 0);
        e.shift = true;
        assert_eq!(enc(P::Vt200, E::Sgr, e).unwrap(), "\x1b[<4;1;1M");
        e.alt = true;
        assert_eq!(enc(P::Vt200, E::Sgr, e).unwrap(), "\x1b[<12;1;1M");
        e.ctrl = true;
        assert_eq!(enc(P::Vt200, E::Sgr, e).unwrap(), "\x1b[<28;1;1M");
        // 既定の書き方では 32 を足す
        assert_eq!(encode(P::Vt200, E::Default, &e).unwrap()[3], 28 + 32);
    }

    #[test]
    fn wheel_uses_codes_64_to_67_and_never_a_release() {
        assert_eq!(
            enc(P::Vt200, E::Sgr, ev(WheelUp, Press, 0, 0)).unwrap(),
            "\x1b[<64;1;1M"
        );
        assert_eq!(
            enc(P::Vt200, E::Sgr, ev(WheelDown, Press, 0, 0)).unwrap(),
            "\x1b[<65;1;1M"
        );
        assert_eq!(
            enc(P::Vt200, E::Sgr, ev(WheelLeft, Press, 0, 0)).unwrap(),
            "\x1b[<66;1;1M"
        );
        assert_eq!(
            enc(P::Vt200, E::Sgr, ev(WheelRight, Press, 0, 0)).unwrap(),
            "\x1b[<67;1;1M"
        );
        assert_eq!(
            enc(P::Vt200, E::Default, ev(WheelDown, Press, 0, 0)).unwrap(),
            "\x1b[Ma!!"
        );
    }

    #[test]
    fn x10_reports_only_presses_without_modifiers() {
        let mut e = ev(Left, Press, 1, 1);
        e.ctrl = true;
        e.shift = true;
        assert_eq!(enc(P::X10, E::Default, e).unwrap(), "\x1b[M \"\"");
        assert_eq!(enc(P::X10, E::Default, ev(Left, Release, 1, 1)), None);
        assert_eq!(enc(P::X10, E::Default, ev(Left, Move, 1, 1)), None);
        assert_eq!(enc(P::X10, E::Default, ev(WheelUp, Press, 1, 1)), None);
        assert_eq!(
            enc(P::X10, E::Sgr, ev(Right, Press, 1, 1)).unwrap(),
            "\x1b[<2;2;2M"
        );
    }

    #[test]
    fn vt200_reports_press_release_and_wheel_but_not_motion() {
        let p = P::Vt200;
        assert!(enc(p, E::Sgr, ev(Left, Press, 0, 0)).is_some());
        assert!(enc(p, E::Sgr, ev(Left, Release, 0, 0)).is_some());
        assert!(enc(p, E::Sgr, ev(WheelUp, Press, 0, 0)).is_some());
        assert_eq!(enc(p, E::Sgr, ev(Left, Move, 0, 0)), None);
        assert_eq!(enc(p, E::Sgr, ev(NoButton, Move, 0, 0)), None);
    }

    #[test]
    fn drag_reports_motion_only_while_a_button_is_down() {
        let p = P::Drag;
        assert_eq!(
            enc(p, E::Sgr, ev(Left, Move, 3, 1)).unwrap(),
            "\x1b[<32;4;2M"
        );
        assert_eq!(
            enc(p, E::Sgr, ev(Right, Move, 3, 1)).unwrap(),
            "\x1b[<34;4;2M"
        );
        assert_eq!(enc(p, E::Sgr, ev(NoButton, Move, 3, 1)), None);
        assert!(enc(p, E::Sgr, ev(Left, Press, 0, 0)).is_some());
        assert!(enc(p, E::Sgr, ev(WheelDown, Press, 0, 0)).is_some());
    }

    #[test]
    fn any_reports_all_motion() {
        let p = P::Any;
        assert_eq!(
            enc(p, E::Sgr, ev(NoButton, Move, 3, 1)).unwrap(),
            "\x1b[<35;4;2M"
        );
        assert_eq!(
            enc(p, E::Default, ev(NoButton, Move, 3, 1)).unwrap(),
            "\x1b[MC$\""
        );
        assert!(enc(p, E::Sgr, ev(Left, Move, 3, 1)).is_some());
    }

    #[test]
    fn terminal_reports_through_the_output_and_skips_repeated_motion() {
        let mut t = crate::Terminal::new(5, 20);
        assert!(!t.mouse_reporting());
        assert!(!t.report_mouse(&ev(Left, Press, 0, 0)));
        assert!(t.take_output().is_empty());
        t.write(b"\x1b[?1003h\x1b[?1006h");
        assert!(t.mouse_reporting());
        assert!(t.report_mouse(&ev(Left, Press, 2, 3)));
        assert_eq!(t.take_output(), b"\x1b[<0;3;4M");
        assert!(t.report_mouse(&ev(NoButton, Move, 5, 5)));
        assert!(!t.report_mouse(&ev(NoButton, Move, 5, 5)));
        assert!(t.report_mouse(&ev(NoButton, Move, 6, 5)));
        assert_eq!(t.take_output(), b"\x1b[<35;6;6M\x1b[<35;7;6M");
        // 動き以外が挟まれば、同じ位置でももう一度報告する
        assert!(t.report_mouse(&ev(Left, Release, 6, 5)));
        assert!(t.report_mouse(&ev(NoButton, Move, 6, 5)));
        t.take_output();
        // 報告をやめると何も出ない
        t.write(b"\x1b[?1003l");
        assert!(!t.mouse_reporting());
        assert!(!t.report_mouse(&ev(Left, Press, 0, 0)));
        assert!(t.take_output().is_empty());
    }

    #[test]
    fn back_and_forward_buttons_set_bit_128() {
        assert_eq!(
            enc(P::Vt200, E::Sgr, ev(Back, Press, 0, 0)).unwrap(),
            "\x1b[<128;1;1M"
        );
        assert_eq!(
            enc(P::Vt200, E::Sgr, ev(Forward, Press, 0, 0)).unwrap(),
            "\x1b[<129;1;1M"
        );
    }
}
