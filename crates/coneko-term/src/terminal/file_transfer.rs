//! kitty の file transfer protocol（OSC 5113）。
//!
//! 仕様: <https://sw.kovidgoyal.net/kitty/file-transfer-protocol/>
//!
//! ここでは、クライアント（ssh 先のプログラム）が送る命令を読んでイベント
//! （[`crate::Event::FileTransfer`]）にするところと、その答え（`ac=status`・`ac=file`・`ac=data`・
//! `ac=end_data`）を PTY へ出すバイト列にするところまでを行う。許すかどうかの判断、ファイルの
//! 読み書き、パスの検査は呼ぶ側（GUI）がする。coneko-term はファイルを読まず、書かず、`zip=zlib`
//! の圧縮も読み解かない（バイト列のまま渡す）。rsync の署名と差分（`tt=rsync`）もバイト列のまま
//! 渡し、`tt` の値だけを伝える。
//!
//! - 読むのは `ac=send`・`receive`・`file`・`data`・`end_data`・`finish`・`cancel`。`ac=status` など
//!   クライアントから来ないものは捨てる。
//! - 鍵や値が知らないもの・読めないものは捨てる（`ac` が無い・知らない、`id` が空か `safe_string`
//!   でない、`file` に `fid` か `n` が無い、base64 や数が読めない、`ft`・`zip`・`tt`・`q` が知らない値）。
//!   知らない鍵は仕様どおり無視する。捨てたものに返事は出さない。
//! - `q`（quiet）は、`send`・`receive` で受けた値をセッションの `id` ごとに覚え（新しい 32 個まで）、
//!   [`status`] の返事から省く。`q=1` は成功の知らせ（`OK`・`STARTED`・`PROGRESS`・`CANCELED`）を、
//!   `q=2` はエラーも省く。データとファイルの一覧は省かない。

use std::collections::VecDeque;

use super::clipboard::{decode_base64, encode_base64};
use super::Event;

/// 1 つのパケットに入れるデータの大きさ（base64 にする前、バイト）。仕様の上限。
const CHUNK: usize = 4096;
/// quiet を覚えるセッションの数の上限。
const MAX_SESSIONS: usize = 32;

/// ファイルの種類（`ft`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum FileType {
    /// 普通のファイル（`regular`。`ft` が無いとき）。
    #[default]
    Regular,
    /// ディレクトリ。
    Directory,
    /// シンボリックリンク。
    Symlink,
    /// ハードリンク（`link`）。
    Link,
}

impl FileType {
    fn parse(s: &str) -> Option<FileType> {
        Some(match s {
            "regular" => FileType::Regular,
            "directory" => FileType::Directory,
            "symlink" => FileType::Symlink,
            "link" => FileType::Link,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            FileType::Regular => "regular",
            FileType::Directory => "directory",
            FileType::Symlink => "symlink",
            FileType::Link => "link",
        }
    }
}

/// データの圧縮の印（`zip`）。coneko-term は圧縮を読み解かない。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum FileCompression {
    /// 圧縮しない（`none`。`zip` が無いとき）。
    #[default]
    None,
    /// zlib（RFC 1950）。
    Zlib,
}

/// 送り方（`tt`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum FileTransmission {
    /// ファイルのデータをそのまま送る（`simple`。`tt` が無いとき）。
    #[default]
    Simple,
    /// rsync の署名と差分を送る。
    Rsync,
}

/// `ac=send`・`ac=receive` が運ぶ、セッションの始まり。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileTransferStart {
    /// セッションの `id`。
    pub id: String,
    /// `pw` を戻した文字列（仕様では `sha256:<hash>`）。無ければ `None`。比べるのは呼ぶ側。
    pub bypass: Option<String>,
    /// `q`（0 が全部返す、1 がエラーだけ、2 が何も返さない）。返事から省くのは coneko-term がする。
    pub quiet: u8,
    /// `sz`。`receive` が頼むパスの数。
    pub paths: Option<u64>,
}

/// `ac=file`: 送る側が決めたファイルの情報、または受け取る側が頼むパス。向きはセッションの種類で決まる。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileTransferFile {
    /// セッションの `id`。
    pub id: String,
    /// `fid`。
    pub file_id: String,
    /// `n`（戻したパス。UTF-8）。
    pub name: String,
    /// `ft`。
    pub file_type: FileType,
    /// `zip`。
    pub compression: FileCompression,
    /// `tt`。
    pub transmission: FileTransmission,
    /// `mod`（ナノ秒。無ければ `None`）。
    pub mtime: Option<i64>,
    /// `prm`（無ければ `None`）。
    pub permissions: Option<u32>,
    /// `sz`（無ければ `None`）。
    pub size: Option<u64>,
    /// `pr`。
    pub parent: Option<String>,
}

/// `ac=data`・`ac=end_data`: ファイルのデータの 1 つのパケット。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileTransferData {
    /// セッションの `id`。
    pub id: String,
    /// `fid`。
    pub file_id: String,
    /// `d` を戻したバイト列（無ければ空）。圧縮や rsync の印があっても、そのまま。
    pub data: Vec<u8>,
    /// `end_data` か（そのファイルのデータの終わり）。
    pub end: bool,
}

/// クライアントが送った命令。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FileTransferEvent {
    /// `ac=send`: クライアントがファイルを送りたい。許すなら [`crate::Terminal::file_transfer_ok`]、
    /// 断るなら [`crate::Terminal::file_transfer_refuse`] で答える。答えが来るまで、クライアントは
    /// 次の命令を送らない。
    Send(FileTransferStart),
    /// `ac=receive`: クライアントがファイルを受け取りたい。答え方は `Send` と同じ。
    Receive(FileTransferStart),
    /// `ac=file`。
    File(FileTransferFile),
    /// `ac=data`・`ac=end_data`。
    Data(FileTransferData),
    /// `ac=finish`: クライアントがセッションを閉じた。
    Finish {
        /// セッションの `id`。
        id: String,
    },
    /// `ac=cancel`: クライアントが取り消した。[`crate::Terminal::file_transfer_canceled`] で答える。
    Cancel {
        /// セッションの `id`。
        id: String,
    },
}

/// 状態の返事のエラー（POSIX の記号名）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FileTransferError {
    /// 許されない（利用者が断った、など）。
    Eperm,
    /// 無い。
    Enoent,
    /// すでにある。
    Eexist,
    /// ディレクトリだった。
    Eisdir,
    /// ディレクトリではなかった。
    Enotdir,
    /// 権限が無い。
    Eacces,
    /// 不正。
    Einval,
    /// 入出力のエラー。
    Eio,
    /// 大きすぎる。
    Efbig,
    /// 空きが無い。
    Enospc,
}

impl FileTransferError {
    fn code(self) -> &'static str {
        match self {
            FileTransferError::Eperm => "EPERM",
            FileTransferError::Enoent => "ENOENT",
            FileTransferError::Eexist => "EEXIST",
            FileTransferError::Eisdir => "EISDIR",
            FileTransferError::Enotdir => "ENOTDIR",
            FileTransferError::Eacces => "EACCES",
            FileTransferError::Einval => "EINVAL",
            FileTransferError::Eio => "EIO",
            FileTransferError::Efbig => "EFBIG",
            FileTransferError::Enospc => "ENOSPC",
        }
    }
}

/// 受け取りの一覧（`ac=file`）の 1 つの項目。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileTransferListing {
    /// クライアントの頼み（`ac=file`）の `fid`。ディレクトリを広げた項目は、頼みと同じ値。
    /// `[0-9A-Za-z_:./@-]` の字だけであること（使えない字を含むと、答えの `fid` を出さない）。
    pub request_id: String,
    /// この項目自身の `fid`（`st` に入る）。`[0-9A-Za-z_:./@-]` の字だけであること。
    pub entry_id: String,
    /// 絶対パス。
    pub name: String,
    /// 種類。
    pub file_type: FileType,
    /// 更新時刻（ナノ秒）。
    pub mtime: Option<i64>,
    /// 権限のビット。
    pub permissions: Option<u32>,
    /// 大きさ（バイト）。
    pub size: Option<u64>,
    /// 入っているディレクトリの `entry_id`。
    pub parent: Option<String>,
    /// リンクの先（ハードリンクとシンボリックリンクだけ。一覧にある先なら、その `entry_id`。
    /// シンボリックリンクで一覧に無ければ、リンクの先のパス）。
    pub data: Option<Vec<u8>>,
}

/// 覚えているセッションの quiet。
#[derive(Debug, Default)]
pub(super) struct FileTransfer {
    quiet: VecDeque<(String, u8)>,
}

impl FileTransfer {
    pub(super) fn clear(&mut self) {
        self.quiet.clear();
    }

    fn remember(&mut self, id: &str, quiet: u8) {
        self.quiet.retain(|(i, _)| i != id);
        if self.quiet.len() >= MAX_SESSIONS {
            self.quiet.pop_front();
        }
        self.quiet.push_back((id.to_string(), quiet));
    }

    fn quiet_of(&self, id: &str) -> u8 {
        self.quiet
            .iter()
            .find(|(i, _)| i == id)
            .map_or(0, |&(_, q)| q)
    }

    /// `OSC 5113 ;` のあとの `鍵=値 ; …` を読む。
    pub(super) fn osc(&mut self, rest: &[u8]) -> Option<Event> {
        let ev = parse(rest)?;
        if let FileTransferEvent::Send(s) | FileTransferEvent::Receive(s) = &ev {
            self.remember(&s.id, s.quiet);
        }
        Some(Event::FileTransfer(Box::new(ev)))
    }

    /// 状態の返事。`status` は `OK`・`STARTED`・`PROGRESS`・`CANCELED`、またはエラー。
    /// quiet に従って省くと空を返す。
    pub(super) fn status(&self, reply: &Status) -> Vec<u8> {
        let quiet = self.quiet_of(reply.id);
        let is_error = reply.error.is_some();
        if quiet >= 2 || (quiet == 1 && !is_error) {
            return Vec::new();
        }
        let mut p = Packet::new("status", reply.id);
        p.safe("fid", reply.file_id);
        let st = match reply.error {
            Some((e, msg)) if msg.is_empty() => e.code().to_string(),
            Some((e, msg)) => format!("{}:{msg}", e.code()),
            None => reply.text.to_string(),
        };
        p.b64("st", st.as_bytes());
        if let Some(n) = reply.size {
            p.raw("sz", &n.to_string());
        }
        if reply.rsync {
            p.raw("tt", "rsync");
        }
        if let Some(name) = reply.name {
            p.b64("n", name.as_bytes());
        }
        p.finish()
    }
}

/// 状態の返事の中身。
pub(super) struct Status<'a> {
    pub id: &'a str,
    pub file_id: Option<&'a str>,
    /// エラーでないときの `st`。
    pub text: &'a str,
    pub error: Option<(FileTransferError, &'a str)>,
    pub size: Option<u64>,
    pub rsync: bool,
    pub name: Option<&'a str>,
}

/// 組み立て中の `ESC ] 5113 ; ac=… ; … ESC \`。
struct Packet(String);

impl Packet {
    fn new(action: &str, id: &str) -> Packet {
        let mut p = Packet(format!("\x1b]5113;ac={action}"));
        p.safe("id", Some(id));
        p
    }

    fn raw(&mut self, key: &str, value: &str) {
        self.0.push(';');
        self.0.push_str(key);
        self.0.push('=');
        self.0.push_str(value);
    }

    /// `safe_string` の鍵。使えない字を含む値は出さない。
    fn safe(&mut self, key: &str, value: Option<&str>) {
        if let Some(v) = value.filter(|v| is_safe(v)) {
            self.raw(key, v);
        }
    }

    fn b64(&mut self, key: &str, value: &[u8]) {
        if !value.is_empty() {
            self.raw(key, &encode_base64(value));
        }
    }

    fn finish(mut self) -> Vec<u8> {
        self.0.push_str("\x1b\\");
        self.0.into_bytes()
    }
}

/// 受け取りの一覧の 1 項目。
pub(super) fn listing(id: &str, l: &FileTransferListing) -> Vec<u8> {
    let mut p = Packet::new("file", id);
    p.safe("fid", Some(&l.request_id));
    // `st` は base64_string。一覧では、この項目自身の `fid` が入る。
    p.b64("st", l.entry_id.as_bytes());
    p.b64("n", l.name.as_bytes());
    p.raw("ft", l.file_type.name());
    if let Some(m) = l.mtime {
        p.raw("mod", &m.to_string());
    }
    if let Some(m) = l.permissions {
        p.raw("prm", &m.to_string());
    }
    if let Some(s) = l.size {
        p.raw("sz", &s.to_string());
    }
    p.safe("pr", l.parent.as_deref());
    if let Some(d) = &l.data {
        p.b64("d", d);
    }
    p.finish()
}

/// データを 4096 バイトごとのパケットにする。`last` なら、最後のパケットを `end_data` にする
/// （データが空なら、データの無い `end_data` 1 つ）。
pub(super) fn data(id: &str, file_id: &str, bytes: &[u8], last: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chunks: Vec<&[u8]> = bytes.chunks(CHUNK).collect();
    if last && chunks.is_empty() {
        chunks.push(&[]);
    }
    let n = chunks.len();
    for (i, c) in chunks.into_iter().enumerate() {
        let action = if last && i + 1 == n {
            "end_data"
        } else {
            "data"
        };
        let mut p = Packet::new(action, id);
        p.safe("fid", Some(file_id));
        p.b64("d", c);
        out.extend_from_slice(&p.finish());
    }
    out
}

// ---- 読む ----

fn is_safe(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'.' | b'/' | b'@' | b'-'))
}

#[derive(Default)]
struct Fields<'a> {
    ac: &'a str,
    zip: Option<&'a str>,
    ft: Option<&'a str>,
    tt: Option<&'a str>,
    id: Option<&'a str>,
    fid: Option<&'a str>,
    pw: Option<&'a str>,
    q: Option<&'a str>,
    mtime: Option<&'a str>,
    prm: Option<&'a str>,
    sz: Option<&'a str>,
    n: Option<&'a str>,
    pr: Option<&'a str>,
    d: Option<&'a str>,
}

/// 10 進の整数（先頭の `-` は 1 つ）。
fn int(s: &str) -> Option<i64> {
    let digits = s.strip_prefix('-').unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// 省かれたとき、または `-1`（指定なし）のとき `Some(None)`。読めなければ `None`。
fn opt_int(s: Option<&str>) -> Option<Option<i64>> {
    match s {
        None => Some(None),
        Some(s) => int(s).map(|n| (n != -1).then_some(n)),
    }
}

fn opt_unsigned(s: Option<&str>) -> Option<Option<u64>> {
    Some(opt_int(s)?.and_then(|n| u64::try_from(n).ok()))
}

fn safe_id(s: Option<&str>) -> Option<String> {
    s.filter(|s| !s.is_empty() && is_safe(s))
        .map(str::to_string)
}

impl<'a> Fields<'a> {
    /// `鍵=値 ; …` を読む。知らない鍵は無視する。
    fn parse(text: &'a str) -> Fields<'a> {
        let mut f = Fields::default();
        for kv in text.split(';') {
            let Some((k, v)) = kv.split_once('=') else {
                continue;
            };
            match k {
                "ac" => f.ac = v,
                "zip" => f.zip = Some(v),
                "ft" => f.ft = Some(v),
                "tt" => f.tt = Some(v),
                "id" => f.id = Some(v),
                "fid" => f.fid = Some(v),
                "pw" => f.pw = Some(v),
                "q" => f.q = Some(v),
                "mod" => f.mtime = Some(v),
                "prm" => f.prm = Some(v),
                "sz" => f.sz = Some(v),
                "n" => f.n = Some(v),
                "pr" => f.pr = Some(v),
                "d" => f.d = Some(v),
                _ => {}
            }
        }
        f
    }

    /// `ac=send`・`ac=receive`。
    fn start(&self, id: String) -> Option<FileTransferStart> {
        let bypass = match self.pw {
            None => None,
            Some(pw) => Some(String::from_utf8(decode_base64(pw.as_bytes())?).ok()?),
        };
        let quiet = match self.q {
            None | Some("0") => 0,
            Some("1") => 1,
            Some("2") => 2,
            Some(_) => return None,
        };
        Some(FileTransferStart {
            id,
            bypass,
            quiet,
            paths: opt_unsigned(self.sz)?,
        })
    }

    /// `ac=file`。
    fn file(&self, id: String) -> Option<FileTransferFile> {
        let name = String::from_utf8(decode_base64(self.n?.as_bytes())?).ok()?;
        let parent = match self.pr {
            None => None,
            Some(p) => Some(safe_id(Some(p))?),
        };
        Some(FileTransferFile {
            id,
            file_id: safe_id(self.fid)?,
            name,
            file_type: match self.ft {
                None => FileType::Regular,
                Some(s) => FileType::parse(s)?,
            },
            compression: match self.zip {
                None | Some("none") => FileCompression::None,
                Some("zlib") => FileCompression::Zlib,
                Some(_) => return None,
            },
            transmission: match self.tt {
                None | Some("simple") => FileTransmission::Simple,
                Some("rsync") => FileTransmission::Rsync,
                Some(_) => return None,
            },
            mtime: opt_int(self.mtime)?,
            permissions: opt_int(self.prm)?.and_then(|n| u32::try_from(n).ok()),
            size: opt_unsigned(self.sz)?,
            parent,
        })
    }

    /// `ac=data`・`ac=end_data`。
    fn data(&self, id: String, end: bool) -> Option<FileTransferData> {
        Some(FileTransferData {
            id,
            file_id: safe_id(self.fid)?,
            data: match self.d {
                None => Vec::new(),
                Some(d) => decode_base64(d.as_bytes())?,
            },
            end,
        })
    }
}

fn parse(rest: &[u8]) -> Option<FileTransferEvent> {
    let f = Fields::parse(std::str::from_utf8(rest).ok()?);
    let id = safe_id(f.id)?;
    Some(match f.ac {
        "send" => FileTransferEvent::Send(f.start(id)?),
        "receive" => FileTransferEvent::Receive(f.start(id)?),
        "file" => FileTransferEvent::File(f.file(id)?),
        "data" => FileTransferEvent::Data(f.data(id, false)?),
        "end_data" => FileTransferEvent::Data(f.data(id, true)?),
        "finish" => FileTransferEvent::Finish { id },
        "cancel" => FileTransferEvent::Cancel { id },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::Terminal;

    fn b64(s: &str) -> String {
        encode_base64(s.as_bytes())
    }

    fn run(input: &str) -> (Terminal, Vec<FileTransferEvent>) {
        let mut t = Terminal::new(5, 20);
        t.write(input.as_bytes());
        let evs = t
            .take_events()
            .into_iter()
            .filter_map(|e| match e {
                Event::FileTransfer(f) => Some(*f),
                _ => None,
            })
            .collect();
        (t, evs)
    }

    fn osc(body: &str) -> String {
        format!("\x1b]5113;{body}\x1b\\")
    }

    fn out(t: &mut Terminal) -> String {
        String::from_utf8(t.take_output()).unwrap()
    }

    #[test]
    fn spec_example_serialization() {
        let (_, evs) = run(&osc("ac=send;id=test;n=c29tZWZpbGU=;sz=3;d=AQID"));
        assert_eq!(
            evs,
            vec![FileTransferEvent::Send(FileTransferStart {
                id: "test".into(),
                bypass: None,
                quiet: 0,
                paths: Some(3),
            })]
        );
    }

    #[test]
    fn send_session_runs_to_the_end() {
        let input = [
            osc("ac=send;id=s1"),
            osc(&format!(
                "ac=file;id=s1;fid=f1;n={};mod=1700000000000000000;prm=420;zip=zlib;tt=rsync",
                b64("/tmp/a.txt")
            )),
            osc(&format!(
                "ac=file;id=s1;fid=f2;n={};ft=directory",
                b64("~/d")
            )),
            osc("ac=data;id=s1;fid=f1;d=AQID"),
            osc("ac=end_data;id=s1;fid=f1;d=BAU="),
            osc("ac=finish;id=s1"),
        ]
        .concat();
        let (mut t, evs) = run(&input);
        assert_eq!(evs.len(), 6);
        assert!(matches!(&evs[0], FileTransferEvent::Send(s) if s.id == "s1"));
        assert_eq!(
            evs[1],
            FileTransferEvent::File(FileTransferFile {
                id: "s1".into(),
                file_id: "f1".into(),
                name: "/tmp/a.txt".into(),
                file_type: FileType::Regular,
                compression: FileCompression::Zlib,
                transmission: FileTransmission::Rsync,
                mtime: Some(1_700_000_000_000_000_000),
                permissions: Some(420),
                size: None,
                parent: None,
            })
        );
        assert!(matches!(
            &evs[2],
            FileTransferEvent::File(f) if f.file_type == FileType::Directory && f.name == "~/d"
        ));
        assert_eq!(
            evs[3],
            FileTransferEvent::Data(FileTransferData {
                id: "s1".into(),
                file_id: "f1".into(),
                data: vec![1, 2, 3],
                end: false,
            })
        );
        assert!(matches!(&evs[4], FileTransferEvent::Data(d) if d.end && d.data == [4, 5]));
        assert_eq!(evs[5], FileTransferEvent::Finish { id: "s1".into() });

        // 答え
        t.file_transfer_ok("s1", None, None, None);
        t.file_transfer_started("s1", "f1", true);
        t.file_transfer_ok("s1", Some("f2"), None, None);
        t.file_transfer_progress("s1", "f1", 3);
        t.file_transfer_ok("s1", Some("f1"), Some(5), None);
        t.file_transfer_error("s1", Some("f1"), FileTransferError::Eio, "Failed to write");
        assert_eq!(
            out(&mut t),
            [
                osc(&format!("ac=status;id=s1;st={}", b64("OK"))),
                osc(&format!(
                    "ac=status;id=s1;fid=f1;st={};tt=rsync",
                    b64("STARTED")
                )),
                osc(&format!("ac=status;id=s1;fid=f2;st={}", b64("OK"))),
                osc(&format!(
                    "ac=status;id=s1;fid=f1;st={};sz=3",
                    b64("PROGRESS")
                )),
                osc(&format!("ac=status;id=s1;fid=f1;st={};sz=5", b64("OK"))),
                osc(&format!(
                    "ac=status;id=s1;fid=f1;st={}",
                    b64("EIO:Failed to write")
                )),
            ]
            .concat()
        );
    }

    #[test]
    fn receive_session_runs_to_the_end() {
        let input = [
            osc("ac=receive;id=r1;sz=1"),
            osc(&format!("ac=file;id=r1;fid=f1;n={}", b64("/some/dir"))),
        ]
        .concat();
        let (mut t, evs) = run(&input);
        assert!(matches!(&evs[0], FileTransferEvent::Receive(s) if s.paths == Some(1)));
        assert!(matches!(&evs[1], FileTransferEvent::File(f) if f.name == "/some/dir"));

        t.file_transfer_ok("r1", None, None, None);
        t.file_transfer_listing(
            "r1",
            &FileTransferListing {
                request_id: "f1".into(),
                entry_id: "e1".into(),
                name: "/some/dir".into(),
                file_type: FileType::Directory,
                mtime: Some(5),
                permissions: Some(493),
                size: None,
                parent: None,
                data: None,
            },
        );
        t.file_transfer_listing(
            "r1",
            &FileTransferListing {
                request_id: "f1".into(),
                entry_id: "e2".into(),
                name: "/some/dir/l".into(),
                file_type: FileType::Symlink,
                parent: Some("e1".into()),
                data: Some(b"e1".to_vec()),
                ..Default::default()
            },
        );
        t.file_transfer_ok("r1", None, None, Some("/home/u"));
        t.file_transfer_error(
            "r1",
            Some("f9"),
            FileTransferError::Enoent,
            "Does not exist",
        );
        t.file_transfer_data("r1", "e2", b"hello", true);
        assert_eq!(
            out(&mut t),
            [
                osc(&format!("ac=status;id=r1;st={}", b64("OK"))),
                osc(&format!(
                    "ac=file;id=r1;fid=f1;st={};n={};ft=directory;mod=5;prm=493",
                    b64("e1"),
                    b64("/some/dir")
                )),
                osc(&format!(
                    "ac=file;id=r1;fid=f1;st={};n={};ft=symlink;pr=e1;d={}",
                    b64("e2"),
                    b64("/some/dir/l"),
                    b64("e1")
                )),
                osc(&format!(
                    "ac=status;id=r1;st={};n={}",
                    b64("OK"),
                    b64("/home/u")
                )),
                osc(&format!(
                    "ac=status;id=r1;fid=f9;st={}",
                    b64("ENOENT:Does not exist")
                )),
                osc(&format!("ac=end_data;id=r1;fid=e2;d={}", b64("hello"))),
            ]
            .concat()
        );
    }

    #[test]
    fn cancel_is_answered_with_canceled() {
        let (mut t, evs) = run(&osc("ac=cancel;id=c1"));
        assert_eq!(evs, vec![FileTransferEvent::Cancel { id: "c1".into() }]);
        t.file_transfer_canceled("c1");
        assert_eq!(
            out(&mut t),
            osc(&format!("ac=status;id=c1;st={}", b64("CANCELED")))
        );
    }

    #[test]
    fn refuse_is_eperm() {
        let (mut t, _) = run(&osc("ac=send;id=x"));
        t.file_transfer_refuse("x");
        assert_eq!(
            out(&mut t),
            osc(&format!(
                "ac=status;id=x;st={}",
                b64("EPERM:User refused the transfer")
            ))
        );
    }

    #[test]
    fn bypass_password_is_decoded() {
        let pw = b64("sha256:abc");
        let (_, evs) = run(&osc(&format!("ac=send;id=b;pw={pw};q=2")));
        assert!(matches!(
            &evs[0],
            FileTransferEvent::Send(s) if s.bypass.as_deref() == Some("sha256:abc") && s.quiet == 2
        ));
    }

    #[test]
    fn quiet_levels_drop_replies() {
        let (mut t, _) = run(&[osc("ac=send;id=q1;q=1"), osc("ac=send;id=q2;q=2")].concat());
        t.file_transfer_ok("q1", None, None, None);
        t.file_transfer_progress("q1", "f", 1);
        t.file_transfer_canceled("q1");
        t.file_transfer_error("q1", None, FileTransferError::Eio, "");
        t.file_transfer_error("q2", None, FileTransferError::Eio, "");
        t.file_transfer_ok("q2", None, None, None);
        // 他のセッションと、データは省かない。
        t.file_transfer_ok("q3", None, None, None);
        t.file_transfer_data("q1", "f", b"x", false);
        assert_eq!(
            out(&mut t),
            [
                osc(&format!("ac=status;id=q1;st={}", b64("EIO"))),
                osc(&format!("ac=status;id=q3;st={}", b64("OK"))),
                osc(&format!("ac=data;id=q1;fid=f;d={}", b64("x"))),
            ]
            .concat()
        );
    }

    #[test]
    fn data_is_chunked_at_4096_bytes() {
        let mut t = Terminal::new(2, 10);
        t.file_transfer_data("d", "f", &vec![7u8; 4096 * 2 + 1], true);
        let s = out(&mut t);
        let packets: Vec<_> = s.split("\x1b\\").filter(|p| !p.is_empty()).collect();
        assert_eq!(packets.len(), 3);
        assert!(packets[0].contains("ac=data;"));
        assert!(packets[1].contains("ac=data;"));
        assert!(packets[2].contains("ac=end_data;"));
        let len = |p: &str| {
            decode_base64(p.rsplit_once("d=").unwrap().1.as_bytes())
                .unwrap()
                .len()
        };
        assert_eq!(
            (len(packets[0]), len(packets[1]), len(packets[2])),
            (4096, 4096, 1)
        );

        // 空のデータは、データの無い end_data 1 つ。
        t.file_transfer_data("d", "f", b"", true);
        assert_eq!(out(&mut t), osc("ac=end_data;id=d;fid=f"));
        // last でない空のデータは何も出さない。
        t.file_transfer_data("d", "f", b"", false);
        assert!(out(&mut t).is_empty());
    }

    #[test]
    fn unknown_and_malformed_packets_are_dropped() {
        let cases = [
            "ac=status;id=a;st=T0s=",
            "ac=bogus;id=a",
            "id=a",
            "ac=send",
            "ac=send;id=a b",
            "ac=send;id=a;q=3",
            "ac=send;id=a;pw=!!!",
            "ac=send;id=a;sz=x",
            "ac=file;id=a;n=Zg==",
            "ac=file;id=a;fid=f",
            "ac=file;id=a;fid=f;n=%%%",
            "ac=file;id=a;fid=f;n=Zg==;ft=device",
            "ac=file;id=a;fid=f;n=Zg==;zip=gzip",
            "ac=file;id=a;fid=f;n=Zg==;tt=magic",
            "ac=file;id=a;fid=f;n=Zg==;mod=1.5",
            "ac=file;id=a;fid=f;n=/w==",
            "ac=data;id=a;d=AQID",
            "ac=data;id=a;fid=f;d=A",
            "ac=finish",
        ];
        for c in cases {
            let (mut t, evs) = run(&osc(c));
            assert!(evs.is_empty(), "{c}");
            assert!(out(&mut t).is_empty(), "{c}");
        }
    }

    #[test]
    fn unknown_keys_are_ignored_and_unspecified_is_none() {
        let (_, evs) = run(&osc(&format!(
            "ac=file;id=a;fid=f;n={};zzz=1;mod=-1;prm=-1;sz=-1;zip=none;tt=simple;ft=link",
            b64("/x")
        )));
        assert_eq!(
            evs,
            vec![FileTransferEvent::File(FileTransferFile {
                id: "a".into(),
                file_id: "f".into(),
                name: "/x".into(),
                file_type: FileType::Link,
                ..Default::default()
            })]
        );
    }

    #[test]
    fn works_with_bel_terminator_and_split_writes() {
        let mut t = Terminal::new(2, 10);
        t.write(b"\x1b]5113;ac=fin");
        assert!(t.take_events().is_empty());
        t.write(b"ish;id=z\x07");
        assert_eq!(t.take_events().len(), 1);
    }

    #[test]
    fn old_sessions_are_forgotten_past_the_cap() {
        let mut t = Terminal::new(2, 10);
        let mut input = String::new();
        for i in 0..=MAX_SESSIONS {
            input.push_str(&osc(&format!("ac=send;id=s{i};q=2")));
        }
        t.write(input.as_bytes());
        t.file_transfer_ok("s0", None, None, None);
        assert!(!out(&mut t).is_empty());
        t.file_transfer_ok("s1", None, None, None);
        assert!(out(&mut t).is_empty());
    }

    #[test]
    fn reset_forgets_quiet() {
        let (mut t, _) = run(&osc("ac=send;id=q;q=2"));
        t.reset();
        t.file_transfer_ok("q", None, None, None);
        assert!(!out(&mut t).is_empty());
    }
}
