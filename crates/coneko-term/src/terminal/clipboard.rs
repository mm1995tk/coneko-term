//! kitty の clipboard protocol（OSC 5522）。
//!
//! 仕様: <https://sw.kovidgoyal.net/kitty/clipboard/>
//!
//! ここでは書き込みと読み取りの頼みを読んでイベント（[`crate::Event::ClipboardWrite`]、
//! [`crate::Event::ClipboardRead`]）にするところと、その答えを PTY へ出すバイト列にするところまでを
//! 行う。OS のクリップボードにつなぐことと、読み取りを許すかどうかの判断は呼ぶ側（GUI）がする。
//!
//! - 書き込み（`type=write`、`type=wdata`、`type=walias`）は、MIME の種類ごとのデータを溜めて、
//!   終わりの `type=wdata`（`mime` もデータも無い）が来たところで 1 つのイベントにする。別名
//!   （`walias`）は、別名ごとに同じデータを持つ項目に広げる。成功の返事（`status=DONE`）は
//!   呼ぶ側が [`write_reply`] で出す。
//! - 書き込みのパケットが不正（base64 が読めない、`mime` が無い、`loc` が知らない値）なら `EINVAL`、
//!   大きすぎれば `EFBIG` を、ここですぐ返す。エラーのあとは、次の `type=write` まで書き込みの
//!   パケットを読み捨てる（仕様）。
//! - 読み取り（`type=read`）は、読み取れない形（base64 が読めない、MIME の種類が無い、`loc` が
//!   知らない値）なら黙って捨てる（仕様: 読み取りにはエラーの状態が無い）。
//! - `id` は返事にそのまま入るので、`[a-zA-Z0-9_+.-]` 以外の字は取り除く（仕様）。
//! - 知らない `type` やキーは捨てる。

use super::Event;

/// 1 つの書き込みの大きさの上限（バイト）。仕様は 64 MB 以上を受けることを求める。
const MAX_WRITE_BYTES: usize = 64 * 1024 * 1024;
/// 1 つの書き込みの MIME の種類の数の上限（別名を広げたあとの数）。
const MAX_ITEMS: usize = 256;
/// 1 つの読み取りの頼みに書ける MIME の種類の数の上限。超えた頼みは捨てる。
const MAX_READ_MIMES: usize = 256;
/// 1 つのパケットに入れるデータの大きさ（base64 にする前、バイト）。仕様の上限。
const CHUNK: usize = 4096;

/// どちらのクリップボードか。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClipboardLocation {
    /// OS の主のクリップボード（`loc` が無いとき）。
    #[default]
    Clipboard,
    /// primary selection（`loc=primary`）。
    Primary,
}

/// 1 つの MIME の種類のデータ。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipboardItem {
    /// MIME の種類（`text/plain` など）。
    pub mime: String,
    /// データ（戻したあとのバイト列）。
    pub data: Vec<u8>,
}

/// そろった書き込み。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipboardWrite {
    /// `id`（無ければ空）。[`write_reply`] に渡す。
    pub id: String,
    /// 書き込み先。
    pub location: ClipboardLocation,
    /// MIME の種類ごとのデータ。別名は広げてある。
    pub items: Vec<ClipboardItem>,
}

/// 読み取りの頼み。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardReadRequest {
    /// あるデータの種類の一覧がほしい（ペイロードが `.`）。許可は求めなくてよい（仕様）。
    Types,
    /// これらの MIME の種類のデータがほしい（先に書いたものほど優先）。
    Mimes(Vec<String>),
}

/// 書き込みの返事のエラー。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClipboardWriteError {
    /// 入出力のエラー。
    Eio,
    /// パケットが不正。
    Einval,
    /// 書き込み先（primary selection）が無い。
    Enosys,
    /// 許されない。
    Eperm,
    /// 一時的に使えない。
    Ebusy,
    /// 大きすぎる。
    Efbig,
}

impl ClipboardWriteError {
    fn code(self) -> &'static str {
        match self {
            ClipboardWriteError::Eio => "EIO",
            ClipboardWriteError::Einval => "EINVAL",
            ClipboardWriteError::Enosys => "ENOSYS",
            ClipboardWriteError::Eperm => "EPERM",
            ClipboardWriteError::Ebusy => "EBUSY",
            ClipboardWriteError::Efbig => "EFBIG",
        }
    }
}

/// 読み取りの返事のエラー。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClipboardReadError {
    /// 頼まれた場所が無い（`loc=primary` に対応していないなど）。
    Enosys,
    /// 許されない。
    Eperm,
    /// 一時的に使えない。
    Ebusy,
}

impl ClipboardReadError {
    fn code(self) -> &'static str {
        match self {
            ClipboardReadError::Enosys => "ENOSYS",
            ClipboardReadError::Eperm => "EPERM",
            ClipboardReadError::Ebusy => "EBUSY",
        }
    }
}

/// OSC 5522 のうち、外へ知らせるもの。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Output {
    /// 書き込みがそろった。
    Event(Event),
    /// 不正な書き込みへの、ターミナル自身の返事。
    Reply(Vec<u8>),
}

/// 組み立て中の書き込み。
#[derive(Debug)]
struct Pending {
    id: String,
    location: ClipboardLocation,
    /// MIME の種類ごとの、戻す前の base64（パケットをつなげたもの）。
    items: Vec<(String, Vec<u8>)>,
    aliases: Vec<(String, Vec<String>)>,
    total: usize,
}

/// 組み立て中の書き込みの置き場（同時に 1 つ）。
#[derive(Debug, Default)]
pub(super) struct Clipboard {
    pending: Option<Pending>,
}

/// 1 つのパケットの鍵。
struct Meta<'a> {
    kind: &'a str,
    id: String,
    loc: Option<&'a str>,
    mime: Option<&'a str>,
}

impl<'a> Meta<'a> {
    fn parse(meta: &'a str) -> Meta<'a> {
        let mut m = Meta {
            kind: "",
            id: String::new(),
            loc: None,
            mime: None,
        };
        for kv in meta.split(':') {
            let Some((k, v)) = kv.split_once('=') else {
                continue;
            };
            match k {
                "type" => m.kind = v,
                "id" => m.id = clean_id(v),
                "loc" => m.loc = Some(v),
                "mime" => m.mime = Some(v),
                _ => {}
            }
        }
        m
    }
}

fn clean_id(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '.' | '-'))
        .collect()
}

fn location(loc: Option<&str>) -> Option<ClipboardLocation> {
    match loc {
        None | Some("") => Some(ClipboardLocation::Clipboard),
        Some("primary") => Some(ClipboardLocation::Primary),
        Some(_) => None,
    }
}

impl Clipboard {
    /// 組み立て中のものを捨てる。
    pub(super) fn clear(&mut self) {
        self.pending = None;
    }

    /// `OSC 5522 ;` のあとの `鍵 ; 中身` を処理する。
    pub(super) fn osc(&mut self, rest: &[u8]) -> Option<Output> {
        let (meta, payload) = match rest.iter().position(|&b| b == b';') {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => (rest, &[][..]),
        };
        let meta = Meta::parse(std::str::from_utf8(meta).ok()?);
        match meta.kind {
            "write" => self.start_write(&meta),
            "wdata" => self.wdata(meta, payload),
            "walias" => self.walias(meta, payload),
            "read" => read(&meta, payload),
            _ => None,
        }
    }

    fn start_write(&mut self, meta: &Meta) -> Option<Output> {
        // 新しい書き込みは、途中のものを捨てて始める。
        self.pending = None;
        let Some(location) = location(meta.loc) else {
            return Some(error(&meta.id, ClipboardWriteError::Einval));
        };
        self.pending = Some(Pending {
            id: meta.id.clone(),
            location,
            items: Vec::new(),
            aliases: Vec::new(),
            total: 0,
        });
        None
    }

    fn wdata(&mut self, meta: Meta, payload: &[u8]) -> Option<Output> {
        let mut p = self.pending.take()?;
        if !meta.id.is_empty() {
            p.id = meta.id;
        }
        let Some(mime) = meta.mime else {
            // 終わり: `mime` もデータも無い。
            if !payload.is_empty() {
                return Some(error(&p.id, ClipboardWriteError::Einval));
            }
            return Some(match p.finish() {
                Ok(w) => Output::Event(Event::ClipboardWrite(Box::new(w))),
                Err((id, e)) => error(&id, e),
            });
        };
        let Some(mime) = decode_mime(mime) else {
            return Some(error(&p.id, ClipboardWriteError::Einval));
        };
        // 大きさは、戻したあとの大きさを base64 の長さから見積もる。
        p.total += payload.len() / 4 * 3;
        if p.total > MAX_WRITE_BYTES {
            return Some(error(&p.id, ClipboardWriteError::Efbig));
        }
        match p.items.iter_mut().find(|i| i.0 == mime) {
            Some(item) => item.1.extend_from_slice(payload),
            None => {
                if p.items.len() >= MAX_ITEMS {
                    return Some(error(&p.id, ClipboardWriteError::Efbig));
                }
                p.items.push((mime, payload.to_vec()));
            }
        }
        self.pending = Some(p);
        None
    }

    fn walias(&mut self, meta: Meta, payload: &[u8]) -> Option<Output> {
        let mut p = self.pending.take()?;
        if !meta.id.is_empty() {
            p.id = meta.id;
        }
        let names = decode_base64(payload)
            .and_then(|b| String::from_utf8(b).ok())
            .map(|s| {
                s.split_ascii_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            });
        let (Some(target), Some(names)) = (meta.mime.and_then(decode_mime), names) else {
            return Some(error(&p.id, ClipboardWriteError::Einval));
        };
        if p.aliases.len() >= MAX_ITEMS || names.len() > MAX_ITEMS {
            return Some(error(&p.id, ClipboardWriteError::Efbig));
        }
        p.aliases.push((target, names));
        self.pending = Some(p);
        None
    }
}

impl Pending {
    /// 別名を広げて書き込みにする。大きすぎれば `EFBIG`。
    fn finish(mut self) -> Result<ClipboardWrite, (String, ClipboardWriteError)> {
        // 分けて送られたパケットは、つなげてから戻す（個々の長さは 4 の倍数でなくてよい）。
        let mut items = Vec::new();
        for (mime, b64) in std::mem::take(&mut self.items) {
            let Some(data) = decode_base64(&b64) else {
                return Err((self.id, ClipboardWriteError::Einval));
            };
            items.push(ClipboardItem { mime, data });
        }
        let mut total = self.total;
        for (target, names) in &self.aliases {
            let Some(data) = items
                .iter()
                .find(|i| &i.mime == target)
                .map(|i| i.data.clone())
            else {
                continue;
            };
            for name in names {
                if items.iter().any(|i| &i.mime == name) {
                    continue;
                }
                total += data.len();
                if total > MAX_WRITE_BYTES || items.len() >= MAX_ITEMS {
                    return Err((self.id, ClipboardWriteError::Efbig));
                }
                items.push(ClipboardItem {
                    mime: name.clone(),
                    data: data.clone(),
                });
            }
        }
        Ok(ClipboardWrite {
            id: self.id,
            location: self.location,
            items,
        })
    }
}

fn read(meta: &Meta, payload: &[u8]) -> Option<Output> {
    let location = location(meta.loc)?;
    let text = String::from_utf8(decode_base64(payload)?).ok()?;
    let mimes: Vec<String> = text.split_ascii_whitespace().map(str::to_string).collect();
    if mimes.is_empty() || mimes.len() > MAX_READ_MIMES {
        return None;
    }
    let request = if mimes == ["."] {
        ClipboardReadRequest::Types
    } else {
        ClipboardReadRequest::Mimes(mimes)
    };
    Some(Output::Event(Event::ClipboardRead {
        id: meta.id.clone(),
        location,
        request,
    }))
}

fn decode_mime(b64: &str) -> Option<String> {
    let mime = String::from_utf8(decode_base64(b64.as_bytes())?).ok()?;
    (!mime.is_empty() && !mime.chars().any(|c| c.is_control() || c.is_whitespace())).then_some(mime)
}

fn error(id: &str, e: ClipboardWriteError) -> Output {
    Output::Reply(write_reply(id, Err(e)))
}

// ---- base64 ----

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode_base64(src: &[u8]) -> String {
    let mut out = String::with_capacity(src.len().div_ceil(3) * 4);
    for c in src.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// 厳密な base64（標準の文字、詰め物は必須、字の間の空白や途中の `=` は不可）を戻す。
/// 仕様: 不正な字を黙って捨ててはいけない。
fn decode_base64(src: &[u8]) -> Option<Vec<u8>> {
    if src.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(src.len() / 4 * 3);
    let last = src.len().saturating_sub(4);
    for (n, quad) in src.chunks(4).enumerate() {
        let pad = if n * 4 == last {
            quad.iter().rev().take_while(|&&b| b == b'=').count()
        } else {
            0
        };
        if pad > 2 {
            return None;
        }
        let mut acc = 0u32;
        for &c in &quad[..4 - pad] {
            let v = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                _ => return None,
            } as u32;
            acc = acc << 6 | v;
        }
        acc <<= 6 * pad as u32;
        // 詰め物に隠れたビットは 0 でなければならない。
        if pad > 0 && acc & ((1 << (8 * pad)) - 1) != 0 {
            return None;
        }
        out.extend_from_slice(&acc.to_be_bytes()[1..4 - pad]);
    }
    Some(out)
}

// ---- PTY へ送る返事 ----

/// `ESC ] 5522 ; 鍵 [: id=…] [; 中身] ESC \`。
fn packet(meta: &str, id: &str, payload: &str) -> String {
    let id = clean_id(id);
    let id = if id.is_empty() {
        String::new()
    } else {
        format!(":id={id}")
    };
    if payload.is_empty() {
        format!("\x1b]5522;{meta}{id}\x1b\\")
    } else {
        format!("\x1b]5522;{meta}{id};{payload}\x1b\\")
    }
}

/// 書き込みの返事。成功なら `status=DONE`、失敗ならそのエラー。
pub fn write_reply(id: &str, result: Result<(), ClipboardWriteError>) -> Vec<u8> {
    let status = match result {
        Ok(()) => "DONE",
        Err(e) => e.code(),
    };
    packet(&format!("type=write:status={status}"), id, "").into_bytes()
}

/// 読み取りの失敗の返事（`status=OK` の代わりに送る）。
pub fn read_error_reply(id: &str, e: ClipboardReadError) -> Vec<u8> {
    packet(&format!("type=read:status={}", e.code()), id, "").into_bytes()
}

/// データを 4096 バイトごとのパケットにして、`status=DATA` で並べる。
fn data_packets(out: &mut String, id: &str, mime: &str, data: &[u8]) {
    let meta = format!(
        "type=read:status=DATA:mime={}",
        encode_base64(mime.as_bytes())
    );
    if data.is_empty() {
        out.push_str(&packet(&meta, id, ""));
    }
    for chunk in data.chunks(CHUNK) {
        out.push_str(&packet(&meta, id, &encode_base64(chunk)));
    }
}

/// 種類の一覧（`.`）への返事。`mimes` が空なら、データの無い `OK` と `DONE` だけを返す。
///
/// 一覧は、`mime` が `.` の `status=DATA` に、空白で区切った MIME の種類を入れて送る。
pub fn read_types_reply(id: &str, mimes: &[String]) -> Vec<u8> {
    let mut out = packet("type=read:status=OK", id, "");
    if !mimes.is_empty() {
        data_packets(&mut out, id, ".", mimes.join(" ").as_bytes());
    }
    out.push_str(&packet("type=read:status=DONE", id, ""));
    out.into_bytes()
}

/// データの読み取りへの返事。`items` が空（頼まれた種類のデータが 1 つも無い）なら、データの無い
/// `OK` と `DONE` だけを返す。場所が無いときは [`read_error_reply`] を使う。
pub fn read_data_reply(id: &str, items: &[ClipboardItem]) -> Vec<u8> {
    let mut out = packet("type=read:status=OK", id, "");
    for item in items {
        data_packets(&mut out, id, &item.mime, &item.data);
    }
    out.push_str(&packet("type=read:status=DONE", id, ""));
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::Terminal;

    fn b64(s: &str) -> String {
        encode_base64(s.as_bytes())
    }

    fn run(input: &str) -> (Terminal, Vec<Event>) {
        let mut t = Terminal::new(5, 20);
        t.write(input.as_bytes());
        let ev = t.take_events();
        (t, ev)
    }

    fn write_of(input: &str) -> ClipboardWrite {
        match run(input).1.as_slice() {
            [Event::ClipboardWrite(w)] => (**w).clone(),
            other => panic!("expected one write, got {other:?}"),
        }
    }

    fn item(mime: &str, data: &[u8]) -> ClipboardItem {
        ClipboardItem {
            mime: mime.into(),
            data: data.to_vec(),
        }
    }

    #[test]
    fn base64_roundtrip_and_strictness() {
        for s in ["", "a", "ab", "abc", "abcd", "日本語"] {
            assert_eq!(
                decode_base64(encode_base64(s.as_bytes()).as_bytes()).unwrap(),
                s.as_bytes()
            );
        }
        assert_eq!(encode_base64(b"ab"), "YWI=");
        // 詰め物が無い・途中の詰め物・空白・不正な字・隠れたビットは断る。
        for bad in ["YWI", "YQ==YQ==", "YW I=", "YW*=", "YR==", "====", "Y==="] {
            assert_eq!(decode_base64(bad.as_bytes()), None, "{bad}");
        }
    }

    #[test]
    fn write_text_and_png() {
        // 仕様の流れ: type=write、MIME ごとの type=wdata、終わりの type=wdata。
        let input = format!(
            "\x1b]5522;type=write\x1b\\\
             \x1b]5522;type=wdata:mime={};{}\x1b\\\
             \x1b]5522;type=wdata:mime={};{}\x1b\\\
             \x1b]5522;type=wdata\x1b\\",
            b64("text/plain"),
            b64("hello"),
            b64("image/png"),
            encode_base64(&[0x89, b'P', b'N', b'G']),
        );
        let w = write_of(&input);
        assert_eq!(w.id, "");
        assert_eq!(w.location, ClipboardLocation::Clipboard);
        assert_eq!(
            w.items,
            vec![
                item("text/plain", b"hello"),
                item("image/png", &[0x89, b'P', b'N', b'G'])
            ]
        );
    }

    #[test]
    fn chunks_of_one_mime_are_joined_and_bel_terminates() {
        let input = format!(
            "\x1b]5522;type=write:id=a-1\x07\
             \x1b]5522;type=wdata:mime={m};{}\x07\
             \x1b]5522;type=wdata:mime={m};{}\x07\
             \x1b]5522;type=wdata\x07",
            b64("hel"),
            b64("lo"),
            m = b64("text/plain"),
        );
        let w = write_of(&input);
        assert_eq!(w.id, "a-1");
        assert_eq!(w.items, vec![item("text/plain", b"hello")]);
    }

    #[test]
    fn primary_selection_and_id_cleaning() {
        let input = "\x1b]5522;type=write:loc=primary:id=x y/z\x1b\\\x1b]5522;type=wdata\x1b\\";
        let w = write_of(input);
        assert_eq!(w.location, ClipboardLocation::Primary);
        assert_eq!(w.id, "xyz");
        assert!(w.items.is_empty());
    }

    #[test]
    fn payloads_split_off_the_4_boundary_are_joined_before_decoding() {
        let m = b64("text/plain");
        // "abc" = YWJj を YW と Jj に分ける。
        let w = write_of(&format!(
            "\x1b]5522;type=write\x1b\\\x1b]5522;type=wdata:mime={m};YW\x1b\\\x1b]5522;type=wdata:mime={m};Jj\x1b\\\x1b]5522;type=wdata\x1b\\"
        ));
        assert_eq!(w.items, vec![item("text/plain", b"abc")]);
        // 途中の詰め物（YWI= + YWJj）は、つなげると不正。
        let (mut t, ev) = run(&format!(
            "\x1b]5522;type=write\x1b\\\x1b]5522;type=wdata:mime={m};YWI=\x1b\\\x1b]5522;type=wdata:mime={m};YWJj\x1b\\\x1b]5522;type=wdata\x1b\\"
        ));
        assert!(ev.is_empty());
        assert_eq!(t.take_output(), b"\x1b]5522;type=write:status=EINVAL\x1b\\");
    }

    #[test]
    fn aliases_become_items() {
        let input = format!(
            "\x1b]5522;type=write\x1b\\\
             \x1b]5522;type=walias:mime={};{}\x1b\\\
             \x1b]5522;type=wdata:mime={};{}\x1b\\\
             \x1b]5522;type=wdata\x1b\\",
            b64("text/plain"),
            b64("text/plain;charset=utf-8 UTF8_STRING"),
            b64("text/plain"),
            b64("hi"),
        );
        let w = write_of(&input);
        assert_eq!(
            w.items,
            vec![
                item("text/plain", b"hi"),
                item("text/plain;charset=utf-8", b"hi"),
                item("UTF8_STRING", b"hi"),
            ]
        );
    }

    #[test]
    fn no_event_until_the_end_packet() {
        let input = format!(
            "\x1b]5522;type=write\x1b\\\x1b]5522;type=wdata:mime={};{}\x1b\\",
            b64("text/plain"),
            b64("hi")
        );
        let (mut t, ev) = run(&input);
        assert!(ev.is_empty());
        assert!(t.take_output().is_empty());
    }

    #[test]
    fn invalid_base64_is_einval_and_later_packets_are_ignored() {
        let input = format!(
            "\x1b]5522;type=write:id=w\x1b\\\
             \x1b]5522;type=wdata:mime={};!!!!\x1b\\\
             \x1b]5522;type=wdata:mime={};{}\x1b\\\
             \x1b]5522;type=wdata\x1b\\",
            b64("text/plain"),
            b64("text/plain"),
            b64("hi"),
        );
        let (mut t, ev) = run(&input);
        assert!(ev.is_empty());
        assert_eq!(
            t.take_output(),
            b"\x1b]5522;type=write:status=EINVAL:id=w\x1b\\"
        );
        // 次の type=write からは、また受ける。
        t.write(b"\x1b]5522;type=write\x1b\\\x1b]5522;type=wdata\x1b\\");
        assert_eq!(t.take_events().len(), 1);
    }

    #[test]
    fn missing_mime_with_data_and_unknown_loc_are_einval() {
        let (mut t, ev) = run(&format!(
            "\x1b]5522;type=write\x1b\\\x1b]5522;type=wdata;{}\x1b\\",
            b64("x")
        ));
        assert!(ev.is_empty());
        assert_eq!(t.take_output(), b"\x1b]5522;type=write:status=EINVAL\x1b\\");
        let (mut t, ev) = run("\x1b]5522;type=write:loc=nowhere\x1b\\\x1b]5522;type=wdata\x1b\\");
        assert!(ev.is_empty());
        assert_eq!(t.take_output(), b"\x1b]5522;type=write:status=EINVAL\x1b\\");
    }

    #[test]
    fn too_big_is_efbig() {
        let mut t = Terminal::new(5, 20);
        t.write(b"\x1b]5522;type=write\x1b\\");
        let chunk = encode_base64(&vec![0u8; CHUNK]);
        let packet = format!("\x1b]5522;type=wdata:mime={};{chunk}\x1b\\", b64("a/b"));
        for _ in 0..=(MAX_WRITE_BYTES / CHUNK) {
            t.write(packet.as_bytes());
        }
        assert_eq!(t.take_output(), b"\x1b]5522;type=write:status=EFBIG\x1b\\");
        // エラーのあとは読み捨てる。
        t.write(b"\x1b]5522;type=wdata\x1b\\");
        assert!(t.take_events().is_empty());
    }

    #[test]
    fn read_mimes_and_list() {
        let input = format!(
            "\x1b]5522;type=read:id=r1;{}\x1b\\\x1b]5522;type=read:loc=primary;{}\x1b\\",
            b64("text/plain image/png"),
            b64("."),
        );
        let (_, ev) = run(&input);
        assert_eq!(
            ev,
            vec![
                Event::ClipboardRead {
                    id: "r1".into(),
                    location: ClipboardLocation::Clipboard,
                    request: ClipboardReadRequest::Mimes(vec![
                        "text/plain".into(),
                        "image/png".into()
                    ]),
                },
                Event::ClipboardRead {
                    id: "".into(),
                    location: ClipboardLocation::Primary,
                    request: ClipboardReadRequest::Types,
                },
            ]
        );
    }

    #[test]
    fn bad_reads_are_dropped_silently() {
        for input in [
            "\x1b]5522;type=read;!!!!\x1b\\".to_string(),
            "\x1b]5522;type=read\x1b\\".to_string(),
            format!("\x1b]5522;type=read;{}\x1b\\", b64("  ")),
            format!("\x1b]5522;type=read:loc=x;{}\x1b\\", b64("text/plain")),
            "\x1b]5522;type=nope\x1b\\".to_string(),
        ] {
            let (mut t, ev) = run(&input);
            assert!(ev.is_empty(), "{input:?}");
            assert!(t.take_output().is_empty(), "{input:?}");
        }
    }

    #[test]
    fn write_replies() {
        assert_eq!(
            write_reply("", Ok(())),
            b"\x1b]5522;type=write:status=DONE\x1b\\"
        );
        assert_eq!(
            write_reply("a b", Err(ClipboardWriteError::Eperm)),
            b"\x1b]5522;type=write:status=EPERM:id=ab\x1b\\"
        );
    }

    #[test]
    fn read_replies() {
        let text = b64("text/plain");
        assert_eq!(
            read_data_reply("i", &[item("text/plain", b"hi")]),
            format!(
                "\x1b]5522;type=read:status=OK:id=i\x1b\\\
                 \x1b]5522;type=read:status=DATA:mime={text}:id=i;aGk=\x1b\\\
                 \x1b]5522;type=read:status=DONE:id=i\x1b\\"
            )
            .into_bytes()
        );
        assert_eq!(
            read_types_reply("", &["text/plain".into(), "image/png".into()]),
            format!(
                "\x1b]5522;type=read:status=OK\x1b\\\
                 \x1b]5522;type=read:status=DATA:mime=Lg==;{}\x1b\\\
                 \x1b]5522;type=read:status=DONE\x1b\\",
                b64("text/plain image/png")
            )
            .into_bytes()
        );
        assert_eq!(
            read_data_reply("", &[]),
            b"\x1b]5522;type=read:status=OK\x1b\\\x1b]5522;type=read:status=DONE\x1b\\"
        );
        assert_eq!(
            read_error_reply("", ClipboardReadError::Eperm),
            b"\x1b]5522;type=read:status=EPERM\x1b\\"
        );
    }

    #[test]
    fn read_data_is_chunked_at_4096_bytes() {
        let data = vec![7u8; CHUNK * 2 + 1];
        let out = String::from_utf8(read_data_reply("", &[item("a/b", &data)])).unwrap();
        // OK + 3 つのデータ + DONE。
        assert_eq!(out.matches("\x1b]5522;").count(), 5);
        let payloads: Vec<&str> = out
            .split("\x1b\\")
            .filter(|p| p.contains("status=DATA"))
            .map(|p| p.rsplit(';').next().unwrap())
            .collect();
        let lens: Vec<usize> = payloads
            .iter()
            .map(|p| decode_base64(p.as_bytes()).unwrap().len())
            .collect();
        assert_eq!(lens, vec![CHUNK, CHUNK, 1]);
    }

    #[test]
    fn mode_5522_is_known_to_decrqm() {
        let mut t = Terminal::new(5, 20);
        t.write(b"\x1b[?5522$p");
        assert_eq!(t.take_output(), b"\x1b[?5522;2$y");
        t.write(b"\x1b[?5522h\x1b[?5522$p");
        assert!(t.modes().clipboard_paste_events);
        assert_eq!(t.take_output(), b"\x1b[?5522;1$y");
    }

    #[test]
    fn terminal_replies_reach_the_output() {
        let mut t = Terminal::new(5, 20);
        t.clipboard_write_reply("w", Ok(()));
        t.clipboard_read_reply_error("r", ClipboardReadError::Eperm);
        assert_eq!(
            t.take_output(),
            b"\x1b]5522;type=write:status=DONE:id=w\x1b\\\x1b]5522;type=read:status=EPERM:id=r\x1b\\"
        );
    }
}
