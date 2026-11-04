//! kitty の text sizing protocol（OSC 66）の読み取り。
//! <https://sw.kovidgoyal.net/kitty/text-sizing-protocol/>
//!
//! `OSC 66 ; メタデータ ; 字 ST`。メタデータは `:` で区切った `キー=値`:
//! `s`（大きさ 1 から 7）、`w`（幅 0 から 7）、`n`・`d`（分数の大きさ。各 0 から 15、
//! `d` が 0 でないなら `d > n`）、`v`・`h`（寄せ方 0 から 2）。知らないキーは読み飛ばす。
//! 知っているキーの値が範囲の外なら、シーケンス全体を捨てる（仕様は決めていないので、
//! 描いてはいけないものを描くより捨てる）。字は UTF-8 で 4096 バイトまで、制御文字を含めない
//! （含むと全体を捨てる）。

use unicode_width::UnicodeWidthStr;

use crate::screen::cell::MAX_SCALE;
use crate::screen::{Align, SizedBlock};

/// 字の最大バイト数。
const MAX_TEXT: usize = 4096;

/// 読み取った結果。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Parsed {
    /// ふつうの大きさ・幅の字（`s=1`、`w` なし、分数なし）。ふつうに書く。
    Plain(String),
    /// 複数のセルにまたがる字。左から順に書く。
    Blocks(Vec<SizedBlock>),
}

fn align(v: u8) -> Option<Align> {
    match v {
        0 => Some(Align::Start),
        1 => Some(Align::End),
        2 => Some(Align::Center),
        _ => None,
    }
}

/// `data` は `66;` より後ろ（`メタデータ;字`）。読めなければ `None`。
pub(super) fn parse(data: &[u8]) -> Option<Parsed> {
    let sep = data.iter().position(|&b| b == b';')?;
    let meta = std::str::from_utf8(&data[..sep]).ok()?;
    let text = std::str::from_utf8(&data[sep + 1..]).ok()?;
    if text.is_empty() || text.len() > MAX_TEXT || text.chars().any(char::is_control) {
        return None;
    }
    let (mut s, mut w, mut n, mut d, mut v, mut h) = (1u8, 0u8, 0u8, 0u8, 0u8, 0u8);
    for pair in meta.split(':').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=')?;
        let (slot, min, max) = match key {
            "s" => (&mut s, 1, MAX_SCALE),
            "w" => (&mut w, 0, 7),
            "n" => (&mut n, 0, 15),
            "d" => (&mut d, 0, 15),
            "v" => (&mut v, 0, 2),
            "h" => (&mut h, 0, 2),
            _ => continue,
        };
        let value: u8 = value.parse().ok()?;
        if !(min..=max).contains(&value) {
            return None;
        }
        *slot = value;
    }
    if d != 0 && d <= n {
        return None;
    }
    // 分母が 0 か分子が 0 なら、分数の大きさは無いのと同じ（字が見えなくなるのを避ける）
    if d == 0 || n == 0 {
        n = 0;
        d = 0;
    }
    let (vertical, horizontal) = (align(v)?, align(h)?);
    if s == 1 && w == 0 && d == 0 {
        return Some(Parsed::Plain(text.to_string()));
    }
    let block = |text: &str, cols: usize| SizedBlock {
        text: text.into(),
        cols: cols as u8,
        scale: s,
        numerator: n,
        denominator: d,
        vertical,
        horizontal,
    };
    if w > 0 {
        return Some(Parsed::Blocks(vec![block(
            text,
            usize::from(w) * usize::from(s),
        )]));
    }
    // 幅が 0 なら、ふつうの字と同じに幅を決めて、1 つずつの四角に分ける。
    let mut clusters: Vec<String> = Vec::new();
    for c in text.chars() {
        let join = clusters.last().is_some_and(|l| l.ends_with('\u{200D}'))
            || unicode_width::UnicodeWidthChar::width(c) == Some(0);
        match clusters.last_mut() {
            Some(last) if join => last.push(c),
            _ if join => {}
            _ => clusters.push(c.to_string()),
        }
    }
    let blocks = clusters
        .iter()
        .map(|c| {
            block(
                c,
                UnicodeWidthStr::width(c.as_str()).clamp(1, 2) * usize::from(s),
            )
        })
        .collect();
    Some(Parsed::Blocks(blocks))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(data: &str) -> Vec<SizedBlock> {
        match parse(data.as_bytes()) {
            Some(Parsed::Blocks(b)) => b,
            other => panic!("not blocks: {other:?}"),
        }
    }

    #[test]
    fn plain_when_nothing_to_size() {
        assert_eq!(parse(b";abc"), Some(Parsed::Plain("abc".into())));
        assert_eq!(parse(b"s=1:v=2;abc"), Some(Parsed::Plain("abc".into())));
    }

    #[test]
    fn scale_splits_per_character() {
        let b = blocks("s=2;ab");
        assert_eq!(b.len(), 2);
        assert_eq!((b[0].cols, b[0].scale, &*b[0].text), (2, 2, "a"));
        let b = blocks("s=3;あ");
        assert_eq!((b[0].cols, b[0].scale), (6, 3));
    }

    #[test]
    fn explicit_width_keeps_text_in_one_block() {
        let b = blocks("w=2:s=2;abc");
        assert_eq!(b.len(), 1);
        assert_eq!((b[0].cols, b[0].scale, &*b[0].text), (4, 2, "abc"));
    }

    #[test]
    fn fraction_and_alignment() {
        let b = blocks("n=1:d=2:w=1:v=2:h=1;ab");
        assert_eq!((b[0].cols, b[0].scale), (1, 1));
        assert_eq!((b[0].numerator, b[0].denominator), (1, 2));
        assert_eq!(
            (b[0].vertical, b[0].horizontal),
            (Align::Center, Align::End)
        );
    }

    #[test]
    fn combining_marks_join_the_previous_character() {
        let b = blocks("s=2;e\u{301}x");
        assert_eq!(b.len(), 2);
        assert_eq!(&*b[0].text, "e\u{301}");
    }

    #[test]
    fn invalid_input_is_dropped() {
        for bad in [
            "s=0;a",
            "s=8;a",
            "w=8;a",
            "n=16:d=1;a",
            "n=2:d=2;a",
            "n=3:d=2;a",
            "v=3;a",
            "h=3;a",
            "s=x;a",
            "s;a",
            "s=2",
            "s=2;",
            "s=2;a\u{7}b",
            "s=2;a\u{1b}b",
            "s=2;a\u{85}",
        ] {
            assert_eq!(parse(bad.as_bytes()), None, "{bad:?}");
        }
        assert_eq!(parse(b"s=2;\xff"), None);
        let long = format!("s=2;{}", "a".repeat(MAX_TEXT + 1));
        assert_eq!(parse(long.as_bytes()), None);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        assert_eq!(blocks("s=2:z=9;a")[0].scale, 2);
        // n だけ（d が 0）は分数の大きさにならない
        assert_eq!(parse(b"n=1;a"), Some(Parsed::Plain("a".into())));
        // 分子が 0 でも同じ
        assert_eq!(parse(b"n=0:d=2;a"), Some(Parsed::Plain("a".into())));
    }
}
