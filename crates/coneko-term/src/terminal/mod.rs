//! ターミナル本体。バイト列を受け取り、パーサー（[`crate::parser`]）が分けたシーケンスで
//! 画面（[`crate::screen`]）を動かす。
//!
//! 対応するシーケンスは xterm.js 6.0.0 の対応表で Supported のもののうち、OSC 8（リンク）を
//! 除いたもの。知らないシーケンスは捨てる。
//!
//! # 外へ出すもの（[`Event`]）
//!
//! BEL・題名の変更・通知（OSC 99）は [`Event`] になって溜まる。[`Terminal::take_events`] で取り出す。
//! 呼ぶ側が取り出さないと溜まり続けるので、`write` のたびに取り出すこと。
//!
//! # PTY へ送るバイト列（出口は 1 つ）
//!
//! 問い合わせへの返事（DA1・DA2・DSR・DECRQM・色の問い合わせ）と、フォーカスの報告は、
//! ターミナルから PTY へ送るバイト列になって溜まる。[`Terminal::take_output`] で取り出して
//! PTY に書く。キーボード（#9）とマウス（#10）のバイト列も、[`Terminal::queue_output`] で
//! 同じ出口に入れる。呼ぶ側が取り出さないと溜まり続けるので、`write` のたびに取り出すこと。
//!
//! # 画像（kitty graphics）
//!
//! APC で送られた画像は [`crate::graphics`] が受け取って持つ。返事は同じ出口に入る。
//!
//! # モード
//!
//! DECSET・DECRST と SM・RM は [`Modes`]（[`Terminal::modes`]）と [`Screen`] の公開の項目が
//! 持つ。対応するモードは次のとおり。DECRQM は、これ以外のモードに 0（知らない）を返す。
//!
//! - ANSI: 4（IRM）、20（LNM）
//! - DEC: 1、6、7、9、12、25、45、47、66、1000、1002、1003、1004、1005、1006、1015、1016、
//!   1047、1048、1049、2004、2026
//!
//! # 画面の状態のうち、ここで持つもの
//!
//! 文字集合（G0〜G3 と GL）、直前に書いた字（REP 用）、DECSC で保存した文字集合。

mod kitty_keyboard;
#[cfg(test)]
mod mode_tests;
mod modes;
mod notification;
mod sgr;
#[cfg(test)]
mod tests;
mod theme;

pub use kitty_keyboard::{KeyboardFlagStack, DISAMBIGUATE, MAX_DEPTH as KEYBOARD_STACK_DEPTH};
pub use modes::{Modes, MouseEncoding, MouseProtocol};
pub use notification::{Actions, Notification, NotificationSupport, When};
pub use theme::{Rgb, Theme};

use crate::graphics::{DeleteSpec, DeleteTarget, Effect, Graphics, Image, PlacedImage, Placement};
use crate::mouse::{self, MouseAction, MouseButton, MouseEvent};
use crate::parser::{Params, Parser, Perform};
use crate::screen::{CursorShape, Screen};

/// REP の繰り返しの上限。
const MAX_REPEAT: usize = 65535;

/// ターミナルが外へ知らせること。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// BEL。
    Bell,
    /// ウィンドウの題名が変わった（OSC 0、OSC 2）。
    Title(String),
    /// アイコンの名前が変わった（OSC 0、OSC 1）。
    IconName(String),
    /// 通知（OSC 99）が 1 つそろった。
    Notification(Box<Notification>),
    /// アプリケーションが通知を閉じるよう求めた（`p=close`）。
    NotificationClose {
        /// `i=`。
        id: String,
    },
    /// アプリケーションが対応を尋ねた（`p=?`）。[`Terminal::notification_query_reply`] で答える。
    NotificationQuery {
        /// `i=`（無ければ空）。
        id: String,
    },
    /// アプリケーションが生きている通知を尋ねた（`p=alive`）。
    /// [`Terminal::notification_alive_reply`] で答える。
    NotificationAlive {
        /// `i=`（無ければ空）。
        id: String,
    },
    /// [`Terminal::register_osc`] で登録した番号の OSC が来た。`data` は番号と `;` のあとの
    /// バイト列（`;` が無ければ空）。
    Osc {
        /// OSC の番号。
        ident: u32,
        /// 番号のあとのバイト列。
        data: Vec<u8>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Charset {
    Ascii,
    Uk,
    DecSpecial,
}

impl Charset {
    fn from_final(b: u8) -> Charset {
        match b {
            b'0' => Charset::DecSpecial,
            b'A' => Charset::Uk,
            _ => Charset::Ascii,
        }
    }

    fn map(self, c: char) -> char {
        match self {
            Charset::Ascii => c,
            Charset::Uk => {
                if c == '#' {
                    '£'
                } else {
                    c
                }
            }
            Charset::DecSpecial => match c {
                '_' => ' ',
                '`' => '◆',
                'a' => '▒',
                'b' => '␉',
                'c' => '␌',
                'd' => '␍',
                'e' => '␊',
                'f' => '°',
                'g' => '±',
                'h' => '␤',
                'i' => '␋',
                'j' => '┘',
                'k' => '┐',
                'l' => '┌',
                'm' => '└',
                'n' => '┼',
                'o' => '⎺',
                'p' => '⎻',
                'q' => '─',
                'r' => '⎼',
                's' => '⎽',
                't' => '├',
                'u' => '┤',
                'v' => '┴',
                'w' => '┬',
                'x' => '│',
                'y' => '≤',
                'z' => '≥',
                '{' => 'π',
                '|' => '≠',
                '}' => '£',
                '~' => '·',
                _ => c,
            },
        }
    }
}

/// 動きの報告の重なりを見分ける印（位置、ボタン、修飾キー）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MouseMoveKey {
    pos: (u64, u64),
    button: MouseButton,
    mods: (bool, bool, bool),
}

/// ターミナル。`write` にバイト列を渡すと、画面が変わる。
#[derive(Debug)]
pub struct Terminal {
    parser: Parser,
    state: State,
    /// 直前に報告した動き。同じ位置の動きを続けて報告しないために覚える。
    last_mouse_move: Option<MouseMoveKey>,
}

#[derive(Debug)]
struct State {
    screen: Screen,
    events: Vec<Event>,
    /// G0〜G3。
    charsets: [Charset; 4],
    /// GL に呼び出している G0〜G3 の番号。
    gl: usize,
    saved_charsets: Option<([Charset; 4], usize)>,
    last_char: Option<char>,
    modes: Modes,
    /// kitty keyboard protocol のフラグの積み重ね。主画面（0）と代替画面（1）で別々。
    keyboard_flags: [KeyboardFlagStack; 2],
    theme: Theme,
    /// PTY へ送るバイト列。
    output: Vec<u8>,
    /// 呼ぶ側が拾う OSC の番号。
    osc_handlers: Vec<u32>,
    /// kitty graphics で受け取った画像。
    graphics: Graphics,
    /// 組み立て中の通知（OSC 99）。
    notifications: notification::Notifications,
}

impl Terminal {
    /// `rows` 行 `cols` 列のターミナル。
    pub fn new(rows: usize, cols: usize) -> Terminal {
        Terminal::with_screen(Screen::new(rows, cols))
    }

    /// 作っておいた画面を使う（スクロールバックの行数を変えたいときなど）。
    pub fn with_screen(screen: Screen) -> Terminal {
        Terminal {
            parser: Parser::new(),
            last_mouse_move: None,
            state: State {
                screen,
                events: Vec::new(),
                charsets: [Charset::Ascii; 4],
                gl: 0,
                saved_charsets: None,
                last_char: None,
                modes: Modes::default(),
                keyboard_flags: Default::default(),
                theme: Theme::default(),
                output: Vec::new(),
                osc_handlers: Vec::new(),
                graphics: Graphics::default(),
                notifications: Default::default(),
            },
        }
    }

    /// バイト列を書く。途中で切れた字やシーケンスは、次の `write` に続く。
    pub fn write(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.state, bytes);
    }

    /// 画面。
    pub fn screen(&self) -> &Screen {
        &self.state.screen
    }

    /// 画面を変える。描いたあとに `take_dirty_rows` を呼ぶときなど。
    pub fn screen_mut(&mut self) -> &mut Screen {
        &mut self.state.screen
    }

    /// 完全に初期状態へ戻す（RIS と同じ）。溜まっているイベントと出力、登録した OSC の番号、
    /// テーマは残す。
    pub fn reset(&mut self) {
        self.parser = Parser::new();
        self.state.full_reset();
        self.last_mouse_move = None;
    }

    /// OSC の番号 `ident` を呼ぶ側で拾う。その番号の OSC は、ターミナルは処理せず
    /// [`Event::Osc`] になる（0・1・2・4・10・11・12 を登録すると、内蔵の処理の代わりになる）。
    pub fn register_osc(&mut self, ident: u32) {
        if !self.state.osc_handlers.contains(&ident) {
            self.state.osc_handlers.push(ident);
        }
    }

    /// [`Terminal::register_osc`] の登録を外す。
    pub fn unregister_osc(&mut self, ident: u32) {
        self.state.osc_handlers.retain(|&i| i != ident);
    }

    /// 大きさを変える。
    pub fn resize(&mut self, rows: usize, cols: usize) {
        self.state.screen.resize(rows, cols);
    }

    /// 溜まったイベントを古い順に取り出す。
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.state.events)
    }

    /// アプリケーションキーパッドか（`ESC =` と `CSI ? 66 h` で真、`ESC >` と `CSI ? 66 l` で偽）。
    pub fn application_keypad(&self) -> bool {
        self.state.modes.application_keypad
    }

    /// モードの状態。
    pub fn modes(&self) -> &Modes {
        &self.state.modes
    }

    /// いまの画面の kitty keyboard protocol のフラグ（`CSI ? u` で答える値）。
    pub fn keyboard_flags(&self) -> u32 {
        self.state.active_keyboard_flags().flags()
    }

    /// PTY へ送るバイト列を取り出す。問い合わせへの返事など、ターミナルが出したものが
    /// 出た順に入っている。
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.state.output)
    }

    /// PTY へ送るバイト列を出口に入れる。キーボードやマウスのバイト列は、これで返事と
    /// 同じ出口から出す。
    pub fn queue_output(&mut self, bytes: &[u8]) {
        self.state.output.extend_from_slice(bytes);
    }

    /// 通知（OSC 99）が押されたことを、アプリケーションへ返す。`button` はボタンの番号（1 始まり）。
    /// 通知の `a` に `report` があるときだけ送ること。`i=` の無い通知には `i=0` で返す（仕様）。
    pub fn notification_activated(&mut self, id: &str, button: Option<u32>) {
        self.queue_output(&notification::activated(id, button));
    }

    /// 通知が閉じられたことを、アプリケーションへ返す。通知の `c=1` のときだけ送ること。
    pub fn notification_closed(&mut self, id: &str) {
        self.queue_output(&notification::closed(id));
    }

    /// `p=?` の問い合わせ（[`Event::NotificationQuery`]）に答える。
    pub fn notification_query_reply(&mut self, id: &str, support: &NotificationSupport) {
        self.queue_output(&notification::query_reply(id, support));
    }

    /// `p=alive` の問い合わせ（[`Event::NotificationAlive`]）に答える。`ids` は生きている通知の `i=`。
    pub fn notification_alive_reply(&mut self, id: &str, ids: &[String]) {
        self.queue_output(&notification::alive_reply(id, ids));
    }

    /// 受け取った画像を ID で引く（kitty graphics の `i=`）。
    pub fn image(&self, id: u32) -> Option<&Image> {
        self.state.graphics.get(id)
    }

    /// 受け取った画像を番号で引く（`I=`）。同じ番号の画像が複数あるときは、いちばん新しいもの。
    pub fn image_by_number(&self, number: u32) -> Option<&Image> {
        self.state.graphics.get_by_number(number)
    }

    /// 持っている画像を古い順に。
    pub fn images(&self) -> impl Iterator<Item = &Image> {
        self.state.graphics.iter()
    }

    /// 持っている画像のバイト列の合計。
    pub fn image_bytes(&self) -> usize {
        self.state.graphics.bytes()
    }

    /// 画像 `id` のいまのコマの表示時間が過ぎたと知らせ、次のコマへ進める。進んだら `true`。
    /// 止まっているとき、画像が無いとき、最後のコマで次を待つときは `false`。
    /// 時計は持たないので、いつ呼ぶかは呼ぶ側が決める（表示時間は [`crate::graphics::Animation::gaps`]）。
    pub fn animation_step(&mut self, id: u32) -> bool {
        self.state.graphics.step_animation(id)
    }

    /// 画像の合計の大きさの上限（バイト）。既定は [`crate::graphics::DEFAULT_QUOTA`]。
    pub fn image_quota(&self) -> usize {
        self.state.graphics.quota()
    }

    /// 画像の合計の大きさの上限を変える。超えている分は、古い画像から捨てる。
    pub fn set_image_quota(&mut self, bytes: usize) {
        self.state.graphics.set_quota(bytes);
        self.state.drop_removed_images();
    }

    /// 画面に置いてある画像の配置を、上の行から順に。今の画面（主画面か代替画面）のもので、
    /// スクロールバックの中のものも含む。
    pub fn placements(&self) -> Vec<PlacedImage> {
        let mut out: Vec<PlacedImage> = self
            .state
            .screen
            .placements()
            .into_iter()
            .map(|(row, p)| PlacedImage {
                row,
                placement: p.clone(),
            })
            .collect();
        out.extend(crate::graphics::derive_placeholders(
            &self.state.graphics,
            self.state.screen.rows_with_index(),
        ));
        out
    }

    /// セルの大きさ（画素。幅, 高さ）。
    pub fn cell_size(&self) -> (u32, u32) {
        self.state.graphics.cell_size()
    }

    /// セルの大きさを教える。`c`・`r` が無い画像の配置の大きさを決めるのに使う。0 は無視する。
    /// 既定は [`crate::graphics::DEFAULT_CELL_SIZE`]。
    pub fn set_cell_size(&mut self, width: u32, height: u32) {
        self.state.graphics.set_cell_size(width, height);
    }

    /// マウスの報告が有効か（`CSI ? 9 h`、`1000`、`1002`、`1003` のどれか）。
    pub fn mouse_reporting(&self) -> bool {
        self.state.modes.mouse_protocol != MouseProtocol::None
    }

    /// マウスのイベントを報告する。今のモードが報告しないイベント（と、書き方で表せない位置）は
    /// 何もせず `false`。報告するときは、バイト列を [`Terminal::take_output`] の出口に入れて `true`。
    /// 位置が前と同じ動きは、続けて報告しない（`false`）。
    pub fn report_mouse(&mut self, event: &MouseEvent) -> bool {
        let m = &self.state.modes;
        let Some(bytes) = mouse::encode(m.mouse_protocol, m.mouse_encoding, event) else {
            return false;
        };
        if event.action == MouseAction::Move {
            let pixels = m.mouse_encoding == MouseEncoding::SgrPixels;
            let key = MouseMoveKey {
                pos: if pixels {
                    (event.x as u64, event.y as u64)
                } else {
                    (event.col as u64, event.row as u64)
                },
                button: event.button,
                mods: (event.shift, event.alt, event.ctrl),
            };
            if self.last_mouse_move == Some(key) {
                return false;
            }
            self.last_mouse_move = Some(key);
        } else {
            self.last_mouse_move = None;
        }
        self.state.output.extend_from_slice(&bytes);
        true
    }

    /// 色の問い合わせに返すテーマ。
    pub fn theme(&self) -> &Theme {
        &self.state.theme
    }

    /// 色の問い合わせに返すテーマを変える。
    pub fn set_theme(&mut self, theme: Theme) {
        self.state.theme = theme;
    }

    /// ウィンドウがフォーカスを得た・失ったことを知らせる。フォーカスの報告（`CSI ? 1004 h`）が
    /// 有効なら、`CSI I`（得た）か `CSI O`（失った）を出口に入れる。
    pub fn set_focus(&mut self, focused: bool) {
        if self.state.modes.send_focus {
            self.state.reply(if focused { "\x1b[I" } else { "\x1b[O" });
        }
    }
}

/// `index` 番目のパラメーターの先頭の値。無いときは `None`。
fn raw(params: &Params, index: usize) -> Option<u32> {
    params.get(index).and_then(|p| p.first().copied())
}

/// `index` 番目のパラメーター。無いか 0 のときは `default`。
fn arg(params: &Params, index: usize, default: usize) -> usize {
    match raw(params, index) {
        None | Some(0) => default,
        Some(v) => v as usize,
    }
}

impl State {
    /// 返事を出口に入れる。
    fn reply(&mut self, s: &str) {
        self.output.extend_from_slice(s.as_bytes());
    }

    /// 1 から数えた行の指定を、画面の行（0 始まり）にする。原点モードでは範囲の中に収める。
    fn row_from_param(&self, n: usize) -> usize {
        if self.modes.origin {
            let (top, bottom) = self.screen.scroll_region();
            (top + n - 1).min(bottom)
        } else {
            n - 1
        }
    }

    /// カーソルを左上へ。原点モードではスクロール範囲の左上。
    fn home_cursor(&mut self) {
        let y = self.row_from_param(1);
        self.screen.set_cursor(0, y);
    }

    /// BS。逆方向の折り返し（45）では、前の行から折り返された行の行頭から、前の行の末尾へ戻る。
    fn backspace(&mut self) {
        let c = self.screen.cursor();
        if self.modes.reverse_wraparound
            && c.x == 0
            && c.y > self.screen.scroll_region().0
            && self.screen.row(c.y - 1).is_wrapped()
        {
            let cols = self.screen.cols();
            self.screen.set_cursor(cols - 1, c.y - 1);
        } else {
            self.screen.backspace();
        }
    }

    fn move_cursor_to(&mut self, x: usize, y: usize) {
        self.screen.set_cursor(x, y);
    }

    fn cursor_up(&mut self, n: usize) {
        let c = self.screen.cursor();
        let (top, _) = self.screen.scroll_region();
        let limit = if c.y >= top { top } else { 0 };
        self.move_cursor_to(c.x, c.y.saturating_sub(n).max(limit));
    }

    fn cursor_down(&mut self, n: usize) {
        let c = self.screen.cursor();
        let (_, bottom) = self.screen.scroll_region();
        let limit = if c.y <= bottom {
            bottom
        } else {
            self.screen.rows() - 1
        };
        self.move_cursor_to(c.x, c.y.saturating_add(n).min(limit));
    }

    fn cursor_forward(&mut self, n: usize) {
        let c = self.screen.cursor();
        self.move_cursor_to(c.x.saturating_add(n), c.y);
    }

    fn cursor_backward(&mut self, n: usize) {
        let c = self.screen.cursor();
        self.move_cursor_to(c.x.saturating_sub(n), c.y);
    }

    fn put(&mut self, c: char) {
        self.screen.print(c);
        self.last_char = Some(c);
    }

    fn erase_in_display(&mut self, mode: u32) {
        let c = self.screen.cursor();
        let (rows, cols) = (self.screen.rows(), self.screen.cols());
        match mode {
            0 => {
                self.screen.erase_cells(c.y, c.x, cols);
                for y in c.y + 1..rows {
                    self.screen.erase_cells(y, 0, cols);
                }
            }
            1 => {
                for y in 0..c.y {
                    self.screen.erase_cells(y, 0, cols);
                }
                self.screen.erase_cells(c.y, 0, c.x + 1);
            }
            2 => {
                self.screen.clear_visible_placements();
                for y in 0..rows {
                    self.screen.erase_cells(y, 0, cols);
                }
            }
            3 => self.screen.clear_scrollback(),
            _ => {}
        }
    }

    fn erase_in_line(&mut self, mode: u32) {
        let c = self.screen.cursor();
        let cols = self.screen.cols();
        match mode {
            0 => self.screen.erase_cells(c.y, c.x, cols),
            1 => self.screen.erase_cells(c.y, 0, c.x + 1),
            2 => self.screen.erase_cells(c.y, 0, cols),
            _ => {}
        }
    }

    fn set_scroll_region(&mut self, params: &Params) {
        let rows = self.screen.rows();
        let top = arg(params, 0, 1);
        let bottom = arg(params, 1, rows).min(rows);
        if top == 1 && bottom == rows {
            self.screen.reset_scroll_region();
            self.home_cursor();
        } else if top >= 1 && self.screen.set_scroll_region(top - 1, bottom - 1) {
            self.home_cursor();
        }
    }

    fn set_cursor_style(&mut self, ps: u32) {
        let (shape, blinking) = match ps {
            0 | 1 => (CursorShape::Block, true),
            2 => (CursorShape::Block, false),
            3 => (CursorShape::Underline, true),
            4 => (CursorShape::Underline, false),
            5 => (CursorShape::Bar, true),
            6 => (CursorShape::Bar, false),
            _ => return,
        };
        self.screen.cursor_shape = shape;
        self.screen.cursor_blinking = blinking;
    }

    fn save_cursor(&mut self) {
        self.screen.save_cursor();
        self.saved_charsets = Some((self.charsets, self.gl));
    }

    fn restore_cursor(&mut self) {
        self.screen.restore_cursor();
        let (charsets, gl) = self.saved_charsets.unwrap_or(([Charset::Ascii; 4], 0));
        self.charsets = charsets;
        self.gl = gl;
        if self.modes.origin {
            // 原点モードでは、カーソルはスクロール範囲の外に出ない（xterm.js の _restrictCursor）。
            let (top, bottom) = self.screen.scroll_region();
            let c = self.screen.cursor();
            self.screen.set_cursor(c.x, c.y.clamp(top, bottom));
        }
    }

    /// 捨てた画像の配置を消し、画像のコマンドが頼んだ仕事（置く、消す）をする。
    fn apply_graphics(&mut self) {
        self.drop_removed_images();
        for effect in self.graphics.take_effects() {
            match effect {
                Effect::Place {
                    mut placement,
                    move_cursor,
                } => {
                    let c = self.screen.cursor();
                    let (cols, rows) = (placement.cols as usize, placement.rows as usize);
                    placement.col = c.x;
                    self.screen.add_placement(placement);
                    if move_cursor {
                        for _ in 1..rows {
                            self.screen.index();
                        }
                        let y = self.screen.cursor().y;
                        self.screen.set_cursor(c.x.saturating_add(cols), y);
                    }
                }
                Effect::Delete(spec) => self.delete_placements(spec),
            }
            self.drop_removed_images();
        }
    }

    fn drop_removed_images(&mut self) {
        let removed = self.graphics.take_removed();
        if !removed.is_empty() {
            self.screen.drop_image_placements(&removed);
        }
    }

    /// `a=d`。
    fn delete_placements(&mut self, spec: DeleteSpec) {
        let rows = self.screen.rows() as i64;
        let c = self.screen.cursor();
        // 見えている画面にかかるか。
        let visible = |row: i64, p: &Placement| row + i64::from(p.rows) > 0 && row < rows;
        // 画面のセル (x, y)（0 始まり）にかかるか。
        let covers = |row: i64, p: &Placement, x: usize, y: usize| {
            let (y, r) = (y as i64, i64::from(p.rows));
            row <= y && y < row + r && p.col <= x && x < p.col + p.cols as usize
        };
        // 1 始まりの値を 0 始まりに（0 は 1 と同じ）。
        let cell = |v: u32| (v.max(1) - 1) as usize;
        let hit = |row: i64, p: &Placement| match spec.target {
            DeleteTarget::All => visible(row, p),
            DeleteTarget::Image { id, placement } => {
                p.image_id == id && (placement == 0 || p.placement_id == placement)
            }
            DeleteTarget::Cursor => visible(row, p) && covers(row, p, c.x, c.y),
            DeleteTarget::Cell { x, y } => visible(row, p) && covers(row, p, cell(x), cell(y)),
            DeleteTarget::CellZ { x, y, z } => {
                visible(row, p) && p.z == z && covers(row, p, cell(x), cell(y))
            }
            DeleteTarget::Column(x) => {
                let x = cell(x);
                visible(row, p) && p.col <= x && x < p.col + p.cols as usize
            }
            DeleteTarget::Row(y) => {
                let y = cell(y) as i64;
                visible(row, p) && row <= y && y < row + i64::from(p.rows)
            }
            DeleteTarget::Z(z) => visible(row, p) && p.z == z,
        };
        let mut ids = self.screen.retain_placements(|row, p| !hit(row, p));
        if let DeleteTarget::Image { id, .. } = spec.target {
            ids.push(id);
        }
        if spec.free {
            ids.sort_unstable();
            ids.dedup();
            for id in ids {
                if !self.screen.image_is_placed(id) {
                    self.graphics.remove(id);
                }
            }
        }
    }

    fn full_reset(&mut self) {
        self.screen.reset();
        self.charsets = [Charset::Ascii; 4];
        self.gl = 0;
        self.saved_charsets = None;
        self.last_char = None;
        self.modes = Modes::default();
        self.keyboard_flags = Default::default();
        self.graphics.clear();
        self.notifications.clear();
    }

    /// 主画面と代替画面を切り替える。カーソルの位置は引き継ぐ（xterm.js と同じ）。
    fn switch_screen(&mut self, alt: bool) {
        if self.screen.is_alt_screen() == alt {
            return;
        }
        let c = self.screen.cursor();
        if alt {
            self.screen.enter_alt_screen();
        } else {
            self.screen.leave_alt_screen();
        }
        self.screen.set_cursor(c.x, c.y);
    }

    /// SM・RM（`private` が偽）と DECSET・DECRST（真）。
    fn set_mode(&mut self, private: bool, mode: u32, on: bool) {
        if !private {
            match mode {
                4 => self.screen.insert_mode = on,
                20 => self.modes.linefeed_newline = on,
                _ => {}
            }
            return;
        }
        match mode {
            1 => self.modes.application_cursor_keys = on,
            6 => {
                self.modes.origin = on;
                self.home_cursor();
            }
            7 => self.screen.wraparound = on,
            9 => self.set_mouse_protocol(MouseProtocol::X10, on),
            12 => self.screen.cursor_blinking = on,
            25 => self.screen.cursor_visible = on,
            45 => self.modes.reverse_wraparound = on,
            47 | 1047 => self.switch_screen(on),
            1048 => {
                if on {
                    self.save_cursor();
                } else {
                    self.restore_cursor();
                }
            }
            1049 => {
                if on {
                    if !self.screen.is_alt_screen() {
                        self.save_cursor();
                    }
                    self.switch_screen(true);
                } else if self.screen.is_alt_screen() {
                    self.switch_screen(false);
                    self.restore_cursor();
                }
            }
            66 => self.modes.application_keypad = on,
            1000 => self.set_mouse_protocol(MouseProtocol::Vt200, on),
            1002 => self.set_mouse_protocol(MouseProtocol::Drag, on),
            1003 => self.set_mouse_protocol(MouseProtocol::Any, on),
            1004 => self.modes.send_focus = on,
            1005 => self.set_mouse_encoding(MouseEncoding::Utf8, on),
            1006 => self.set_mouse_encoding(MouseEncoding::Sgr, on),
            1015 => self.set_mouse_encoding(MouseEncoding::Urxvt, on),
            1016 => self.set_mouse_encoding(MouseEncoding::SgrPixels, on),
            2004 => self.modes.bracketed_paste = on,
            2026 => self.modes.synchronized_output = on,
            _ => {}
        }
    }

    fn set_mouse_protocol(&mut self, p: MouseProtocol, on: bool) {
        self.modes.mouse_protocol = if on { p } else { MouseProtocol::None };
    }

    fn set_mouse_encoding(&mut self, e: MouseEncoding, on: bool) {
        self.modes.mouse_encoding = if on { e } else { MouseEncoding::Default };
    }

    /// DECRQM の Ps。0 は知らないモード、1 は設定中、2 は解除中。
    fn mode_status(&self, private: bool, mode: u32) -> u8 {
        let m = &self.modes;
        let v = if !private {
            match mode {
                4 => self.screen.insert_mode,
                20 => m.linefeed_newline,
                _ => return 0,
            }
        } else {
            match mode {
                1 => m.application_cursor_keys,
                6 => m.origin,
                7 => self.screen.wraparound,
                9 => m.mouse_protocol == MouseProtocol::X10,
                12 => self.screen.cursor_blinking,
                25 => self.screen.cursor_visible,
                45 => m.reverse_wraparound,
                47 | 1047 | 1049 => self.screen.is_alt_screen(),
                // 1048 は保存の操作で、状態ではない。
                66 => m.application_keypad,
                1000 => m.mouse_protocol == MouseProtocol::Vt200,
                1002 => m.mouse_protocol == MouseProtocol::Drag,
                1003 => m.mouse_protocol == MouseProtocol::Any,
                1004 => m.send_focus,
                1005 => m.mouse_encoding == MouseEncoding::Utf8,
                1006 => m.mouse_encoding == MouseEncoding::Sgr,
                1015 => m.mouse_encoding == MouseEncoding::Urxvt,
                1016 => m.mouse_encoding == MouseEncoding::SgrPixels,
                2004 => m.bracketed_paste,
                2026 => m.synchronized_output,
                _ => return 0,
            }
        };
        if v {
            1
        } else {
            2
        }
    }

    /// SM・RM・DECSET・DECRST・DA・DSR・DECRQM。扱ったら真。
    fn mode_or_query(
        &mut self,
        params: &Params,
        intermediates: &[u8],
        marker: Option<u8>,
        action: u8,
    ) -> bool {
        let first = raw(params, 0).unwrap_or(0);
        match (marker, intermediates, action) {
            (None | Some(b'?'), [], b'h' | b'l') => {
                for p in params {
                    self.set_mode(marker.is_some(), p[0], action == b'h');
                }
            }
            (None, [], b'c') if first == 0 => self.reply("\x1b[?1;2c"),
            (Some(b'>'), [], b'c') if first == 0 => self.reply("\x1b[>0;276;0c"),
            (None, [], b'n') => match first {
                5 => self.reply("\x1b[0n"),
                6 => self.report_cursor(false),
                _ => {}
            },
            (Some(b'?'), [], b'n') if first == 6 => self.report_cursor(true),
            (None | Some(b'?'), [b'$'], b'p') => {
                let private = marker.is_some();
                let status = self.mode_status(private, first);
                let q = if private { "?" } else { "" };
                self.reply(&format!("\x1b[{q}{first};{status}$y"));
            }
            (Some(b'>'), [], b'u') => self.active_keyboard_flags_mut().push(first),
            (Some(b'<'), [], b'u') => {
                let n = if first == 0 { 1 } else { first as usize };
                self.active_keyboard_flags_mut().pop(n);
            }
            (Some(b'='), [], b'u') => {
                let mode = raw(params, 1).filter(|&m| m != 0).unwrap_or(1);
                self.active_keyboard_flags_mut().modify(first, mode);
            }
            (Some(b'?'), [], b'u') => {
                let flags = self.active_keyboard_flags().flags();
                self.reply(&format!("\x1b[?{flags}u"));
            }
            _ => return false,
        }
        true
    }

    fn active_keyboard_flags(&self) -> &KeyboardFlagStack {
        &self.keyboard_flags[usize::from(self.screen.is_alt_screen())]
    }

    fn active_keyboard_flags_mut(&mut self) -> &mut KeyboardFlagStack {
        &mut self.keyboard_flags[usize::from(self.screen.is_alt_screen())]
    }

    /// CPR。原点モードでは、スクロール範囲の上端からの行を返す。
    fn report_cursor(&mut self, private: bool) {
        let c = self.screen.cursor();
        let top = if self.modes.origin {
            self.screen.scroll_region().0
        } else {
            0
        };
        let q = if private { "?" } else { "" };
        self.reply(&format!(
            "\x1b[{q}{};{}R",
            c.y.saturating_sub(top) + 1,
            c.x + 1
        ));
    }

    fn reply_color(&mut self, ps: &str, color: Rgb, bell: bool) {
        let st = if bell { "\x07" } else { "\x1b\\" };
        let rgb = theme::to_xparse_color(color);
        self.reply(&format!("\x1b]{ps};{rgb}{st}"));
    }

    /// 色の問い合わせ。`?` の指定にだけ返事し、色を変える指定は無視する。
    fn osc_color_query(&mut self, cmd: &[u8], rest: &str, bell: bool) {
        if cmd == b"4" {
            let mut it = rest.split(';');
            while let (Some(idx), Some(spec)) = (it.next(), it.next()) {
                if let (Ok(i), "?") = (idx.parse::<u8>(), spec) {
                    let color = self.theme.palette(i);
                    self.reply_color(&format!("4;{i}"), color, bell);
                }
            }
            return;
        }
        // 10・11・12。続く指定は、次の番号（11・12）に当たる。
        let first: u8 = match cmd {
            b"10" => 10,
            b"11" => 11,
            _ => 12,
        };
        for (k, spec) in rest.split(';').enumerate() {
            let ps = first + k as u8;
            if ps > 12 {
                break;
            }
            if spec == "?" {
                let color = match ps {
                    10 => self.theme.foreground,
                    11 => self.theme.background,
                    _ => self.theme.cursor,
                };
                self.reply_color(&ps.to_string(), color, bell);
            }
        }
    }

    fn osc(&mut self, data: &[u8], bell: bool) {
        let end = data.iter().position(|&b| b == b';').unwrap_or(data.len());
        if let Some(ident) = std::str::from_utf8(&data[..end])
            .ok()
            .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|i| self.osc_handlers.contains(i))
        {
            let rest = data.get(end + 1..).unwrap_or(&[]).to_vec();
            self.events.push(Event::Osc { ident, data: rest });
            return;
        }
        let Some(sep) = data.iter().position(|&b| b == b';') else {
            return;
        };
        let text = || String::from_utf8_lossy(&data[sep + 1..]).into_owned();
        match &data[..sep] {
            b"0" => {
                self.events.push(Event::IconName(text()));
                self.events.push(Event::Title(text()));
            }
            b"1" => self.events.push(Event::IconName(text())),
            b"2" => self.events.push(Event::Title(text())),
            b"99" => {
                use notification::Output;
                let ev = match self.notifications.osc(&data[sep + 1..]) {
                    Some(Output::Notification(n)) => Event::Notification(n),
                    Some(Output::Close(id)) => Event::NotificationClose { id },
                    Some(Output::Query(id)) => Event::NotificationQuery { id },
                    Some(Output::Alive(id)) => Event::NotificationAlive { id },
                    None => return,
                };
                self.events.push(ev);
            }
            cmd @ (b"4" | b"10" | b"11" | b"12") => {
                let rest = text();
                self.osc_color_query(cmd, &rest, bell);
            }
            _ => {}
        }
    }
}

impl Perform for State {
    fn print(&mut self, c: char) {
        let c = if c.is_ascii() {
            self.charsets[self.gl].map(c)
        } else {
            c
        };
        self.put(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x07 => self.events.push(Event::Bell),
            0x08 => self.backspace(),
            0x09 => self.screen.tab(),
            0x0A..=0x0C => {
                self.screen.index();
                if self.modes.linefeed_newline {
                    self.screen.carriage_return();
                }
            }
            0x0D => self.screen.carriage_return(),
            0x0E => self.gl = 1,
            0x0F => self.gl = 0,
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], byte: u8) {
        match (intermediates, byte) {
            ([], b'7') => self.save_cursor(),
            ([], b'8') => self.restore_cursor(),
            ([], b'D') => self.screen.index(),
            ([], b'E') => {
                self.screen.carriage_return();
                self.screen.index();
            }
            ([], b'H') => self.screen.set_tab_stop(),
            ([], b'M') => self.screen.reverse_index(),
            ([], b'c') => self.full_reset(),
            ([], b'=') => self.modes.application_keypad = true,
            ([], b'>') => self.modes.application_keypad = false,
            ([], b'n') => self.gl = 2,
            ([], b'o') => self.gl = 3,
            ([b'#'], b'8') => self.screen.fill_with('E'),
            ([d @ (b'(' | b')' | b'*' | b'+')], f) => {
                let slot = match d {
                    b'(' => 0,
                    b')' => 1,
                    b'*' => 2,
                    _ => 3,
                };
                self.charsets[slot] = Charset::from_final(f);
            }
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &Params,
        intermediates: &[u8],
        marker: Option<u8>,
        action: u8,
    ) {
        if self.mode_or_query(params, intermediates, marker, action) {
            return;
        }
        if marker.is_some() {
            return;
        }
        if intermediates == b" " {
            if action == b'q' {
                self.set_cursor_style(raw(params, 0).unwrap_or(0));
            }
            return;
        }
        if !intermediates.is_empty() {
            return;
        }
        let n = arg(params, 0, 1);
        let c = self.screen.cursor();
        match action {
            b'A' => self.cursor_up(n),
            b'B' | b'e' => self.cursor_down(n),
            b'C' | b'a' => self.cursor_forward(n),
            b'D' => self.cursor_backward(n),
            b'E' => {
                self.cursor_down(n);
                self.screen.carriage_return();
            }
            b'F' => {
                self.cursor_up(n);
                self.screen.carriage_return();
            }
            b'G' | b'`' => self.move_cursor_to(n - 1, c.y),
            b'H' | b'f' => self.move_cursor_to(arg(params, 1, 1) - 1, self.row_from_param(n)),
            b'd' => self.move_cursor_to(c.x, self.row_from_param(n)),
            b'J' => self.erase_in_display(raw(params, 0).unwrap_or(0)),
            b'K' => self.erase_in_line(raw(params, 0).unwrap_or(0)),
            b'X' => self.screen.erase_cells(c.y, c.x, c.x.saturating_add(n)),
            b'@' => self.screen.insert_cells(n),
            b'P' => self.screen.delete_cells(n),
            b'L' => self.screen.insert_lines(n),
            b'M' => self.screen.delete_lines(n),
            b'S' if params.len() <= 1 => self.screen.scroll_up_discard(n),
            b'T' if params.len() <= 1 => self.screen.scroll_down(n),
            b'r' => self.set_scroll_region(params),
            b'g' => match raw(params, 0).unwrap_or(0) {
                0 => self.screen.clear_tab_stop(),
                3 => self.screen.clear_all_tab_stops(),
                _ => {}
            },
            b'I' => {
                for _ in 0..n.min(self.screen.cols()) {
                    self.screen.tab();
                }
            }
            b'Z' => {
                for _ in 0..n.min(self.screen.cols()) {
                    self.screen.back_tab();
                }
            }
            b'b' => {
                if let Some(ch) = self.last_char {
                    for _ in 0..n.min(MAX_REPEAT) {
                        self.screen.print(ch);
                    }
                }
            }
            b'm' => {
                let mut pen = self.screen.pen;
                sgr::apply(&mut pen, params);
                self.screen.pen = pen;
            }
            b's' if params.is_empty() => self.save_cursor(),
            b'u' if params.is_empty() => self.restore_cursor(),
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, data: &[u8], bell_terminated: bool) {
        self.osc(data, bell_terminated);
    }

    fn apc_dispatch(&mut self, data: &[u8]) {
        self.graphics.apc(data, &mut self.output);
        self.apply_graphics();
    }
}
