//! ターミナル本体。バイト列を受け取り、パーサー（[`crate::parser`]）が分けたシーケンスで
//! 画面（[`crate::screen`]）を動かす。
//!
//! 対応するシーケンスは xterm.js 6.0.0 の対応表で Supported のもののうち、DEC のモードと
//! 問い合わせへの返事（#5）、OSC 8（リンク）を除いたもの。知らないシーケンスは捨てる。
//!
//! # 外へ出すもの（[`Event`]）
//!
//! BEL と題名の変更は [`Event`] になって溜まる。[`Terminal::take_events`] で取り出す。
//! 呼ぶ側が取り出さないと溜まり続けるので、`write` のたびに取り出すこと。
//!
//! # 画面の状態のうち、ここで持つもの
//!
//! 文字集合（G0〜G3 と GL）、直前に書いた字（REP 用）、DECSC で保存した文字集合、
//! DECKPAM/DECKPNM（`ESC =` / `ESC >`）の状態。

mod sgr;
#[cfg(test)]
mod tests;

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

/// ターミナル。`write` にバイト列を渡すと、画面が変わる。
#[derive(Debug)]
pub struct Terminal {
    parser: Parser,
    state: State,
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
    application_keypad: bool,
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
            state: State {
                screen,
                events: Vec::new(),
                charsets: [Charset::Ascii; 4],
                gl: 0,
                saved_charsets: None,
                last_char: None,
                application_keypad: false,
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

    /// 大きさを変える。
    pub fn resize(&mut self, rows: usize, cols: usize) {
        self.state.screen.resize(rows, cols);
    }

    /// 溜まったイベントを古い順に取り出す。
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.state.events)
    }

    /// アプリケーションキーパッドか（`ESC =` で真、`ESC >` で偽）。
    pub fn application_keypad(&self) -> bool {
        self.state.application_keypad
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
            self.screen.set_cursor(0, 0);
        } else if top >= 1 {
            self.screen.set_scroll_region(top - 1, bottom - 1);
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
    }

    fn full_reset(&mut self) {
        self.screen.reset();
        self.charsets = [Charset::Ascii; 4];
        self.gl = 0;
        self.saved_charsets = None;
        self.last_char = None;
        self.application_keypad = false;
    }

    fn osc(&mut self, data: &[u8]) {
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
            0x08 => self.screen.backspace(),
            0x09 => self.screen.tab(),
            0x0A..=0x0C => self.screen.index(),
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
            ([], b'=') => self.application_keypad = true,
            ([], b'>') => self.application_keypad = false,
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
            b'H' | b'f' => self.move_cursor_to(arg(params, 1, 1) - 1, n - 1),
            b'd' => self.move_cursor_to(c.x, n - 1),
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

    fn osc_dispatch(&mut self, data: &[u8], _bell_terminated: bool) {
        self.osc(data);
    }
}
