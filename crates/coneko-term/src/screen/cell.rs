//! セルと、セルに付く属性。

use std::sync::Arc;

/// 色。`Palette(0..=15)` が 16 色、`Palette(16..=255)` が 256 色の残り。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Color {
    #[default]
    /// 既定の色。
    Default,
    /// パレットの色（0 から 15 が 16 色、16 から 255 が 256 色）。
    Palette(u8),
    /// RGB。
    Rgb(u8, u8, u8),
}

/// 下線の形。波線などは kitty の下線の拡張で使う。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnderlineStyle {
    #[default]
    /// 下線なし。
    None,
    /// 1 本線。
    Single,
    /// 2 本線。
    Double,
    /// 波線。
    Curly,
    /// 点線。
    Dotted,
    /// 破線。
    Dashed,
}

/// 下線。形と色（`Color::Default` は前景色と同じ）を持つ。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Underline {
    /// 形。
    pub style: UnderlineStyle,
    /// 色。
    pub color: Color,
}

/// 二値の属性のビット集合。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttrFlags(u16);

impl AttrFlags {
    /// 太字。
    pub const BOLD: AttrFlags = AttrFlags(1 << 0);
    /// 薄字。
    pub const DIM: AttrFlags = AttrFlags(1 << 1);
    /// 斜体。
    pub const ITALIC: AttrFlags = AttrFlags(1 << 2);
    /// 点滅。
    pub const BLINK: AttrFlags = AttrFlags(1 << 3);
    /// 反転。
    pub const INVERSE: AttrFlags = AttrFlags(1 << 4);
    /// 不可視。
    pub const INVISIBLE: AttrFlags = AttrFlags(1 << 5);
    /// 取り消し線。
    pub const STRIKETHROUGH: AttrFlags = AttrFlags(1 << 6);

    /// `other` のビットがすべて立っているか。
    pub fn contains(self, other: AttrFlags) -> bool {
        self.0 & other.0 == other.0
    }
    /// `other` のビットを立てる。
    pub fn insert(&mut self, other: AttrFlags) {
        self.0 |= other.0;
    }
    /// `other` のビットを下ろす。
    pub fn remove(&mut self, other: AttrFlags) {
        self.0 &= !other.0;
    }
}

/// セルの属性。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attrs {
    /// 前景色。
    pub fg: Color,
    /// 背景色。
    pub bg: Color,
    /// 二値の属性。
    pub flags: AttrFlags,
    /// 下線。
    pub underline: Underline,
}

/// 寄せ方（kitty の text sizing の `v`・`h`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Align {
    /// 上（縦）または左（横）。
    #[default]
    Start,
    /// 下（縦）または右（横）。
    End,
    /// 中央。
    Center,
}

/// 複数のセルにまたがる字（kitty の text sizing、OSC 66）。幅 `cols` セル、高さ `scale` セルの
/// 四角に収めて描く。四角の左上はカーソルがあったセルで、右と下へ広がる。
/// 四角の全部のセルがこの値を（`Arc` で）共有する。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SizedBlock {
    /// 字。`w` を決めたときは複数の字でもよい。
    pub text: Box<str>,
    /// 四角の幅（セルの数。`w * s`。`w` が 0 なら、字の幅 × `s`）。
    pub cols: u8,
    /// 大きさ `s`。四角の高さ（セルの数）でもある。
    pub scale: u8,
    /// 分数の大きさの分子。
    pub numerator: u8,
    /// 分数の大きさの分母。0 なら分数の大きさは無い。
    pub denominator: u8,
    /// 縦の寄せ方（分数の大きさのときだけ効く）。
    pub vertical: Align,
    /// 横の寄せ方（分数の大きさのときだけ効く）。
    pub horizontal: Align,
}

impl SizedBlock {
    /// 字を描く大きさの倍率（`scale` に分数を掛けたもの）。
    pub fn font_scale(&self) -> f64 {
        let s = f64::from(self.scale);
        if self.denominator > self.numerator {
            s * f64::from(self.numerator) / f64::from(self.denominator)
        } else {
            s
        }
    }
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
    /// 複数のセルにまたがる字（[`SizedBlock`]）の一部。四角の左上から右へ `dx`、下へ `dy` 番目のセル。
    /// 左上（`dx == 0 && dy == 0`）のセルだけが `width` を持つ。
    Sized {
        /// 四角全体。
        block: Arc<SizedBlock>,
        /// 左上から右へ何セル目か。
        dx: u8,
        /// 左上から下へ何セル目か。
        dy: u8,
    },
}

/// 画面の 1 マス。
///
/// `width` は、字が使うセルの数（1 か 2）。全角の字は左のセルが `width == 2` で字を持ち、
/// 右のセルは `Content::WideTail`（`width == 0`）になる。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    /// セルの中身。
    pub content: Content,
    /// 字が使うセルの数（1 か 2。全角の右半分は 0。複数のセルにまたがる字の左上は四角の幅、残りは 0）。
    pub width: u8,
    /// 属性。
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

    /// 何も書かれていないか。
    pub fn is_empty(&self) -> bool {
        self.content == Content::Empty
    }

    /// 全角の字の右半分か。
    pub fn is_wide_tail(&self) -> bool {
        self.content == Content::WideTail
    }

    /// 複数のセルにまたがる字の一部か。
    pub fn is_sized(&self) -> bool {
        matches!(self.content, Content::Sized { .. })
    }

    /// 全角の字（幅 2）の左半分か。
    pub(crate) fn is_wide(&self) -> bool {
        self.width == 2 && !self.is_sized()
    }

    /// 複数のセルにまたがる字の、左上と同じ行にある左上以外のセルか。
    pub(crate) fn is_sized_tail_right(&self) -> bool {
        matches!(self.content, Content::Sized { dx, dy: 0, .. } if dx != 0)
    }

    /// 字の続き（全角の右半分、または複数のセルにまたがる字の左上以外）か。
    pub fn is_continuation(&self) -> bool {
        match &self.content {
            Content::WideTail => true,
            Content::Sized { dx, dy, .. } => *dx != 0 || *dy != 0,
            _ => false,
        }
    }

    /// 全角の字の右半分のセル。幅は 0 と決めている。
    pub fn wide_tail(attrs: Attrs) -> Cell {
        Cell {
            content: Content::WideTail,
            width: 0,
            attrs,
        }
    }

    /// セルの字。空や全角の右半分は空文字列。
    pub fn text(&self) -> String {
        match &self.content {
            Content::Char(c) => c.to_string(),
            Content::Cluster(s) => s.to_string(),
            Content::Sized {
                block,
                dx: 0,
                dy: 0,
            } => block.text.to_string(),
            Content::Empty | Content::WideTail | Content::Sized { .. } => String::new(),
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
