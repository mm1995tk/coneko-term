//! coneko-term を WebAssembly にするための wasm-bindgen の層。中身は `coneko-term` に置く。
//!
//! 画面の中身は、WASM のメモリにあるバッファー（[`WasmTerminal::cells_ptr`]）に 1 行ずつ
//! 書き出す。TypeScript は `memory.buffer` から直接読む。

use coneko_term::mouse::{MouseAction, MouseButton, MouseEvent};
use coneko_term::screen::cell::{Cell, Color, Content, UnderlineStyle};
use coneko_term::screen::Line;
use coneko_term::selection::{self, Pos};
use coneko_term::terminal::Theme;
use coneko_term::{Event, Terminal};
use wasm_bindgen::prelude::*;

/// このライブラリの版を返す。
#[wasm_bindgen]
pub fn version() -> String {
    coneko_term::version().to_string()
}

/// セル 1 つが使う `u32` の数。1 つのセルは次の並び:
///
/// 0. 字。`0` は空、`CLUSTER_FLAG` つきは `cluster(番号)`、それ以外は Unicode のコードポイント。
/// 1. 字が使うセルの数（全角の右半分は 0）。
/// 2. 前景色。3. 背景色。色は上位 8 ビットが種類（0 既定、1 パレット、2 RGB）、下位 24 ビットが値。
/// 4. ビット 0 から 6 が属性（太字、薄字、斜体、点滅、反転、不可視、取り消し線）、
///    ビット 8 から 10 が下線の形（0 なし、1 1 本、2 2 本、3 波、4 点、5 破）。
/// 5. 下線の色。
pub const CELL_STRIDE: usize = 6;

/// クラスター（結合文字を含む書記素）のセルの字に立てるビット。残りのビットが `cluster(番号)` の番号。
pub const CLUSTER_FLAG: u32 = 0x8000_0000;

fn color_word(c: Color) -> u32 {
    match c {
        Color::Default => 0,
        Color::Palette(i) => (1 << 24) | i as u32,
        Color::Rgb(r, g, b) => (2 << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32,
        _ => 0,
    }
}

fn underline_code(s: UnderlineStyle) -> u32 {
    match s {
        UnderlineStyle::None => 0,
        UnderlineStyle::Single => 1,
        UnderlineStyle::Double => 2,
        UnderlineStyle::Curly => 3,
        UnderlineStyle::Dotted => 4,
        UnderlineStyle::Dashed => 5,
        _ => 1,
    }
}

/// ターミナルが外へ知らせること 1 つ。`kind` は 0 が BEL、1 が題名、2 がアイコン名、3 が OSC。
#[wasm_bindgen]
pub struct WasmEvent {
    kind: u8,
    ident: u32,
    data: Vec<u8>,
}

#[wasm_bindgen]
impl WasmEvent {
    /// 種類（0 が BEL、1 が題名、2 がアイコン名、3 が登録した OSC）。
    #[wasm_bindgen(getter)]
    pub fn kind(&self) -> u8 {
        self.kind
    }
    /// OSC の番号（種類が 3 のとき）。
    #[wasm_bindgen(getter)]
    pub fn ident(&self) -> u32 {
        self.ident
    }
    /// 中身（題名・アイコン名・OSC のバイト列。題名とアイコン名は UTF-8）。
    #[wasm_bindgen(getter)]
    pub fn data(&self) -> Vec<u8> {
        self.data.clone()
    }
}

/// ターミナル。
#[wasm_bindgen]
pub struct WasmTerminal {
    term: Terminal,
    cells: Vec<u32>,
    clusters: Vec<String>,
    events: std::collections::VecDeque<Event>,
    last_wrapped: bool,
}

#[wasm_bindgen]
impl WasmTerminal {
    /// `rows` 行 `cols` 列で、スクロールバックが `scrollback` 行のターミナル。
    #[wasm_bindgen(constructor)]
    pub fn new(rows: usize, cols: usize, scrollback: usize) -> WasmTerminal {
        let screen = coneko_term::screen::Screen::with_scrollback(rows, cols, scrollback);
        WasmTerminal {
            term: Terminal::with_screen(screen),
            cells: Vec::new(),
            clusters: Vec::new(),
            events: Default::default(),
            last_wrapped: false,
        }
    }

    /// バイト列を書く。
    pub fn write(&mut self, bytes: &[u8]) {
        self.term.write(bytes);
        self.events.extend(self.term.take_events());
    }

    /// 初期状態に戻す。
    pub fn reset(&mut self) {
        self.term.reset();
    }

    /// 大きさを変える。
    pub fn resize(&mut self, rows: usize, cols: usize) {
        self.term.resize(rows.max(1), cols.max(1));
    }

    /// 行の数。
    pub fn rows(&self) -> usize {
        self.term.screen().rows()
    }

    /// 列の数。
    pub fn cols(&self) -> usize {
        self.term.screen().cols()
    }

    /// スクロールバックの行の数。
    pub fn scrollback_len(&self) -> usize {
        self.term.screen().scrollback_len()
    }

    /// 次のイベントを取り出す。無ければ `undefined`。
    pub fn next_event(&mut self) -> Option<WasmEvent> {
        self.events.pop_front().map(|e| match e {
            Event::Bell => WasmEvent {
                kind: 0,
                ident: 0,
                data: Vec::new(),
            },
            Event::Title(s) => WasmEvent {
                kind: 1,
                ident: 0,
                data: s.into_bytes(),
            },
            Event::IconName(s) => WasmEvent {
                kind: 2,
                ident: 0,
                data: s.into_bytes(),
            },
            Event::Osc { ident, data } => WasmEvent {
                kind: 3,
                ident,
                data,
            },
            _ => WasmEvent {
                kind: 255,
                ident: 0,
                data: Vec::new(),
            },
        })
    }

    /// PTY へ送るバイト列を取り出す。
    pub fn take_output(&mut self) -> Vec<u8> {
        self.term.take_output()
    }

    /// PTY へ送るバイト列を、返事と同じ出口に入れる（キーボードやマウス用）。
    pub fn queue_output(&mut self, bytes: &[u8]) {
        self.term.queue_output(bytes);
    }

    /// 色の問い合わせに返すテーマを変える。`ansi` は 16 色ぶんの RGB（48 バイト）。
    pub fn set_theme(&mut self, foreground: &[u8], background: &[u8], cursor: &[u8], ansi: &[u8]) {
        let rgb = |b: &[u8]| [b[0], b[1], b[2]];
        let mut theme = Theme {
            foreground: rgb(foreground),
            background: rgb(background),
            cursor: rgb(cursor),
            ..Theme::default()
        };
        for i in 0..16 {
            theme.ansi[i] = rgb(&ansi[i * 3..]);
        }
        self.term.set_theme(theme);
    }

    /// ウィンドウのフォーカスの出入りを知らせる。
    pub fn set_focus(&mut self, focused: bool) {
        self.term.set_focus(focused);
    }

    /// OSC の番号 `ident` を呼ぶ側で拾う。
    pub fn register_osc(&mut self, ident: u32) {
        self.term.register_osc(ident);
    }

    /// OSC の番号の登録を外す。
    pub fn unregister_osc(&mut self, ident: u32) {
        self.term.unregister_osc(ident);
    }

    /// カーソルを点滅させるか（`DECSCUSR` で変わる）。
    pub fn set_cursor_blink(&mut self, blink: bool) {
        self.term.screen_mut().cursor_blinking = blink;
    }

    /// カーソルの状態。`[x, y, 見えるか, 点滅するか, 形（0 ブロック 1 下線 2 縦棒）]`。
    pub fn cursor(&self) -> Vec<u32> {
        use coneko_term::screen::CursorShape;
        let c = self.term.screen().cursor();
        vec![
            c.x as u32,
            c.y as u32,
            c.visible as u32,
            c.blinking as u32,
            match c.shape {
                CursorShape::Underline => 1,
                CursorShape::Bar => 2,
                _ => 0,
            },
        ]
    }

    /// マウスの報告が有効か。
    pub fn mouse_reporting(&self) -> bool {
        self.term.mouse_reporting()
    }

    /// マウスのイベントを報告する。報告したら `true`（バイト列は `take_output` に出る）。
    ///
    /// `button`: 0 左、1 真ん中、2 右、3 押していない、4 戻る、5 進む、6 ホイール上、
    /// 7 ホイール下、8 ホイール左、9 ホイール右。`action`: 0 押した、1 離した、2 動かした。
    /// `mods`: ビット 0 が Shift、1 が Alt、2 が Ctrl。`col` と `row` は 0 始まり。
    #[allow(clippy::too_many_arguments)]
    pub fn report_mouse(
        &mut self,
        button: u8,
        action: u8,
        col: usize,
        row: usize,
        x: u32,
        y: u32,
        mods: u8,
    ) -> bool {
        let button = match button {
            0 => MouseButton::Left,
            1 => MouseButton::Middle,
            2 => MouseButton::Right,
            4 => MouseButton::Back,
            5 => MouseButton::Forward,
            6 => MouseButton::WheelUp,
            7 => MouseButton::WheelDown,
            8 => MouseButton::WheelLeft,
            9 => MouseButton::WheelRight,
            _ => MouseButton::NoButton,
        };
        let action = match action {
            0 => MouseAction::Press,
            1 => MouseAction::Release,
            _ => MouseAction::Move,
        };
        let mut e = MouseEvent::new(button, action, col, row);
        e.x = x;
        e.y = y;
        e.shift = mods & 1 != 0;
        e.alt = mods & 2 != 0;
        e.ctrl = mods & 4 != 0;
        self.term.report_mouse(&e)
    }

    /// スクロールバックの上から捨てた行の数の合計（代替画面では 0）。選択の位置を覚えるのに使う。
    pub fn scrollback_trimmed(&self) -> f64 {
        self.term.screen().scrollback_trimmed() as f64
    }

    /// 行の並び（スクロールバック + 画面）の `row` 行 `col` 列の単語の範囲 `[開始の行, 開始の列, 終わりの行, 終わりの列]`（終わりは含まない）。
    pub fn word_range(&self, row: usize, col: usize, separators: &str) -> Vec<u32> {
        let (a, b) = selection::word_at(self.term.screen(), row, col, separators);
        vec![a.row as u32, a.col as u32, b.row as u32, b.col as u32]
    }

    /// `row` 行目を含む、折り返しでつながった行全体の範囲。並びは `word_range` と同じ。
    pub fn line_range(&self, row: usize) -> Vec<u32> {
        let (a, b) = selection::line_range(self.term.screen(), row);
        vec![a.row as u32, a.col as u32, b.row as u32, b.col as u32]
    }

    /// 範囲 `[開始, 終わり)` の字。折り返した行は改行を入れずにつなぐ。
    pub fn selection_text(&self, row0: usize, col0: usize, row1: usize, col1: usize) -> String {
        selection::text(
            self.term.screen(),
            Pos::new(row0, col0),
            Pos::new(row1, col1),
        )
    }

    /// 代替画面か。
    pub fn is_alt_screen(&self) -> bool {
        self.term.screen().is_alt_screen()
    }

    /// bracketed paste が有効か（`CSI ? 2004 h`）。
    pub fn bracketed_paste(&self) -> bool {
        self.term.modes().bracketed_paste
    }

    /// アプリケーションカーソルキーか（`CSI ? 1 h`）。
    pub fn application_cursor_keys(&self) -> bool {
        self.term.modes().application_cursor_keys
    }

    /// いまの画面の kitty keyboard protocol のフラグ（`CSI ? u` で答える値）。
    /// 1 が立っていれば disambiguate escape codes。
    pub fn keyboard_flags(&self) -> u32 {
        self.term.keyboard_flags()
    }

    /// 変わった行（見えている画面の行番号）を返し、「変わった」の印を消す。
    pub fn take_dirty_rows(&mut self) -> Vec<u32> {
        self.term
            .screen_mut()
            .take_dirty_rows()
            .into_iter()
            .map(|y| y as u32)
            .collect()
    }

    /// 見えている画面の `y` 行目を、セルのバッファーに書き出す。セルの数を返す。
    pub fn read_row(&mut self, y: usize) -> usize {
        if y >= self.term.screen().rows() {
            return self.fill(None);
        }
        let line = self.term.screen().row(y).clone();
        self.fill(Some(&line))
    }

    /// スクロールバックの `i` 行目（0 が一番古い）を、セルのバッファーに書き出す。
    pub fn read_scrollback_row(&mut self, i: usize) -> usize {
        if i >= self.term.screen().scrollback_len() {
            return self.fill(None);
        }
        let line = self.term.screen().scrollback_row(i).clone();
        self.fill(Some(&line))
    }

    /// 直前の `read_*` で書き出した行が、次の行へ折り返されているか。
    pub fn row_wrapped(&self) -> bool {
        self.last_wrapped
    }

    /// セルのバッファーの位置（WASM のメモリの中の byte offset）。
    pub fn cells_ptr(&self) -> *const u32 {
        self.cells.as_ptr()
    }

    /// クラスター（結合文字を含む書記素）の字。番号は、直前の `read_*` で書き出したセルのもの。
    pub fn cluster(&self, index: usize) -> String {
        self.clusters.get(index).cloned().unwrap_or_default()
    }
}

impl WasmTerminal {
    fn fill(&mut self, line: Option<&Line>) -> usize {
        self.cells.clear();
        self.clusters.clear();
        self.last_wrapped = line.is_some_and(|l| l.is_wrapped());
        let Some(line) = line else { return 0 };
        for cell in line.cells() {
            self.push_cell(cell);
        }
        line.cells().len()
    }

    fn push_cell(&mut self, cell: &Cell) {
        let cp = match &cell.content {
            Content::Empty | Content::WideTail => 0,
            Content::Char(c) => *c as u32,
            Content::Cluster(s) => {
                self.clusters.push(s.to_string());
                CLUSTER_FLAG | (self.clusters.len() as u32 - 1)
            }
            _ => 0,
        };
        let f = cell.attrs.flags;
        use coneko_term::screen::cell::AttrFlags as F;
        let bits = [
            F::BOLD,
            F::DIM,
            F::ITALIC,
            F::BLINK,
            F::INVERSE,
            F::INVISIBLE,
            F::STRIKETHROUGH,
        ]
        .iter()
        .enumerate()
        .fold(0u32, |acc, (i, b)| acc | ((f.contains(*b) as u32) << i));
        self.cells.extend_from_slice(&[
            cp,
            cell.width as u32,
            color_word(cell.attrs.fg),
            color_word(cell.attrs.bg),
            bits | (underline_code(cell.attrs.underline.style) << 8),
            color_word(cell.attrs.underline.color),
        ]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_core() {
        assert_eq!(super::version(), coneko_term::version());
    }

    #[test]
    fn rows_are_exported_to_buffer() {
        let mut t = WasmTerminal::new(2, 4, 10);
        t.write("a\u{1b}[1;31mあ".as_bytes());
        assert_eq!(t.take_dirty_rows(), vec![0, 1]);
        assert_eq!(t.read_row(0), 4);
        assert_eq!(t.cells[0], 'a' as u32);
        assert_eq!(t.cells[CELL_STRIDE], 'あ' as u32);
        assert_eq!(t.cells[CELL_STRIDE + 1], 2);
        assert_eq!(t.cells[CELL_STRIDE + 2], (1 << 24) | 1);
        assert_eq!(t.cells[CELL_STRIDE + 4] & 1, 1);
        assert_eq!(t.cells[2 * CELL_STRIDE + 1], 0);
        assert!(t.take_dirty_rows().is_empty());
    }
}
