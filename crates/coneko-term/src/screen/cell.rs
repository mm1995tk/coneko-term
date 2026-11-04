//! セルと、セルに付く属性。

/// 色。`Palette(0..=15)` が 16 色、`Palette(16..=255)` が 256 色の残り。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Color {
    #[default]
    Default,
    Palette(u8),
    Rgb(u8, u8, u8),
}

/// 下線の形。波線などは kitty の下線の拡張で使う。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnderlineStyle {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

/// 下線。形と色（`Color::Default` は前景色と同じ）を持つ。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Underline {
    pub style: UnderlineStyle,
    pub color: Color,
}

/// 二値の属性のビット集合。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttrFlags(u16);

impl AttrFlags {
    pub const BOLD: AttrFlags = AttrFlags(1 << 0);
    pub const DIM: AttrFlags = AttrFlags(1 << 1);
    pub const ITALIC: AttrFlags = AttrFlags(1 << 2);
    pub const BLINK: AttrFlags = AttrFlags(1 << 3);
    pub const INVERSE: AttrFlags = AttrFlags(1 << 4);
    pub const INVISIBLE: AttrFlags = AttrFlags(1 << 5);
    pub const STRIKETHROUGH: AttrFlags = AttrFlags(1 << 6);

    pub fn contains(self, other: AttrFlags) -> bool {
        self.0 & other.0 == other.0
    }
    pub fn insert(&mut self, other: AttrFlags) {
        self.0 |= other.0;
    }
    pub fn remove(&mut self, other: AttrFlags) {
        self.0 &= !other.0;
    }
}

/// セルの属性。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attrs {
    pub fg: Color,
    pub bg: Color,
    pub flags: AttrFlags,
    pub underline: Underline,
}

/// セルの中身。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Content {
    /// 何も書かれていない。
    #[default]
    Empty,
    /// 1 つの字。
    Char(char),
    /// 結合文字などを含む書記素（2 字以上）。
    Cluster(Box<str>),
    /// 全角の字の右半分。字そのものは左のセルが持つ。
    WideTail,
}

/// 画面の 1 マス。
///
/// `width` は、字が使うセルの数（1 か 2）。全角の字は左のセルが `width == 2` で字を持ち、
/// 右のセルは `Content::WideTail`（`width == 0`）になる。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub content: Content,
    pub width: u8,
    pub attrs: Attrs,
}

impl Default for Cell {
    fn default() -> Self {
        Cell::blank(Attrs::default())
    }
}

impl Cell {
    /// 空のセル。
    pub fn blank(attrs: Attrs) -> Cell {
        Cell {
            content: Content::Empty,
            width: 1,
            attrs,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.content == Content::Empty
    }

    pub fn is_wide_tail(&self) -> bool {
        self.content == Content::WideTail
    }

    /// セルの字。空や全角の右半分は空文字列。
    pub fn text(&self) -> String {
        match &self.content {
            Content::Char(c) => c.to_string(),
            Content::Cluster(s) => s.to_string(),
            Content::Empty | Content::WideTail => String::new(),
        }
    }

    pub(crate) fn push_char(&mut self, c: char) {
        self.content = match std::mem::take(&mut self.content) {
            Content::Char(first) => {
                let mut s = String::from(first);
                s.push(c);
                Content::Cluster(s.into_boxed_str())
            }
            Content::Cluster(s) => {
                let mut s = s.into_string();
                s.push(c);
                Content::Cluster(s.into_boxed_str())
            }
            other => other,
        };
    }
}
