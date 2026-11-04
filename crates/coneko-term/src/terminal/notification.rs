//! kitty の desktop notifications（OSC 99）。
//!
//! 仕様: <https://sw.kovidgoyal.net/kitty/desktop-notifications/>
//!
//! ここでは通知を読んでイベント（[`crate::Event::Notification`] など）にするところまでを行う。
//! OS の通知を出すことと、アイコンの画像（`p=icon`）の読み込みはしない。
//!
//! - `d=0` で分けて送られた通知は、`d=1`（既定）の塊が来たところで 1 つにそろえる。題名（`p=title`）
//!   と本文（`p=body`）とボタン（`p=buttons`）は別々に溜める。`i` が同じ塊は同じ通知。`i` が無い塊は
//!   すべて同じ 1 つの通知として扱う。`i` 以外の鍵は、後の塊に書いてあれば上書きする。
//! - `e=1`（base64）は、そろった後で戻す。
//! - 題名も本文も空の通知は捨てる。
//! - `i` に使えるのは `[a-zA-Z0-9_+.-]` だけ。それ以外を含む塊は捨てる（`i` は PTY への返事に
//!   そのまま入るので、制御文字を通さないため）。
//! - 知らない `p`、読めない値、大きすぎる通知は捨てる。

/// 1 つの通知の中身の大きさの上限（バイト）。超えた通知は捨てる。
const MAX_PAYLOAD: usize = 256 * 1024;
/// 同時に組み立て中にできる通知の数。超えたら古いものから捨てる。
const MAX_PENDING: usize = 16;
/// ボタンの区切り（U+2028）。
const BUTTON_SEP: char = '\u{2028}';

/// 通知を押したときの動き（`a`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Actions {
    /// 押されたらウィンドウを前に出す（`focus`）。
    pub focus: bool,
    /// 押されたことをアプリケーションへ知らせる（`report`）。
    pub report: bool,
}

impl Actions {
    /// `a` の値（`,` 区切り。既定から順に当てる）。
    fn parse(v: &str) -> Actions {
        let mut a = Actions::default();
        for t in v.split(',') {
            match t {
                "focus" => a.focus = true,
                "-focus" => a.focus = false,
                "report" => a.report = true,
                "-report" => a.report = false,
                _ => {}
            }
        }
        a
    }
}

impl Default for Actions {
    fn default() -> Self {
        Actions {
            focus: true,
            report: false,
        }
    }
}

/// いつ通知を出すか（`o`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum When {
    /// いつでも（`always`、既定）。
    #[default]
    Always,
    /// ウィンドウがフォーカスされていないとき（`unfocused`）。
    Unfocused,
    /// ウィンドウが見えていないとき（`invisible`）。
    Invisible,
}

impl When {
    fn parse(v: &str) -> Option<When> {
        Some(match v {
            "always" => When::Always,
            "unfocused" => When::Unfocused,
            "invisible" => When::Invisible,
            _ => return None,
        })
    }
}

/// 組み立て終わった通知。仕様のキーを読んだ形で持つ。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Notification {
    /// `i`。無ければ空。
    pub id: String,
    /// 題名（`p=title`）。
    pub title: String,
    /// 本文（`p=body`）。
    pub body: String,
    /// 緊急度（`u`）。0 低、1 普通、2 高。指定が無ければ `None`。
    pub urgency: Option<u8>,
    /// 押されたときの動き（`a`）。
    pub actions: Actions,
    /// いつ出すか（`o`）。
    pub when: When,
    /// 閉じられたときに知らせるか（`c=1`）。
    pub report_close: bool,
    /// アプリケーションの名前（`f`）。
    pub app_name: Option<String>,
    /// 通知の種類（`t`。複数）。
    pub types: Vec<String>,
    /// 消えるまでの時間（ミリ秒、`w`）。`-1` は OS の既定、`0` は消えない。指定が無ければ `None`。
    pub timeout: Option<i32>,
    /// 音（`s`）。
    pub sound: Option<String>,
    /// アイコンの名前（`n`。複数）。
    pub icon_names: Vec<String>,
    /// アイコンの画像を指す ID（`g`）。画像そのものは読まない。
    pub icon_cache_id: Option<String>,
    /// ボタンの文字（`p=buttons`。U+2028 で区切る）。
    pub buttons: Vec<String>,
}

/// `p=?` への返事に書く、対応していることの一覧。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationSupport {
    /// 対応する `a` の値（`focus`、`report`）。
    pub actions: Vec<String>,
    /// 対応する `o` の値。
    pub when: Vec<String>,
    /// 対応する `p` の値。
    pub payloads: Vec<String>,
    /// 対応する `u` の値。
    pub urgencies: Vec<u8>,
    /// `c=1`（閉じたことの報告）に対応するか。
    pub close_report: bool,
    /// `w`（消えるまでの時間）に対応するか。
    pub timeout: bool,
    /// 対応する `s`（音）の値。
    pub sounds: Vec<String>,
    /// 対応する `t`（種類）の値。
    pub types: Vec<String>,
}

/// OSC 99 のうち、外へ知らせるもの。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Output {
    Notification(Box<Notification>),
    Close(String),
    Query(String),
    Alive(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Payload {
    Title,
    Body,
    Close,
    Query,
    Alive,
    Icon,
    Buttons,
}

impl Payload {
    fn parse(s: &str) -> Option<Payload> {
        Some(match s {
            "title" => Payload::Title,
            "body" => Payload::Body,
            "close" => Payload::Close,
            "?" => Payload::Query,
            "alive" => Payload::Alive,
            "icon" => Payload::Icon,
            "buttons" => Payload::Buttons,
            _ => return None,
        })
    }
}

/// 1 つの塊の鍵。
struct Meta {
    id: String,
    done: bool,
    payload: Payload,
    base64: Option<bool>,
    actions: Option<Actions>,
    when: Option<When>,
    urgency: Option<u8>,
    report_close: Option<bool>,
    timeout: Option<i32>,
    app_name: Option<Vec<u8>>,
    sound: Option<Vec<u8>>,
    icon_cache_id: Option<String>,
    types: Vec<Vec<u8>>,
    icon_names: Vec<Vec<u8>>,
}

fn valid_id(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'.' | b'-'))
}

impl Meta {
    fn parse(meta: &str) -> Option<Meta> {
        let mut m = Meta {
            id: String::new(),
            done: true,
            payload: Payload::Title,
            base64: None,
            actions: None,
            when: None,
            urgency: None,
            report_close: None,
            timeout: None,
            app_name: None,
            sound: None,
            icon_cache_id: None,
            types: Vec::new(),
            icon_names: Vec::new(),
        };
        for kv in meta.split(':') {
            let Some((k, v)) = kv.split_once('=') else {
                continue;
            };
            match k {
                "i" => {
                    if !valid_id(v) {
                        return None;
                    }
                    m.id = v.to_string();
                }
                "d" => match v {
                    "0" => m.done = false,
                    "1" => m.done = true,
                    _ => {}
                },
                "p" => m.payload = Payload::parse(v)?,
                "e" => match v {
                    "0" => m.base64 = Some(false),
                    "1" => m.base64 = Some(true),
                    _ => {}
                },
                "a" => m.actions = Some(Actions::parse(v)),
                "o" => m.when = When::parse(v).or(m.when),
                "u" => {
                    if let Ok(u @ 0..=2) = v.parse::<u8>() {
                        m.urgency = Some(u);
                    }
                }
                "c" => match v {
                    "0" => m.report_close = Some(false),
                    "1" => m.report_close = Some(true),
                    _ => {}
                },
                "w" => {
                    if let Ok(w) = v.parse::<i32>() {
                        if w >= -1 {
                            m.timeout = Some(w);
                        }
                    }
                }
                "f" => m.app_name = decode_base64(v.as_bytes()),
                "s" => m.sound = decode_base64(v.as_bytes()),
                "g" => {
                    if valid_id(v) && !v.is_empty() {
                        m.icon_cache_id = Some(v.to_string());
                    }
                }
                "t" => m.types.extend(decode_base64(v.as_bytes())),
                "n" => m.icon_names.extend(decode_base64(v.as_bytes())),
                _ => {}
            }
        }
        Some(m)
    }
}

/// 組み立て中の通知。
#[derive(Debug, Default)]
struct Pending {
    id: String,
    title: Vec<u8>,
    title_b64: bool,
    body: Vec<u8>,
    body_b64: bool,
    buttons: Vec<u8>,
    buttons_b64: bool,
    n: Notification,
    too_big: bool,
}

/// 組み立て中の通知の置き場。
#[derive(Debug, Default)]
pub(super) struct Notifications {
    pending: Vec<Pending>,
}

impl Notifications {
    /// 組み立て中のものを捨てる。
    pub(super) fn clear(&mut self) {
        self.pending.clear();
    }

    /// `OSC 99 ;` のあとの `鍵 ; 中身` を処理する。
    pub(super) fn osc(&mut self, rest: &[u8]) -> Option<Output> {
        let (meta, payload) = match rest.iter().position(|&b| b == b';') {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => (rest, &[][..]),
        };
        let meta = Meta::parse(std::str::from_utf8(meta).ok()?)?;
        match meta.payload {
            // `i` の無い `p=close` は何もしない（仕様）。
            Payload::Close => return (!meta.id.is_empty()).then_some(Output::Close(meta.id)),
            Payload::Query => return Some(Output::Query(meta.id)),
            Payload::Alive => return Some(Output::Alive(meta.id)),
            // アイコンの画像は読まない。
            Payload::Icon => return None,
            _ => {}
        }
        let idx = self.pending_for(&meta.id);
        let p = &mut self.pending[idx];
        p.append(meta.payload, meta.base64, payload);
        p.merge_meta(&meta);
        if !meta.done {
            return None;
        }
        self.pending.remove(idx).finish()
    }

    /// `id` の組み立て中の通知の位置。無ければ作る（上限を超えるときは古いものを捨てる）。
    fn pending_for(&mut self, id: &str) -> usize {
        if let Some(i) = self.pending.iter().position(|p| p.id == id) {
            return i;
        }
        if self.pending.len() >= MAX_PENDING {
            self.pending.remove(0);
        }
        self.pending.push(Pending {
            id: id.to_string(),
            ..Default::default()
        });
        self.pending.len() - 1
    }
}

impl Pending {
    /// 題名・本文・ボタンのどれかに中身を足す。大きさの上限を超えたら捨てる印をつける。
    fn append(&mut self, kind: Payload, base64: Option<bool>, payload: &[u8]) {
        let (buf, flag) = match kind {
            Payload::Title => (&mut self.title, &mut self.title_b64),
            Payload::Body => (&mut self.body, &mut self.body_b64),
            _ => (&mut self.buttons, &mut self.buttons_b64),
        };
        if let Some(b) = base64 {
            *flag = b;
        }
        buf.extend_from_slice(payload);
        if self.title.len() + self.body.len() + self.buttons.len() > MAX_PAYLOAD {
            self.too_big = true;
            self.title.clear();
            self.body.clear();
            self.buttons.clear();
        }
    }

    /// 塊の鍵を通知に反映する（後の塊の鍵が上書き）。
    fn merge_meta(&mut self, meta: &Meta) {
        let n = &mut self.n;
        if let Some(a) = meta.actions {
            n.actions = a;
        }
        if let Some(w) = meta.when {
            n.when = w;
        }
        if meta.urgency.is_some() {
            n.urgency = meta.urgency;
        }
        if let Some(c) = meta.report_close {
            n.report_close = c;
        }
        if meta.timeout.is_some() {
            n.timeout = meta.timeout;
        }
        if let Some(f) = &meta.app_name {
            n.app_name = Some(String::from_utf8_lossy(f).into_owned());
        }
        if let Some(s) = &meta.sound {
            n.sound = Some(String::from_utf8_lossy(s).into_owned());
        }
        if meta.icon_cache_id.is_some() {
            n.icon_cache_id = meta.icon_cache_id.clone();
        }
        for t in &meta.types {
            n.types.push(String::from_utf8_lossy(t).into_owned());
        }
        for t in &meta.icon_names {
            n.icon_names.push(String::from_utf8_lossy(t).into_owned());
        }
    }

    fn finish(self) -> Option<Output> {
        if self.too_big {
            return None;
        }
        let text = |raw: Vec<u8>, b64: bool| -> Option<String> {
            let bytes = if b64 { decode_base64(&raw)? } else { raw };
            Some(String::from_utf8_lossy(&bytes).into_owned())
        };
        let mut n = self.n;
        n.id = self.id;
        n.title = text(self.title, self.title_b64)?;
        n.body = text(self.body, self.body_b64)?;
        let buttons = text(self.buttons, self.buttons_b64)?;
        n.buttons = if buttons.is_empty() {
            Vec::new()
        } else {
            buttons.split(BUTTON_SEP).map(str::to_string).collect()
        };
        if n.title.is_empty() && n.body.is_empty() {
            return None;
        }
        Some(Output::Notification(Box::new(n)))
    }
}

/// base64（標準の文字、`=` は有っても無くてもよい）を戻す。途中の `=`（塊ごとの詰め物）は、そこで並びが
/// 終わったものとして読む。読めなければ `None`。
fn decode_base64(src: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(src.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in src {
        if c == b'=' {
            // 塊ごとの詰め物。ここで並びが終わるので、溜まりを捨てて続きを新しい並びとして読む。
            if bits >= 6 {
                return None;
            }
            (acc, bits) = (0, 0);
            continue;
        }
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if bits >= 6 {
        return None;
    }
    Some(out)
}

// ---- PTY へ送る返事 ----

/// 通知が押された。`button` はボタンを押したときの番号（1 始まり）。
/// `i=` の無い通知には、仕様どおり `i=0` で返す。
pub fn activated(id: &str, button: Option<u32>) -> Vec<u8> {
    let id = if id.is_empty() { "0" } else { id };
    if !valid_id(id) {
        return Vec::new();
    }
    match button {
        Some(b) => format!("\x1b]99;i={id};{b}\x1b\\"),
        None => format!("\x1b]99;i={id};\x1b\\"),
    }
    .into_bytes()
}

/// 通知が閉じられた（`c=1` の通知にだけ送る）。
pub fn closed(id: &str) -> Vec<u8> {
    let id = if id.is_empty() { "0" } else { id };
    if !valid_id(id) {
        return Vec::new();
    }
    format!("\x1b]99;i={id}:p=close;\x1b\\").into_bytes()
}

fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '.' | '?'))
        .collect()
}

fn list<T: AsRef<str>>(key: &str, items: &[T]) -> Option<String> {
    let v: Vec<String> = items.iter().map(|s| clean(s.as_ref())).collect();
    let v: Vec<&str> = v
        .iter()
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .collect();
    (!v.is_empty()).then(|| format!("{key}={}", v.join(",")))
}

/// `p=?` への返事。
pub fn query_reply(id: &str, s: &NotificationSupport) -> Vec<u8> {
    if !id.is_empty() && !valid_id(id) {
        return Vec::new();
    }
    let urgencies: Vec<String> = s.urgencies.iter().map(u8::to_string).collect();
    let parts: Vec<Option<String>> = vec![
        list("a", &s.actions),
        s.close_report.then(|| "c=1".to_string()),
        list("o", &s.when).or_else(|| Some("o=always".to_string())),
        list("p", &s.payloads),
        list("s", &s.sounds),
        list("t", &s.types),
        list("u", &urgencies),
        s.timeout.then(|| "w=1".to_string()),
    ];
    let body = parts.into_iter().flatten().collect::<Vec<_>>().join(":");
    let head = if id.is_empty() {
        "p=?".to_string()
    } else {
        format!("i={id}:p=?")
    };
    format!("\x1b]99;{head};{body}\x1b\\").into_bytes()
}

/// `p=alive` への返事。`ids` はまだ生きている通知の `i`。
pub fn alive_reply(id: &str, ids: &[String]) -> Vec<u8> {
    if !id.is_empty() && !valid_id(id) {
        return Vec::new();
    }
    let ids: Vec<&str> = ids
        .iter()
        .map(String::as_str)
        .filter(|i| valid_id(i))
        .collect();
    let head = if id.is_empty() {
        "p=alive".to_string()
    } else {
        format!("i={id}:p=alive")
    };
    format!("\x1b]99;{head};{}\x1b\\", ids.join(",")).into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::{Event, Terminal};

    fn events(input: &str) -> (Terminal, Vec<Event>) {
        let mut t = Terminal::new(5, 20);
        t.write(input.as_bytes());
        let ev = t.take_events();
        (t, ev)
    }

    fn one(input: &str) -> Notification {
        match events(input).1.as_slice() {
            [Event::Notification(n)] => (**n).clone(),
            other => panic!("expected one notification, got {other:?}"),
        }
    }

    #[test]
    fn simple_title() {
        let n = one("\x1b]99;;Hello world\x1b\\");
        assert_eq!(n.title, "Hello world");
        assert_eq!(n.body, "");
        assert_eq!(n.id, "");
        assert_eq!(n.actions, Actions::default());
        assert_eq!(n.when, When::Always);
        assert_eq!(n.urgency, None);
    }

    #[test]
    fn title_may_contain_semicolon_and_bel_terminates() {
        assert_eq!(one("\x1b]99;;a;b\x07").title, "a;b");
    }

    #[test]
    fn chunked_title_and_body_make_one_event() {
        // 仕様の例: 題名を d=0 で送り、本文で終わる。
        let (_, ev) = events("\x1b]99;i=1:d=0;Hello\x1b\\\x1b]99;i=1:p=body;This is a body\x1b\\");
        assert_eq!(ev.len(), 1);
        let Event::Notification(n) = &ev[0] else {
            panic!()
        };
        assert_eq!((n.id.as_str(), n.title.as_str()), ("1", "Hello"));
        assert_eq!(n.body, "This is a body");
    }

    #[test]
    fn no_event_until_done() {
        assert!(events("\x1b]99;i=1:d=0;Hel\x1b\\").1.is_empty());
    }

    #[test]
    fn title_chunks_are_concatenated() {
        let n = one("\x1b]99;i=a:d=0;Hel\x1b\\\x1b]99;i=a:d=0;lo \x1b\\\x1b]99;i=a;world\x1b\\");
        assert_eq!(n.title, "Hello world");
    }

    #[test]
    fn chunks_without_id_are_one_notification() {
        let n = one("\x1b]99;d=0;Hel\x1b\\\x1b]99;;lo\x1b\\");
        assert_eq!(n.title, "Hello");
    }

    #[test]
    fn interleaved_ids_stay_separate() {
        let (_, ev) = events("\x1b]99;i=a:d=0;A\x1b\\\x1b]99;i=b;B\x1b\\\x1b]99;i=a;1\x1b\\");
        let titles: Vec<_> = ev
            .iter()
            .map(|e| match e {
                Event::Notification(n) => n.title.clone(),
                _ => panic!(),
            })
            .collect();
        assert_eq!(titles, ["B", "A1"]);
    }

    #[test]
    fn keys_are_read() {
        let n = one(
            "\x1b]99;i=x:u=2:a=-focus,report:o=unfocused:c=1:w=500:s=aW5mbw==:g=ic1:f=a2l0dHk=:t=aW0=:n=Zm9v;hi\x1b\\",
        );
        assert_eq!(n.id, "x");
        assert_eq!(n.urgency, Some(2));
        assert_eq!(
            n.actions,
            Actions {
                focus: false,
                report: true
            }
        );
        assert_eq!(n.when, When::Unfocused);
        assert!(n.report_close);
        assert_eq!(n.timeout, Some(500));
        assert_eq!(n.sound.as_deref(), Some("info"));
        assert_eq!(n.icon_cache_id.as_deref(), Some("ic1"));
        assert_eq!(n.app_name.as_deref(), Some("kitty"));
        assert_eq!(n.types, ["im"]);
        assert_eq!(n.icon_names, ["foo"]);
    }

    #[test]
    fn base64_payload() {
        // "Hello, 世界" in base64.
        let n = one("\x1b]99;e=1;SGVsbG8sIOS4lueVjA==\x1b\\");
        assert_eq!(n.title, "Hello, 世界");
        // 分けて送り、後の塊に e=1 が無くても、そろってから戻す。
        let n = one("\x1b]99;i=q:e=1:d=0;SGVsbG8s\x1b\\\x1b]99;i=q;IOS4lueVjA==\x1b\\");
        assert_eq!(n.title, "Hello, 世界");
    }

    #[test]
    fn padded_chunks_are_decoded() {
        // エンコードしてから分割: 塊ごとに詰め物が付く。
        let n = one("\x1b]99;i=p:e=1:d=0;SGVsbG8=\x1b\\\x1b]99;i=p:e=1;IHdvcmxk\x1b\\");
        assert_eq!(n.title, "Hello world");
        let n = one("\x1b]99;i=p:e=1:d=0;SGVsbG8=\x1b\\\x1b]99;i=p:e=1:d=0;IHdvcmxk\x1b\\\x1b]99;i=p;\x1b\\");
        assert_eq!(n.title, "Hello world");
    }

    #[test]
    fn close_without_id_is_noop() {
        assert!(events("\x1b]99;p=close;\x1b\\").1.is_empty());
    }

    #[test]
    fn query_reply_defaults_when() {
        let mut t = Terminal::new(5, 20);
        t.notification_query_reply("", &NotificationSupport::default());
        assert_eq!(
            String::from_utf8(t.take_output()).unwrap(),
            "\x1b]99;p=?;o=always\x1b\\"
        );
    }

    #[test]
    fn bad_base64_is_dropped() {
        assert!(events("\x1b]99;e=1;!!!\x1b\\").1.is_empty());
    }

    #[test]
    fn buttons() {
        let n = one("\x1b]99;i=b:d=0;T\x1b\\\x1b]99;i=b:p=buttons;Yes\u{2028}No\x1b\\");
        assert_eq!(n.title, "T");
        assert_eq!(n.buttons, ["Yes", "No"]);
    }

    #[test]
    fn empty_and_invalid_are_dropped() {
        assert!(events("\x1b]99;;\x1b\\").1.is_empty());
        assert!(events("\x1b]99;i=a b;x\x1b\\").1.is_empty());
        assert!(events("\x1b]99;i=\x1b;x\x1b\\").1.is_empty());
        assert!(events("\x1b]99;p=bogus;x\x1b\\").1.is_empty());
        assert!(events("\x1b]99\x1b\\").1.is_empty());
        // 読めない値は無視して通知は出す。
        let n = one("\x1b]99;u=9:o=never:w=-5;x\x1b\\");
        assert_eq!((n.urgency, n.when, n.timeout), (None, When::Always, None));
    }

    #[test]
    fn icon_payload_is_ignored() {
        assert!(events("\x1b]99;p=icon:i=1;AAAA\x1b\\").1.is_empty());
    }

    #[test]
    fn too_big_is_dropped() {
        let big = "x".repeat(MAX_PAYLOAD + 1);
        assert!(events(&format!("\x1b]99;;{big}\x1b\\")).1.is_empty());
    }

    #[test]
    fn close_query_alive_become_events() {
        let (_, ev) =
            events("\x1b]99;i=7:p=close;\x1b\\\x1b]99;i=8:p=?;\x1b\\\x1b]99;p=alive;\x1b\\");
        assert_eq!(
            ev,
            [
                Event::NotificationClose { id: "7".into() },
                Event::NotificationQuery { id: "8".into() },
                Event::NotificationAlive { id: String::new() },
            ]
        );
    }

    #[test]
    fn registered_osc_99_goes_to_handler() {
        let mut t = Terminal::new(5, 20);
        t.register_osc(99);
        t.write(b"\x1b]99;;hi\x1b\\");
        assert!(matches!(
            t.take_events().as_slice(),
            [Event::Osc { ident: 99, .. }]
        ));
    }

    #[test]
    fn replies() {
        let mut t = Terminal::new(5, 20);
        t.notification_activated("1", None);
        t.notification_activated("1", Some(2));
        t.notification_closed("1");
        t.notification_alive_reply("9", &["a".into(), "b c".into(), "d".into()]);
        t.notification_activated("bad\x1b", None);
        t.notification_activated("", None);
        t.notification_closed("");
        assert_eq!(
            String::from_utf8(t.take_output()).unwrap(),
            "\x1b]99;i=1;\x1b\\\x1b]99;i=1;2\x1b\\\x1b]99;i=1:p=close;\x1b\\\x1b]99;i=9:p=alive;a,d\x1b\\\x1b]99;i=0;\x1b\\\x1b]99;i=0:p=close;\x1b\\"
        );
    }

    #[test]
    fn query_reply_shape() {
        let mut t = Terminal::new(5, 20);
        let s = NotificationSupport {
            actions: vec!["focus".into(), "report".into()],
            when: vec!["always".into(), "unfocused".into(), "invisible".into()],
            payloads: vec!["title".into(), "body".into(), "?".into(), "close".into()],
            urgencies: vec![0, 1, 2],
            close_report: true,
            timeout: false,
            sounds: vec![],
            types: vec![],
        };
        t.notification_query_reply("q", &s);
        assert_eq!(
            String::from_utf8(t.take_output()).unwrap(),
            "\x1b]99;i=q:p=?;a=focus,report:c=1:o=always,unfocused,invisible:p=title,body,?,close:u=0,1,2\x1b\\"
        );
    }

    #[test]
    fn reset_drops_pending() {
        let mut t = Terminal::new(5, 20);
        t.write(b"\x1b]99;i=1:d=0;a\x1b\\\x1bc\x1b]99;i=1;b\x1b\\");
        let ev = t.take_events();
        let [Event::Notification(n)] = ev.as_slice() else {
            panic!()
        };
        assert_eq!(n.title, "b");
    }
}
