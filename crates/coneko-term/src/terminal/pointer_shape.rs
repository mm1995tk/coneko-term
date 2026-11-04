//! kitty の mouse pointer shapes（OSC 22）。
//! <https://sw.kovidgoyal.net/kitty/pointer-shapes/>
//!
//! `OSC 22 ; [=|>|<|?] 名前,名前,... ST`。先頭の字が `=`（または無し）なら設定、`>` なら積む、
//! `<` なら降ろす（名前は読まない）、`?` なら問い合わせ。形の名前は CSS の `cursor` の値と
//! 同じ（表は [`SHAPES`]）。知らない名前は捨てる。形は主画面と代替画面で別々の積み重ねに持つ。
//! 空なら、使う形は呼ぶ側（ブラウザー）の自由。マウスの報告が有効かどうかには関わらない。
//!
//! xterm の古い名前（`X11/cursorfont.h`）は、仕様が「実装してもよい」とする別名で、
//! 扱わない（知らない名前として捨てる）。

/// 仕様の形の名前の表。どれも CSS の `cursor` の値としてそのまま使える。
pub const SHAPES: [&str; 30] = [
    "alias",
    "cell",
    "copy",
    "crosshair",
    "default",
    "e-resize",
    "ew-resize",
    "grab",
    "grabbing",
    "help",
    "move",
    "n-resize",
    "ne-resize",
    "nesw-resize",
    "no-drop",
    "not-allowed",
    "ns-resize",
    "nw-resize",
    "nwse-resize",
    "pointer",
    "progress",
    "s-resize",
    "se-resize",
    "sw-resize",
    "text",
    "vertical-text",
    "w-resize",
    "wait",
    "zoom-in",
    "zoom-out",
];

/// 積み重ねの最大の長さ（仕様の下限は 16）。あふれたら一番下を捨てる。
pub const MAX_STACK: usize = 32;

/// `__default__` に答える形。
const DEFAULT_SHAPE: &str = "text";
/// `__grabbed__` に答える形。
const GRABBED_SHAPE: &str = "default";

fn lookup(name: &str) -> Option<&'static str> {
    SHAPES.iter().copied().find(|&s| s == name)
}

/// 主画面（0）と代替画面（1）の形の積み重ね。
#[derive(Debug, Default)]
pub(super) struct PointerShapes {
    stacks: [Vec<&'static str>; 2],
}

impl PointerShapes {
    /// 両方の積み重ねを空にする。
    pub(super) fn clear(&mut self) {
        self.stacks = Default::default();
    }

    /// 画面 `alt` の今の形（積み重ねの一番上）。空なら `None`。
    pub(super) fn current(&self, alt: bool) -> Option<&'static str> {
        self.stacks[usize::from(alt)].last().copied()
    }

    /// `data` は `22;` より後ろ。問い合わせなら、返事の中身（`22;` より後ろ）を返す。
    pub(super) fn osc(&mut self, alt: bool, data: &[u8]) -> Option<String> {
        let text = std::str::from_utf8(data).ok()?;
        let (op, rest) = match text.chars().next() {
            Some(c @ ('=' | '>' | '<' | '?')) => (c, &text[1..]),
            _ => ('=', text),
        };
        let stack = &mut self.stacks[usize::from(alt)];
        let names = || rest.split(',').filter_map(|n| lookup(n.trim()));
        match op {
            '=' => {
                if rest.trim().is_empty() {
                    stack.clear();
                } else if let Some(shape) = names().next() {
                    match stack.last_mut() {
                        Some(top) => *top = shape,
                        None => stack.push(shape),
                    }
                }
                None
            }
            '>' => {
                for shape in names() {
                    if stack.len() >= MAX_STACK {
                        stack.remove(0);
                    }
                    stack.push(shape);
                }
                None
            }
            '<' => {
                stack.pop();
                None
            }
            _ => {
                if rest.is_empty() {
                    return None;
                }
                let current = stack.last().copied();
                let answers: Vec<&str> = rest
                    .split(',')
                    .map(|n| match n {
                        "__current__" => current.unwrap_or("0"),
                        "__default__" => DEFAULT_SHAPE,
                        "__grabbed__" => GRABBED_SHAPE,
                        n if lookup(n).is_some() => "1",
                        _ => "0",
                    })
                    .collect();
                Some(format!("22;{}", answers.join(",")))
            }
        }
    }
}
