//! DEC のモード（DECSET・DECRST）と ANSI のモード（SM・RM）のうち、画面の外に出ないもの。
//!
//! 画面の中のモード（自動折り返し、カーソルの表示と点滅、挿入モード）は
//! [`Screen`](crate::screen::Screen) が持つ。

/// マウスの報告の種類（`CSI ? 9 h`、`1000`、`1002`、`1003`）。ここでは状態を持つだけで、
/// 報告のバイト列を作るのは #10。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum MouseProtocol {
    #[default]
    /// 報告しない。
    None,
    /// 9。押したときだけ。
    X10,
    /// 1000。押したときと離したとき。
    Vt200,
    /// 1002。押したままの動きも。
    Drag,
    /// 1003。すべての動き。
    Any,
}

/// マウスの報告の書き方（`CSI ? 1005 h`、`1006`、`1015`、`1016`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum MouseEncoding {
    #[default]
    /// 既定（`CSI M` に 3 バイト）。
    Default,
    /// 1005。UTF-8。
    Utf8,
    /// 1006。SGR。
    Sgr,
    /// 1015。urxvt。
    Urxvt,
    /// 1016。ピクセル単位の SGR。
    SgrPixels,
}

/// モードの状態。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    /// DECCKM（1）。アプリケーションカーソルキー。
    pub application_cursor_keys: bool,
    /// DECOM（6）。カーソルの位置がスクロール範囲の左上からになる。
    pub origin: bool,
    /// 逆方向の折り返し（45）。
    pub reverse_wraparound: bool,
    /// 66。アプリケーションキーパッド（`ESC =` と `ESC >` と同じ状態）。
    pub application_keypad: bool,
    /// 1004。フォーカスの出入りを報告する。
    pub send_focus: bool,
    /// 2004。bracketed paste。
    pub bracketed_paste: bool,
    /// 2026。同期した出力。状態を持つだけで、描画を止めるのは呼ぶ側。
    pub synchronized_output: bool,
    /// 5522。クリップボードの貼り付けイベント（kitty）を受けたいか。状態を持つだけで、
    /// イベントを送るのは呼ぶ側。
    pub clipboard_paste_events: bool,
    /// LNM（20）。LF・VT・FF でカーソルが左端にも行く。
    pub linefeed_newline: bool,
    /// マウスの報告の種類。
    pub mouse_protocol: MouseProtocol,
    /// マウスの報告の書き方。
    pub mouse_encoding: MouseEncoding,
}
