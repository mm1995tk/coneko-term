//! 色の問い合わせ（OSC 4・10・11・12）に返す色。

/// RGB（8 ビットずつ）。
pub type Rgb = [u8; 3];

/// ターミナルのテーマ。色の問い合わせには、ここにある値で返す。
///
/// 既定値は xterm.js 6.0.0 の既定のテーマ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    /// 文字の色（OSC 10）。
    pub foreground: Rgb,
    /// 背景の色（OSC 11）。
    pub background: Rgb,
    /// カーソルの色（OSC 12）。
    pub cursor: Rgb,
    /// 16 色（OSC 4 の 0 から 15）。16 から 255 は 6x6x6 の立方体と 24 段階の灰色で決まる。
    pub ansi: [Rgb; 16],
}

impl Default for Theme {
    fn default() -> Theme {
        Theme {
            foreground: [0xff, 0xff, 0xff],
            background: [0x00, 0x00, 0x00],
            cursor: [0xff, 0xff, 0xff],
            ansi: [
                [0x2e, 0x34, 0x36],
                [0xcc, 0x00, 0x00],
                [0x4e, 0x9a, 0x06],
                [0xc4, 0xa0, 0x00],
                [0x34, 0x65, 0xa4],
                [0x75, 0x50, 0x7b],
                [0x06, 0x98, 0x9a],
                [0xd3, 0xd7, 0xcf],
                [0x55, 0x57, 0x53],
                [0xef, 0x29, 0x29],
                [0x8a, 0xe2, 0x34],
                [0xfc, 0xe9, 0x4f],
                [0x72, 0x9f, 0xcf],
                [0xad, 0x7f, 0xa8],
                [0x34, 0xe2, 0xe2],
                [0xee, 0xee, 0xec],
            ],
        }
    }
}

impl Theme {
    /// パレットの `index` 番目の色。
    pub fn palette(&self, index: u8) -> Rgb {
        const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        match index {
            0..=15 => self.ansi[index as usize],
            16..=231 => {
                let i = (index - 16) as usize;
                [LEVELS[i / 36], LEVELS[(i / 6) % 6], LEVELS[i % 6]]
            }
            _ => {
                let v = 8 + 10 * (index - 232);
                [v, v, v]
            }
        }
    }
}

/// `rgb:rrrr/gggg/bbbb`（16 ビットずつ。xterm.js と同じ書き方）。
pub(super) fn to_xparse_color(c: Rgb) -> String {
    format!(
        "rgb:{:04x}/{:04x}/{:04x}",
        c[0] as u16 * 0x101,
        c[1] as u16 * 0x101,
        c[2] as u16 * 0x101
    )
}
