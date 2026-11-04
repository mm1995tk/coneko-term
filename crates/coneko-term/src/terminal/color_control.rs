//! kitty の color control（OSC 21）と色の積み重ね（XTPUSHCOLORS・XTPOPCOLORS・XTREPORTCOLORS・
//! OSC 30001・OSC 30101）、OSC 4・10・11・12 で書く色の読み方。
//!
//! 色の表（[`Colors`]）は 1 つで、OSC 4・10・11・12 と OSC 21 のどちらで書いても、もう一方から読める。
//! 既定の色は [`Theme`] が持つ。

use super::theme::{Rgb, Theme};

/// いまの色。描画はこれを使う。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Colors {
    /// 文字の色（`foreground`、OSC 10）。
    pub foreground: Rgb,
    /// 背景の色（`background`、OSC 11）。
    pub background: Rgb,
    /// カーソルの色（`cursor`、OSC 12）。
    pub cursor: Rgb,
    /// カーソルの下の字の色（`cursor_text`）。`None` は動的（描画の既定に従う）。
    pub cursor_text: Option<Rgb>,
    /// 選択範囲の背景（`selection_background`）。`None` は動的。
    pub selection_background: Option<Rgb>,
    /// 選択範囲の字の色（`selection_foreground`）。`None` は動的。
    pub selection_foreground: Option<Rgb>,
    /// ビジュアルベルの色（`visual_bell`）。`None` は動的。
    pub visual_bell: Option<Rgb>,
    /// `transparent_background_color1` から `7`。`None` は未設定。不透明度は持たない。
    pub transparent: [Option<Rgb>; 7],
    /// 256 色の表。
    pub palette: [Rgb; 256],
}

impl Colors {
    /// テーマの色から作る。
    pub fn from_theme(theme: &Theme) -> Colors {
        let mut palette = [[0; 3]; 256];
        for (i, c) in palette.iter_mut().enumerate() {
            *c = theme.palette(i as u8);
        }
        Colors {
            foreground: theme.foreground,
            background: theme.background,
            cursor: theme.cursor,
            cursor_text: None,
            selection_background: None,
            selection_foreground: None,
            visual_bell: None,
            transparent: [None; 7],
            palette,
        }
    }
}

#[derive(Clone, Copy)]
enum Key {
    Foreground,
    Background,
    SelectionBackground,
    SelectionForeground,
    Cursor,
    CursorText,
    VisualBell,
    Transparent(usize),
    Palette(usize),
}

fn parse_key(k: &str) -> Option<Key> {
    Some(match k {
        "foreground" => Key::Foreground,
        "background" => Key::Background,
        "selection_background" => Key::SelectionBackground,
        "selection_foreground" => Key::SelectionForeground,
        "cursor" => Key::Cursor,
        "cursor_text" => Key::CursorText,
        "visual_bell" => Key::VisualBell,
        _ => {
            if let Some(n) = k.strip_prefix("transparent_background_color") {
                let n: usize = n.parse().ok().filter(|n| (1..=7).contains(n))?;
                return Some(Key::Transparent(n - 1));
            }
            if k.bytes().all(|b| b.is_ascii_digit()) && !k.is_empty() {
                return Some(Key::Palette(k.parse().ok().filter(|&n| n < 256)?));
            }
            return None;
        }
    })
}

impl Colors {
    fn get(&self, key: Key) -> Option<Rgb> {
        match key {
            Key::Foreground => Some(self.foreground),
            Key::Background => Some(self.background),
            Key::Cursor => Some(self.cursor),
            Key::Palette(i) => Some(self.palette[i]),
            Key::CursorText => self.cursor_text,
            Key::SelectionBackground => self.selection_background,
            Key::SelectionForeground => self.selection_foreground,
            Key::VisualBell => self.visual_bell,
            Key::Transparent(i) => self.transparent[i],
        }
    }

    /// 色を決める。`None`（動的）にできないキーへの `None` は無視する。
    fn set(&mut self, key: Key, v: Option<Rgb>) {
        match (key, v) {
            (Key::Foreground, Some(c)) => self.foreground = c,
            (Key::Background, Some(c)) => self.background = c,
            (Key::Cursor, Some(c)) => self.cursor = c,
            (Key::Palette(i), Some(c)) => self.palette[i] = c,
            (Key::CursorText, v) => self.cursor_text = v,
            (Key::SelectionBackground, v) => self.selection_background = v,
            (Key::SelectionForeground, v) => self.selection_foreground = v,
            (Key::VisualBell, v) => self.visual_bell = v,
            (Key::Transparent(i), v) => self.transparent[i] = v,
            _ => {}
        }
    }
}

/// OSC 21 の本体（`21;` より後ろ）を処理する。問い合わせへの返事（`21;` より後ろ）があれば返す。
pub(super) fn osc21(colors: &mut Colors, defaults: &Colors, data: &str) -> Option<String> {
    let mut reply: Vec<String> = Vec::new();
    for item in data.split(';') {
        if item.is_empty() {
            continue;
        }
        let (name, value) = match item.split_once('=') {
            Some((n, v)) => (n, Some(v)),
            None => (item, None),
        };
        let Some(key) = parse_key(name) else {
            reply.push(format!("unknown={}", base64_unpadded(name.as_bytes())));
            continue;
        };
        match value {
            None => colors.set(key, defaults.get(key)),
            Some("?") => {
                let v = colors.get(key).map(to_osc21_color).unwrap_or_default();
                reply.push(format!("{name}={v}"));
            }
            Some("") => colors.set(key, None),
            Some(v) => {
                if let Some(c) = parse_color(v) {
                    colors.set(key, Some(c));
                }
            }
        }
    }
    (!reply.is_empty()).then(|| reply.join(";"))
}

/// OSC 21 の返事の色（`rgb:rr/gg/bb`）。
fn to_osc21_color(c: Rgb) -> String {
    format!("rgb:{:02x}/{:02x}/{:02x}", c[0], c[1], c[2])
}

fn base64_unpadded(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |a, (i, &b)| a | (b as u32) << (16 - 8 * i));
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

/// 色の書き方（`rgb:`・`#`・`rgbi:`・名前。`@不透明度` は読み捨てる）を RGB にする。読めなければ `None`。
pub(super) fn parse_color(s: &str) -> Option<Rgb> {
    let s = match s.split_once('@') {
        Some((c, alpha)) => {
            alpha.parse::<f64>().ok()?;
            c
        }
        None => s,
    };
    let s = s.to_ascii_lowercase();
    if let Some(rest) = s.strip_prefix("rgb:") {
        let mut it = rest.split('/');
        let (r, g, b) = (it.next()?, it.next()?, it.next()?);
        if it.next().is_some() {
            return None;
        }
        return Some([scale_hex(r)?, scale_hex(g)?, scale_hex(b)?]);
    }
    if let Some(rest) = s.strip_prefix('#') {
        let n = rest.len();
        if n == 0 || n % 3 != 0 || n > 12 {
            return None;
        }
        let d = n / 3;
        return Some([
            scale_hex(&rest[..d])?,
            scale_hex(&rest[d..2 * d])?,
            scale_hex(&rest[2 * d..])?,
        ]);
    }
    if let Some(rest) = s.strip_prefix("rgbi:") {
        let mut it = rest.split('/');
        let mut out = [0u8; 3];
        for o in &mut out {
            let v: f64 = it.next()?.parse().ok()?;
            if !(0.0..=1.0).contains(&v) {
                return None;
            }
            *o = (v * 255.0).round() as u8;
        }
        return it.next().is_none().then_some(out);
    }
    NAMES.iter().find(|(n, _)| *n == s).map(|(_, c)| *c)
}

/// 1 から 4 桁の 16 進数を 8 ビットにする。
fn scale_hex(s: &str) -> Option<u8> {
    if s.is_empty() || s.len() > 4 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    let max = (1u32 << (4 * s.len())) - 1;
    Some(((v * 255 + max / 2) / max) as u8)
}

/// 受け付ける色の名前（X11 の色の一部。大文字小文字は区別しない）。
const NAMES: &[(&str, Rgb)] = &[
    ("black", [0, 0, 0]),
    ("white", [255, 255, 255]),
    ("red", [255, 0, 0]),
    ("green", [0, 255, 0]),
    ("blue", [0, 0, 255]),
    ("yellow", [255, 255, 0]),
    ("cyan", [0, 255, 255]),
    ("magenta", [255, 0, 255]),
    ("gray", [190, 190, 190]),
    ("grey", [190, 190, 190]),
    ("orange", [255, 165, 0]),
    ("purple", [160, 32, 240]),
    ("brown", [165, 42, 42]),
    ("pink", [255, 192, 203]),
    ("navy", [0, 0, 128]),
    ("gold", [255, 215, 0]),
    ("maroon", [176, 48, 96]),
];

/// 積み重ねの段の数。
const SLOTS: usize = 10;

/// 色の積み重ね。`CSI # P` で積み、`CSI # Q` で戻す。
#[derive(Default, Debug)]
pub(super) struct ColorStack {
    slots: [Option<Box<Colors>>; SLOTS],
    /// 次に積む段。
    top: usize,
}

impl ColorStack {
    /// `n` が 0 なら積み重ねの上に積み（満杯なら一番古いものを捨てる）、1 から 10 ならその段に置く。
    pub fn push(&mut self, current: &Colors, n: usize) {
        match n {
            0 => {
                if self.top >= SLOTS {
                    self.slots.rotate_left(1);
                    self.top = SLOTS - 1;
                }
                self.slots[self.top] = Some(Box::new(current.clone()));
                self.top += 1;
            }
            1..=SLOTS => self.slots[n - 1] = Some(Box::new(current.clone())),
            _ => {}
        }
    }

    /// `n` が 0 なら一番上を取り出し、1 から 10 ならその段の色を返す（段は残す）。
    pub fn pop(&mut self, n: usize) -> Option<Colors> {
        match n {
            0 => {
                if self.top == 0 {
                    return None;
                }
                self.top -= 1;
                self.slots[self.top].take().map(|b| *b)
            }
            1..=SLOTS => self.slots[n - 1].as_deref().cloned(),
            _ => None,
        }
    }

    /// XTREPORTCOLORS の `(いまの段, 入っている数)`。
    pub fn report(&self) -> (usize, usize) {
        (self.top, self.slots.iter().flatten().count())
    }

    pub fn clear(&mut self) {
        *self = ColorStack::default();
    }
}
