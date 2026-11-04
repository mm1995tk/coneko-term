//! kitty の drag and drop protocol（OSC 72）。
//!
//! 仕様: <https://sw.kovidgoyal.net/kitty/dnd-protocol/>
//!
//! ここでは、PTY から来るパケットを読んでイベント（[`crate::Event::Dnd`]）にするところと、
//! 呼ぶ側（GUI）がドロップやドラッグの状況を渡す API からパケットを組んで PTY へ出すところまでを
//! 行う。OS のドラッグとドロップにつなぐこと、ファイルを読む・書くことは呼ぶ側の仕事。
//!
//! # ドロップを受ける（Accepting drops）
//!
//! - プログラムが `t=a` で受け付けると言うと [`DndEvent::DropEnabled`]、`t=A` で
//!   [`DndEvent::DropDisabled`] になる。受け付けている間だけ、`drop_*` のメソッドは働く。
//! - GUI は OS のドラッグの動きを `Terminal::drop_move`・`Terminal::drop_leave`・
//!   `Terminal::drop_drop` で渡す（位置、MIME の種類、許す操作）。`t=m` / `t=M` の
//!   バイト列はここで仕様の形に組む。MIME の種類の一覧は、最初の移動と変わったときだけ付ける。
//! - プログラムの答え（`t=m:o=O`）は [`DndEvent::DropResponse`]、データの頼み（`t=r`）は
//!   [`DndEvent::DropRequest`]、終わり（`t=r:o=O`）は [`DndEvent::DropFinished`] になる。
//!   データの頼みのうち、ドロップの前・範囲外・`text/uri-list` を先に頼んでいない・頼みが多すぎる、
//!   はここですぐ `t=R` で断る。答えは `Terminal::drop_data_reply` と `Terminal::drop_error_reply`。
//!
//! # ドラッグを始める（Starting drags）
//!
//! - `t=o:x=1` が [`DndEvent::DragEnabled`]、`t=o:x=2` が [`DndEvent::DragDisabled`]。
//!   GUI がドラッグの身振りを見つけたら `Terminal::drag_gesture` で `t=o` をプログラムへ送る。
//! - プログラムの申し出（`t=o:o=flags`）、先に送るデータ（`t=p`）、画像、`t=P:x=-1` を溜め、
//!   そろったところで [`DndEvent::DragStart`] にする。GUI は OS のドラッグを始め、
//!   `Terminal::drag_started` か `Terminal::drag_error` で答える。
//! - 始まったあとの `t=e`（データ）・`t=E`（エラー・取り消し）・`t=k`（遠くの機械へのドラッグ）は
//!   それぞれイベントになる。OS 側の進み具合は `Terminal::drag_accepted` などで `t=e` にする。
//!
//! # そのほか
//!
//! - 対応の問い合わせ（`t=q`）には、その場で `t=q` を返す。
//! - `i` は、`t=a` と `t=o` で渡されたものを、そのあとの送信にそのまま入れる（多重化装置のため）。
//! - 分けて送られたペイロード（`m=1`）は、そろってから 1 つとして扱う。最初のパケットの
//!   鍵だけを使う。別の `t` のパケットが割り込んだら、途中のものを捨てる。
//! - 知らない `t`・不正な鍵・不正な数は捨てる。

use std::collections::HashSet;

use super::clipboard::{decode_base64, encode_base64};

/// 1 つのパケットのペイロードの大きさの上限（符号化後、バイト）。
const CHUNK: usize = 4096;
/// 分けて送る生のデータの大きさ（base64 にすると `CHUNK` になる）。
const RAW_CHUNK: usize = CHUNK / 4 * 3;
/// 先に送るデータの合計の上限（バイト）。仕様は 64 MB 以上を受けることを求める。
pub(super) const MAX_PRESENT_BYTES: usize = 64 * 1024 * 1024;
/// ドラッグの画像の合計の上限（バイト）。
const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;
/// 分けて送られるデータ 1 つの上限（base64 のまま、バイト）。
const MAX_DATA_CHAIN: usize = MAX_PRESENT_BYTES / 3 * 4 + 1024;
/// 分けて送られるテキスト（MIME の一覧など）1 つの上限（バイト）。
const MAX_TEXT_CHAIN: usize = 1024 * 1024;
/// MIME の種類の数の上限。
const MAX_MIMES: usize = 256;
/// MIME の種類 1 つの長さの上限（字）。
const MAX_MIME_LEN: usize = 255;
/// 画像の数の上限。
const MAX_IMAGES: usize = 16;
/// 答え待ちのデータの頼みの数の上限。超えたら `EMFILE` でドロップを終わらせる（仕様）。
pub(super) const MAX_PENDING: usize = 64;
/// 機械の ID の長さの上限。
const MAX_MACHINE_ID: usize = 256;
/// エラーの説明の長さの上限（字）。
const MAX_DESCRIPTION: usize = 256;

/// ドラッグの位置。セルは左上が (0, 0)。画素は左上からのずれ。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DndLocation {
    /// セルの x。
    pub x: i32,
    /// セルの y。
    pub y: i32,
    /// 画素の x。
    pub pixel_x: i32,
    /// 画素の y。
    pub pixel_y: i32,
}

/// ドロップやドラッグの操作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DndOperation {
    /// コピー（1）。
    Copy,
    /// 移動（2）。
    Move,
}

impl DndOperation {
    fn code(self) -> u8 {
        match self {
            DndOperation::Copy => 1,
            DndOperation::Move => 2,
        }
    }

    fn from_code(code: i32) -> Option<Option<DndOperation>> {
        match code {
            0 => Some(None),
            1 => Some(Some(DndOperation::Copy)),
            2 => Some(Some(DndOperation::Move)),
            _ => None,
        }
    }
}

/// 許す操作の組（1 が copy、2 が move、3 が両方）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DndOperations {
    /// コピーを許す。
    pub copy: bool,
    /// 移動を許す。
    pub r#move: bool,
}

impl DndOperations {
    /// 仕様の数（1・2・3）から。0 や知らない値は `None`。
    pub fn from_bits(bits: u8) -> Option<DndOperations> {
        match bits {
            1..=3 => Some(DndOperations {
                copy: bits & 1 != 0,
                r#move: bits & 2 != 0,
            }),
            _ => None,
        }
    }

    /// 仕様の数（1・2・3、どちらも許さないなら 0）。
    pub fn bits(self) -> u8 {
        self.copy as u8 | (self.r#move as u8) << 1
    }
}

/// ターミナルが答えるエラー（POSIX の記号名）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DndError {
    /// 無い。
    Enoent,
    /// 不正。
    Einval,
    /// 知らないエラー。
    Eunknown,
    /// 許されない。
    Eperm,
    /// 入出力のエラー。
    Eio,
    /// 頼みが多すぎる。
    Emfile,
    /// 資源が足りない。
    Enomem,
    /// 大きすぎる。
    Efbig,
}

impl DndError {
    fn code(self) -> &'static str {
        match self {
            DndError::Enoent => "ENOENT",
            DndError::Einval => "EINVAL",
            DndError::Eunknown => "EUNKNOWN",
            DndError::Eperm => "EPERM",
            DndError::Eio => "EIO",
            DndError::Emfile => "EMFILE",
            DndError::Enomem => "ENOMEM",
            DndError::Efbig => "EFBIG",
        }
    }
}

/// ドロップされたデータの頼み。鍵は仕様の `x`・`y`・`Y`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropRequest {
    /// `t=r:x=idx`: 落とされた一覧の `index` 番目（1 始まり）の MIME の種類のデータ。
    Data {
        /// 1 始まりの番号。
        index: u32,
    },
    /// `t=r:x=idx:y=subidx`: `text/uri-list`（`index` 番目の種類）の `sub_index` 番目の項目
    /// （遠くの機械からのドロップ）。
    Entry {
        /// `text/uri-list` の、MIME の種類の中の番号（1 始まり）。
        index: u32,
        /// 一覧の中の項目の番号（1 始まり）。
        sub_index: u32,
    },
    /// `t=r:Y=handle:x=num`: ディレクトリ（`handle`）の `index` 番目（1 始まり）の項目。
    DirEntry {
        /// ディレクトリの handle。
        handle: u32,
        /// ディレクトリの中の番号（1 始まり）。
        index: u32,
    },
    /// `t=r:Y=handle`: ディレクトリを読み終えた。handle を捨ててよい。
    DirClose {
        /// ディレクトリの handle。
        handle: u32,
    },
}

/// データの答えの形（`X` 鍵）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropForm {
    /// MIME の種類のデータ（[`DropRequest::Data`] への答え）。`remote` が真なら、送り元が別の
    /// 機械（`X=1`、`text/uri-list` の答えにだけ付ける）。
    Mime {
        /// 送り元が、プログラムとは別の機械か。
        remote: bool,
    },
    /// 普通のファイルの中身。
    File,
    /// シンボリックリンク。データはリンク先。
    Symlink,
    /// ディレクトリ。データは、中の項目を NUL で区切った一覧。`u32` は handle（0 と 1 以外）。
    Directory(u32),
}

/// 画像の形式（`y` 鍵）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragImageFormat {
    /// 24 ビットの RGB（sRGB）。
    Rgb,
    /// 32 ビットの RGBA（sRGB）。
    Rgba,
    /// PNG。
    Png,
    /// UTF-8 のテキスト。ターミナルが描く。
    Text,
}

/// ドラッグの画像（サムネイル）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DragImage {
    /// 形式。
    pub format: DragImageFormat,
    /// 幅（画素）。[`DragImageFormat::Text`] では、文字の大きさの割合の分子（`base_font_size * width / height`）。
    pub width: u32,
    /// 高さ（画素）。[`DragImageFormat::Text`] では、同じ割合の分母。
    pub height: u32,
    /// 背景の不透明度（0 から 1024、`o` 鍵）。[`DragImageFormat::Text`] のときだけ意味がある。
    pub opacity: u32,
    /// 画像のデータ（base64 を戻したあと）。
    pub data: Vec<u8>,
}

/// プログラムが申し出たドラッグ（`t=P:x=-1` でそろったもの）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DragOffer {
    /// プログラムが許す操作。
    pub operations: DndOperations,
    /// 申し出た MIME の種類（順番が番号になる）。
    pub mimes: Vec<String>,
    /// 先に送られたデータ。`mimes` と同じ並びで、送られていないものは `None`。
    pub data: Vec<Option<Vec<u8>>>,
    /// 画像（先に送られた順）。
    pub images: Vec<DragImage>,
    /// 始めるときに使う画像の番号（0 始まり）。無ければ `None`。
    pub image: Option<u32>,
    /// `t=o:x=1` で渡された、プログラムの機械の ID。
    pub machine_id: Option<String>,
}

/// 遠くの機械へのドラッグで、プログラムが返した項目の形（`X` 鍵）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragEntryForm {
    /// 普通のファイルの中身。
    File,
    /// シンボリックリンク。データはリンク先。
    Symlink,
    /// ディレクトリ。データは、中の項目を NUL で区切った一覧。`u32` は handle。
    Directory(u32),
}

/// 遠くの機械へのドラッグで、項目を指す鍵（仕様の `x`・`y`・`Y`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DragEntryKey {
    /// `text/uri-list` の中の番号（1 始まり）。
    pub index: u32,
    /// ディレクトリの中の項目の番号（1 始まり）。トップの項目なら 0。
    pub sub_index: u32,
    /// 親のディレクトリの handle。トップの項目なら 0。
    pub parent: u32,
}

/// OSC 72 から出るイベント。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DndEvent {
    /// プログラムがドロップを受け付けると言った（`t=a`）。`mimes` は、受け付ける MIME の種類
    /// （空でもよい）。
    DropEnabled {
        /// 受け付ける MIME の種類。
        mimes: Vec<String>,
    },
    /// プログラムがドロップを受け付けないと言った（`t=A`）、またはリセットされた。
    DropDisabled,
    /// ドラッグが窓の上にある間の、プログラムの答え（`t=m:o=O`）。GUI は OS に、ドロップを
    /// 受けるか・どの操作かを伝える。
    DropResponse {
        /// 行う操作。`None` は受け付けない。
        operation: Option<DndOperation>,
        /// 受けたい MIME の種類（好む順）。`None` は、一覧を変えない。
        mimes: Option<Vec<String>>,
    },
    /// ドロップされたデータの頼み。GUI が読み、`Terminal::drop_data_reply` か
    /// `Terminal::drop_error_reply` で答える。
    DropRequest(DropRequest),
    /// プログラムがドロップを読み終えた（`t=r:o=O`）。GUI は OS にドロップの完了を伝える。
    /// `operation` が `None` なら取り消し。頼みが多すぎてターミナルが終わらせたときも、これが出る。
    DropFinished {
        /// 最後に行った操作。
        operation: Option<DndOperation>,
    },
    /// プログラムがドラッグを始められると言った（`t=o:x=1`）。
    DragEnabled {
        /// プログラムの機械の ID（無ければ `None`）。
        machine_id: Option<String>,
    },
    /// プログラムがドラッグを始めないと言った（`t=o:x=2`）、またはリセットされた。
    DragDisabled,
    /// ドラッグを始める求め（`t=P:x=-1`）。GUI は OS のドラッグを始め、`Terminal::drag_started`
    /// か `Terminal::drag_error` で答える。
    DragStart(Box<DragOffer>),
    /// ドラッグの画像を変える（`t=P:x=idx`）。`None` は、画像を外す。
    DragImage {
        /// 画像の番号（0 始まり）。
        index: Option<u32>,
    },
    /// 頼まれた MIME の種類のデータ（`t=e:y=idx`）。`index` は 0 始まり。
    DragData {
        /// MIME の種類の番号（0 始まり）。
        index: u32,
        /// データ。
        data: Vec<u8>,
    },
    /// 頼まれた MIME の種類のデータが渡せない（`t=E:y=idx`）。
    DragDataError {
        /// MIME の種類の番号（0 始まり）。
        index: u32,
        /// POSIX の記号名（`ENOENT` など）。
        error: String,
        /// 説明（無ければ空）。
        description: String,
    },
    /// 遠くの機械へのドラッグの項目のデータ（`t=k`）。
    DragEntry {
        /// 項目を指す鍵。
        key: DragEntryKey,
        /// 形。
        form: DragEntryForm,
        /// データ。
        data: Vec<u8>,
    },
    /// プログラムがドラッグ全体を取り消した（`t=E:y=-1`）。
    DragCanceled,
    /// プログラムがエラーでドラッグを止めた（`y` の無い `t=E`）。
    DragAborted {
        /// POSIX の記号名。
        error: String,
        /// 説明（無ければ空）。
        description: String,
    },
}

// ---- 鍵 ----

/// 1 つのパケットの鍵。
#[derive(Clone, Debug, Default)]
struct Meta {
    kind: char,
    kind_given: bool,
    more: bool,
    id: u32,
    op: Option<i32>,
    x: Option<i32>,
    y: Option<i32>,
    big_x: Option<i32>,
    big_y: Option<i32>,
}

impl Meta {
    /// 読めない数があれば `None`。
    fn parse(meta: &str) -> Option<Meta> {
        let mut m = Meta {
            kind: 'a',
            ..Meta::default()
        };
        for kv in meta.split(':') {
            let Some((k, v)) = kv.split_once('=') else {
                continue;
            };
            let int = || v.parse::<i64>().ok().and_then(|n| i32::try_from(n).ok());
            match k {
                "t" => {
                    let mut cs = v.chars();
                    m.kind = cs.next().filter(|_| cs.next().is_none())?;
                    m.kind_given = true;
                }
                "m" => m.more = v == "1",
                "i" => m.id = u32::try_from(int()?).ok()?,
                "o" => m.op = Some(int()?),
                "x" => m.x = Some(int()?),
                "y" => m.y = Some(int()?),
                "X" => m.big_x = Some(int()?),
                "Y" => m.big_y = Some(int()?),
                _ => {}
            }
        }
        Some(m)
    }
}

/// 分けて送られている途中のパケット。
#[derive(Debug)]
struct Chain {
    meta: Meta,
    payload: Vec<u8>,
}

// ---- 受ける側の状態 ----

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    /// ドラッグが窓の上に無い。
    #[default]
    Idle,
    /// ドラッグが窓の上にある（`t=m` を送っている）。
    Over,
    /// ドロップされた（`t=M` を送った）。
    Dropped,
}

#[derive(Debug, Default)]
struct Session {
    phase: Phase,
    /// 最後に送った MIME の一覧。
    announced: Vec<String>,
    /// ドロップされた MIME の一覧。
    dropped: Vec<String>,
    /// ドラッグがこの窓から始まったか。
    from_self: bool,
    /// `text/uri-list` を頼まれたか。
    uri_list: bool,
    /// 答え待ちの頼みの数。
    pending: usize,
    /// 答えたディレクトリの handle。
    handles: HashSet<u32>,
}

// ---- 始める側の状態 ----

#[derive(Debug)]
struct Builder {
    operations: DndOperations,
    mimes: Vec<String>,
    data: Vec<Option<Vec<u8>>>,
    images: Vec<DragImage>,
    image: Option<u32>,
    data_total: usize,
    image_total: usize,
}

#[derive(Debug, Default)]
enum Drag {
    #[default]
    Idle,
    /// 申し出を受けて、データと画像を溜めている。
    Building(Builder),
    /// `DragStart` を出して、GUI の答えを待っている。
    Starting { mimes: usize },
    /// ドラッグが始まっている。
    Active { mimes: usize },
}

/// OSC 72 の状態。
#[derive(Debug, Default)]
pub(super) struct Dnd {
    chain: Option<Chain>,
    events: Vec<DndEvent>,
    out: Vec<u8>,
    // 受ける側
    accepting: bool,
    accept_mimes: Vec<String>,
    machine_id: Option<String>,
    accept_id: u32,
    session: Session,
    // 始める側
    offers: bool,
    offer_machine_id: Option<String>,
    offer_id: u32,
    gesture: bool,
    drag: Drag,
}

// ---- 文字列 ----

fn clean_text(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

fn valid_mime(m: &str) -> bool {
    !m.is_empty() && m.chars().count() <= MAX_MIME_LEN && !m.chars().any(|c| c.is_control())
}

/// 空白で区切った MIME の一覧。多すぎる・不正な種類があるなら `None`。
fn parse_mimes(payload: &[u8]) -> Option<Vec<String>> {
    let text = std::str::from_utf8(payload).ok()?;
    let mimes: Vec<String> = text.split_ascii_whitespace().map(str::to_string).collect();
    (mimes.len() <= MAX_MIMES && mimes.iter().all(|m| valid_mime(m))).then_some(mimes)
}

/// 詰め物が無くてもよい base64（仕様）を戻す。
fn decode(payload: &[u8]) -> Option<Vec<u8>> {
    let mut v = payload.to_vec();
    while v.last() == Some(&b'=') {
        v.pop();
    }
    match v.len() % 4 {
        1 => return None,
        2 => v.extend_from_slice(b"=="),
        3 => v.push(b'='),
        _ => {}
    }
    decode_base64(&v)
}

/// `POSIX の記号名[:説明]`。
fn parse_error(payload: &[u8]) -> (String, String) {
    let text = String::from_utf8_lossy(payload);
    let (name, desc) = text.split_once(':').unwrap_or((&text, ""));
    let name: String = name
        .chars()
        .filter(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
        .take(32)
        .collect();
    let name = if name.is_empty() {
        "EUNKNOWN".to_string()
    } else {
        name
    };
    (name, clean_text(desc, MAX_DESCRIPTION))
}

fn parse_machine_id(payload: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(payload).ok()?;
    (!s.is_empty() && s.len() <= MAX_MACHINE_ID && s.bytes().all(|b| b.is_ascii_graphic()))
        .then(|| s.to_string())
}

// ---- PTY へ出す ----

/// `ESC ] 72 ; 鍵 [:i=…] [; 中身] ESC \`。
fn packet(out: &mut Vec<u8>, meta: &str, id: u32, payload: &str) {
    out.extend_from_slice(b"\x1b]72;");
    out.extend_from_slice(meta.as_bytes());
    if id != 0 {
        out.extend_from_slice(format!(":i={id}").as_bytes());
    }
    if !payload.is_empty() {
        out.push(b';');
        out.extend_from_slice(payload.as_bytes());
    }
    out.extend_from_slice(b"\x1b\\");
}

/// テキストを、4096 バイトを超えるなら `m=1` で分けて出す。
fn text_packets(out: &mut Vec<u8>, meta: &str, id: u32, text: &str) {
    if text.len() <= CHUNK {
        packet(out, meta, id, text);
        return;
    }
    let mut rest = text;
    while !rest.is_empty() {
        let mut n = rest.len().min(CHUNK);
        while !rest.is_char_boundary(n) {
            n -= 1;
        }
        let (head, tail) = rest.split_at(n);
        let more = if tail.is_empty() { 0 } else { 1 };
        packet(out, &format!("{meta}:m={more}"), id, head);
        rest = tail;
    }
}

/// データを base64 にして `m=1` で分け、終わりに空の `m=0` を出す。
fn data_packets(out: &mut Vec<u8>, meta: &str, id: u32, data: &[u8]) {
    for chunk in data.chunks(RAW_CHUNK) {
        packet(out, &format!("{meta}:m=1"), id, &encode_base64(chunk));
    }
    packet(out, &format!("{meta}:m=0"), id, "");
}

fn error_payload(e: DndError, description: &str) -> String {
    let d = clean_text(description, MAX_DESCRIPTION);
    if d.is_empty() {
        e.code().to_string()
    } else {
        format!("{}:{d}", e.code())
    }
}

fn request_keys(req: &DropRequest) -> String {
    match *req {
        DropRequest::Data { index } => format!("x={index}"),
        DropRequest::Entry { index, sub_index } => format!("x={index}:y={sub_index}"),
        DropRequest::DirEntry { handle, index } => format!("Y={handle}:x={index}"),
        DropRequest::DirClose { handle } => format!("Y={handle}"),
    }
}

fn mimes_text(mimes: &[String]) -> String {
    mimes
        .iter()
        .filter(|m| valid_mime(m))
        .take(MAX_MIMES)
        .cloned()
        .collect::<Vec<_>>()
        .join(" ")
}

impl Dnd {
    /// 溜まったイベントを渡す。
    pub(super) fn take_events(&mut self) -> Vec<DndEvent> {
        std::mem::take(&mut self.events)
    }

    /// 溜まった PTY へのバイト列を渡す。
    pub(super) fn take_out(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    /// 全部捨てる（リセット）。受け付けていた・申し出ていたなら、それを知らせる。
    pub(super) fn reset(&mut self) {
        let accepting = self.accepting;
        let offers = self.offers;
        let events = std::mem::take(&mut self.events);
        *self = Dnd::default();
        self.events = events;
        if accepting {
            self.events.push(DndEvent::DropDisabled);
        }
        if offers {
            self.events.push(DndEvent::DragDisabled);
        }
    }

    // ---- PTY から ----

    /// `OSC 72 ;` のあとの `鍵 ; 中身` を処理する。
    pub(super) fn osc(&mut self, rest: &[u8]) {
        let (meta, payload) = match rest.iter().position(|&b| b == b';') {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => (rest, &[][..]),
        };
        let Some(meta) = std::str::from_utf8(meta).ok().and_then(Meta::parse) else {
            return;
        };
        if meta.kind == 'q' {
            // 問い合わせは、分けて送られている途中でもよい。
            let mut m = String::from("t=q");
            if meta.id != 0 {
                m.push_str(&format!(":i={}", meta.id));
            }
            packet(&mut self.out, &m, 0, "");
            return;
        }
        if let Some(chain) = &mut self.chain {
            if !meta.kind_given || meta.kind == chain.meta.kind {
                if chain.payload.len() + payload.len() > chain_cap(chain.meta.kind) {
                    let kind = chain.meta.kind;
                    self.chain = None;
                    if matches!(kind, 'p' | 'e' | 'k') {
                        self.fail_drag(DndError::Efbig);
                    }
                    return;
                }
                chain.payload.extend_from_slice(payload);
                if !meta.more {
                    let chain = self.chain.take().expect("chain");
                    self.process(chain.meta, chain.payload);
                }
                return;
            }
            // 分けて送っている途中に別のパケットが来た: 仕様の違反。途中のものを捨てる。
            self.chain = None;
        }
        if meta.more {
            if payload.len() > chain_cap(meta.kind) {
                if matches!(meta.kind, 'p' | 'e' | 'k') {
                    self.fail_drag(DndError::Efbig);
                }
                return;
            }
            self.chain = Some(Chain {
                meta,
                payload: payload.to_vec(),
            });
            return;
        }
        self.process(meta, payload.to_vec());
    }

    fn process(&mut self, meta: Meta, payload: Vec<u8>) {
        match meta.kind {
            'a' => self.accept(&meta, &payload),
            'A' => self.stop_accepting(),
            'm' => self.drop_response(&meta, &payload),
            'r' => self.drop_request(&meta),
            'o' => self.offer(&meta, &payload),
            'p' => self.present(&meta, &payload),
            'P' => self.drag_command(&meta),
            'e' => self.drag_data(&meta, &payload),
            'E' => self.drag_error_from_client(&meta, &payload),
            'k' => self.drag_entry(&meta, &payload),
            _ => {}
        }
    }

    // ---- 受ける側: プログラムから ----

    fn accept(&mut self, meta: &Meta, payload: &[u8]) {
        if meta.id != 0 {
            self.accept_id = meta.id;
        }
        if meta.x == Some(1) {
            // 機械の ID だけを渡すパケット。
            if let Some(id) = parse_machine_id(payload) {
                self.machine_id = Some(id);
            }
            return;
        }
        let Some(mimes) = parse_mimes(payload) else {
            return;
        };
        self.accepting = true;
        self.accept_mimes = mimes.clone();
        self.events.push(DndEvent::DropEnabled { mimes });
    }

    fn stop_accepting(&mut self) {
        self.session = Session::default();
        self.machine_id = None;
        if std::mem::take(&mut self.accepting) {
            self.accept_mimes.clear();
            self.events.push(DndEvent::DropDisabled);
        }
    }

    fn drop_response(&mut self, meta: &Meta, payload: &[u8]) {
        if !self.accepting || self.session.phase != Phase::Over {
            return;
        }
        let Some(operation) = DndOperation::from_code(meta.op.unwrap_or(0)) else {
            return;
        };
        let mimes = if payload.is_empty() {
            None
        } else {
            match parse_mimes(payload) {
                Some(m) => Some(m),
                None => return,
            }
        };
        self.events
            .push(DndEvent::DropResponse { operation, mimes });
    }

    /// 受ける側の `t=R`。
    fn drop_error(&mut self, keys: &str, e: DndError, description: &str) {
        let meta = if keys.is_empty() {
            "t=R".to_string()
        } else {
            format!("t=R:{keys}")
        };
        packet(
            &mut self.out,
            &meta,
            self.accept_id,
            &error_payload(e, description),
        );
    }

    fn drop_request(&mut self, meta: &Meta) {
        // 終わり: `t=r:o=operation`。
        if meta.x.is_none() && meta.big_y.is_none() {
            if self.session.phase != Phase::Dropped {
                self.drop_error("", DndError::Eperm, "");
                return;
            }
            let operation = DndOperation::from_code(meta.op.unwrap_or(0)).unwrap_or(None);
            self.session = Session::default();
            self.events.push(DndEvent::DropFinished { operation });
            return;
        }
        let positive = |n: Option<i32>| n.and_then(|n| u32::try_from(n).ok()).filter(|&n| n >= 1);
        let handle = meta.big_y.map(|h| u32::try_from(h).unwrap_or(u32::MAX));
        let req = match (handle, meta.x, meta.y) {
            (Some(handle), None, _) => Some(DropRequest::DirClose { handle }),
            (Some(handle), Some(_), _) => {
                positive(meta.x).map(|index| DropRequest::DirEntry { handle, index })
            }
            (None, Some(_), Some(_)) => positive(meta.x)
                .zip(positive(meta.y))
                .map(|(index, sub_index)| DropRequest::Entry { index, sub_index }),
            (None, Some(_), None) => positive(meta.x).map(|index| DropRequest::Data { index }),
            (None, None, _) => None,
        };
        let Some(req) = req else {
            let keys = [
                meta.x.map(|v| format!("x={v}")),
                meta.y.map(|v| format!("y={v}")),
                meta.big_y.map(|v| format!("Y={v}")),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(":");
            self.drop_error(&keys, DndError::Enoent, "");
            return;
        };
        let keys = request_keys(&req);
        // ドロップの前の頼みは断る。ドロップは終わらない。
        if !self.accepting || self.session.phase != Phase::Dropped {
            self.drop_error(&keys, DndError::Eperm, "");
            return;
        }
        match req {
            DropRequest::Data { index } => {
                let Some(mime) = self.session.dropped.get(index as usize - 1) else {
                    self.drop_error(&keys, DndError::Enoent, "");
                    return;
                };
                if mime == "text/uri-list" {
                    self.session.uri_list = true;
                }
            }
            DropRequest::Entry { index, .. } => {
                let is_list = self
                    .session
                    .dropped
                    .get(index as usize - 1)
                    .is_some_and(|m| m == "text/uri-list");
                if !self.session.uri_list || !is_list {
                    self.drop_error(&keys, DndError::Einval, "");
                    return;
                }
                if self.session.from_self {
                    self.drop_error(&keys, DndError::Eperm, "");
                    return;
                }
            }
            DropRequest::DirEntry { handle, .. } => {
                if !self.session.handles.contains(&handle) {
                    self.drop_error(&keys, DndError::Einval, "");
                    return;
                }
                if self.session.from_self {
                    self.drop_error(&keys, DndError::Eperm, "");
                    return;
                }
            }
            DropRequest::DirClose { handle } => {
                if !self.session.handles.remove(&handle) {
                    self.drop_error(&keys, DndError::Einval, "");
                    return;
                }
                self.events.push(DndEvent::DropRequest(req));
                return;
            }
        }
        if self.session.pending >= MAX_PENDING {
            // 頼みが多すぎる: 断ってドロップを終わらせる（仕様）。
            self.drop_error(&keys, DndError::Emfile, "");
            self.session = Session::default();
            self.events.push(DndEvent::DropFinished { operation: None });
            return;
        }
        self.session.pending += 1;
        self.events.push(DndEvent::DropRequest(req));
    }

    // ---- 受ける側: GUI から ----

    pub(super) fn accepting(&self) -> bool {
        self.accepting
    }

    pub(super) fn accept_mimes(&self) -> &[String] {
        &self.accept_mimes
    }

    pub(super) fn machine_id(&self) -> Option<&str> {
        self.machine_id.as_deref()
    }

    fn drop_packet(&mut self, kind: char, loc: DndLocation, op: u8, mimes: &str) {
        let meta = format!(
            "t={kind}:x={}:y={}:X={}:Y={}:o={op}",
            loc.x, loc.y, loc.pixel_x, loc.pixel_y
        );
        text_packets(&mut self.out, &meta, self.accept_id, mimes);
    }

    pub(super) fn drop_move(
        &mut self,
        loc: DndLocation,
        mimes: &[String],
        allowed: DndOperations,
    ) -> bool {
        if !self.accepting || loc.x < 0 || loc.y < 0 {
            return false;
        }
        if self.session.phase == Phase::Dropped {
            // 前のドロップが終わらないまま、新しいドラッグが入ってきた。
            self.session = Session::default();
        }
        self.session.phase = Phase::Over;
        let valid: Vec<String> = mimes.iter().filter(|m| valid_mime(m)).cloned().collect();
        let text = if valid != self.session.announced {
            mimes_text(&valid)
        } else {
            String::new()
        };
        self.session.announced = valid;
        self.drop_packet('m', loc, allowed.bits(), &text);
        true
    }

    pub(super) fn drop_leave(&mut self) -> bool {
        if !self.accepting || self.session.phase != Phase::Over {
            return false;
        }
        self.session = Session::default();
        let loc = DndLocation {
            x: -1,
            y: -1,
            pixel_x: 0,
            pixel_y: 0,
        };
        self.drop_packet('m', loc, 0, "");
        true
    }

    pub(super) fn drop_drop(
        &mut self,
        loc: DndLocation,
        mimes: &[String],
        allowed: DndOperations,
        from_self: bool,
    ) -> bool {
        let valid: Vec<String> = mimes.iter().filter(|m| valid_mime(m)).cloned().collect();
        if !self.accepting || loc.x < 0 || loc.y < 0 || valid.is_empty() {
            return false;
        }
        let text = mimes_text(&valid);
        self.session = Session {
            phase: Phase::Dropped,
            announced: valid.clone(),
            dropped: valid,
            from_self,
            ..Session::default()
        };
        self.drop_packet('M', loc, allowed.bits(), &text);
        true
    }

    /// データの答え。頼まれていないものは出さない。
    pub(super) fn drop_data_reply(&mut self, req: &DropRequest, form: DropForm, data: &[u8]) {
        if self.session.phase != Phase::Dropped || matches!(req, DropRequest::DirClose { .. }) {
            return;
        }
        let keys = request_keys(req);
        let x = match form {
            DropForm::Mime { remote: true } | DropForm::Symlink => ":X=1".to_string(),
            DropForm::Mime { remote: false } | DropForm::File => String::new(),
            DropForm::Directory(h) if h >= 2 => {
                self.session.handles.insert(h);
                format!(":X={h}")
            }
            DropForm::Directory(_) => {
                self.drop_error_reply(req, DndError::Eio, "bad directory handle");
                return;
            }
        };
        self.session.pending = self.session.pending.saturating_sub(1);
        data_packets(
            &mut self.out,
            &format!("t=r:{keys}{x}"),
            self.accept_id,
            data,
        );
    }

    pub(super) fn drop_error_reply(&mut self, req: &DropRequest, e: DndError, description: &str) {
        if matches!(req, DropRequest::DirClose { .. }) {
            return;
        }
        self.session.pending = self.session.pending.saturating_sub(1);
        self.drop_error(&request_keys(req), e, description);
    }

    // ---- 始める側: プログラムから ----

    /// 始める側の `t=E`。
    fn drag_error_packet(&mut self, y: Option<i32>, payload: &str) {
        let meta = match y {
            Some(y) => format!("t=E:y={y}"),
            None => "t=E".to_string(),
        };
        packet(&mut self.out, &meta, self.offer_id, payload);
    }

    /// エラーを返して、ドラッグを捨てる。
    fn fail_drag(&mut self, e: DndError) {
        self.drag_error_packet(None, e.code());
        self.cancel_drag();
    }

    /// ドラッグを捨てる。始まっていたら、取り消しを知らせる。
    fn cancel_drag(&mut self) {
        if matches!(
            std::mem::take(&mut self.drag),
            Drag::Starting { .. } | Drag::Active { .. }
        ) {
            self.events.push(DndEvent::DragCanceled);
        }
    }

    fn offer(&mut self, meta: &Meta, payload: &[u8]) {
        if meta.id != 0 {
            self.offer_id = meta.id;
        }
        match meta.x {
            Some(1) => {
                self.offers = true;
                self.gesture = false;
                self.drag = Drag::Idle;
                self.offer_machine_id = parse_machine_id(payload);
                self.events.push(DndEvent::DragEnabled {
                    machine_id: self.offer_machine_id.clone(),
                });
                return;
            }
            Some(2) => {
                self.gesture = false;
                self.drag = Drag::Idle;
                self.offer_machine_id = None;
                if std::mem::take(&mut self.offers) {
                    self.events.push(DndEvent::DragDisabled);
                }
                return;
            }
            _ => {}
        }
        // 申し出。身振りの知らせ（`drag_gesture`）のあとの 1 回だけ受ける。
        if !self.offers || !std::mem::take(&mut self.gesture) || !matches!(self.drag, Drag::Idle) {
            self.drag_error_packet(None, DndError::Eperm.code());
            return;
        }
        let Some(operations) = u8::try_from(meta.op.unwrap_or(0))
            .ok()
            .and_then(DndOperations::from_bits)
        else {
            self.drag_error_packet(None, DndError::Einval.code());
            return;
        };
        let Some(mimes) = parse_mimes(payload) else {
            self.drag_error_packet(None, DndError::Efbig.code());
            return;
        };
        if mimes.is_empty() {
            self.drag_error_packet(None, DndError::Einval.code());
            return;
        }
        self.drag = Drag::Building(Builder {
            operations,
            data: vec![None; mimes.len()],
            mimes,
            images: Vec::new(),
            image: None,
            data_total: 0,
            image_total: 0,
        });
    }

    fn present(&mut self, meta: &Meta, payload: &[u8]) {
        let Drag::Building(b) = &mut self.drag else {
            self.drag_error_packet(None, DndError::Eperm.code());
            return;
        };
        let Some(bytes) = decode(payload) else {
            self.fail_drag(DndError::Einval);
            return;
        };
        let idx = meta.x.unwrap_or(0);
        if idx >= 0 {
            let Some(slot) = b.data.get_mut(idx as usize) else {
                self.fail_drag(DndError::Einval);
                return;
            };
            b.data_total = b.data_total - slot.as_ref().map_or(0, Vec::len) + bytes.len();
            if b.data_total > MAX_PRESENT_BYTES {
                self.fail_drag(DndError::Efbig);
                return;
            }
            *slot = Some(bytes);
            return;
        }
        // 画像: -1 から順に。
        if idx.unsigned_abs() as usize != b.images.len() + 1 {
            self.fail_drag(DndError::Einval);
            return;
        }
        let (Some(width), Some(height)) = (
            meta.big_x.and_then(|v| u32::try_from(v).ok()),
            meta.big_y.and_then(|v| u32::try_from(v).ok()),
        ) else {
            self.fail_drag(DndError::Einval);
            return;
        };
        let format = match meta.y.unwrap_or(0) {
            24 => DragImageFormat::Rgb,
            32 => DragImageFormat::Rgba,
            100 => DragImageFormat::Png,
            0 => DragImageFormat::Text,
            _ => {
                self.fail_drag(DndError::Einval);
                return;
            }
        };
        let per_pixel = match format {
            DragImageFormat::Rgb => Some(3u64),
            DragImageFormat::Rgba => Some(4),
            _ => None,
        };
        if let Some(n) = per_pixel {
            if u64::from(width) * u64::from(height) * n != bytes.len() as u64 {
                self.fail_drag(DndError::Einval);
                return;
            }
        }
        let opacity = meta.op.unwrap_or(0);
        if !(0..=1024).contains(&opacity)
            || (format == DragImageFormat::Text && std::str::from_utf8(&bytes).is_err())
        {
            self.fail_drag(DndError::Einval);
            return;
        }
        b.image_total += bytes.len();
        if b.images.len() >= MAX_IMAGES || b.image_total > MAX_IMAGE_BYTES {
            self.fail_drag(DndError::Efbig);
            return;
        }
        b.images.push(DragImage {
            format,
            width,
            height,
            opacity: opacity as u32,
            data: bytes,
        });
    }

    fn drag_command(&mut self, meta: &Meta) {
        let idx = meta.x.unwrap_or(0);
        match &mut self.drag {
            Drag::Building(b) if idx == -1 => {
                let image = b.image.or(if b.images.is_empty() { None } else { Some(0) });
                let offer = DragOffer {
                    operations: b.operations,
                    mimes: std::mem::take(&mut b.mimes),
                    data: std::mem::take(&mut b.data),
                    images: std::mem::take(&mut b.images),
                    image,
                    machine_id: self.offer_machine_id.clone(),
                };
                self.drag = Drag::Starting {
                    mimes: offer.mimes.len(),
                };
                self.events.push(DndEvent::DragStart(Box::new(offer)));
            }
            Drag::Building(b) => {
                b.image = u32::try_from(idx)
                    .ok()
                    .filter(|&i| (i as usize) < b.images.len());
            }
            Drag::Active { .. } if idx != -1 => {
                let index = u32::try_from(idx).ok();
                self.events.push(DndEvent::DragImage { index });
            }
            _ => self.drag_error_packet(None, DndError::Eperm.code()),
        }
    }

    fn drag_data(&mut self, meta: &Meta, payload: &[u8]) {
        let Drag::Active { mimes } = self.drag else {
            // 始まる前の `t=e` は不正（仕様）。
            self.fail_drag(DndError::Einval);
            return;
        };
        let Some(index) = meta
            .y
            .and_then(|y| u32::try_from(y).ok())
            .filter(|&i| (i as usize) < mimes)
        else {
            self.drag_error_packet(meta.y, DndError::Einval.code());
            return;
        };
        match decode(payload) {
            Some(data) => self.events.push(DndEvent::DragData { index, data }),
            None => self.events.push(DndEvent::DragDataError {
                index,
                error: DndError::Einval.code().to_string(),
                description: "invalid base64".to_string(),
            }),
        }
    }

    fn drag_error_from_client(&mut self, meta: &Meta, payload: &[u8]) {
        let (error, description) = parse_error(payload);
        match (&self.drag, meta.y) {
            (Drag::Idle, _) => {}
            (Drag::Active { mimes }, Some(y)) if y >= 0 => {
                if (y as usize) < *mimes {
                    self.events.push(DndEvent::DragDataError {
                        index: y as u32,
                        error,
                        description,
                    });
                } else {
                    self.drag_error_packet(Some(y), DndError::Einval.code());
                }
            }
            (Drag::Building(_), _) => self.drag = Drag::Idle,
            (Drag::Starting { .. } | Drag::Active { .. }, Some(-1)) => {
                self.drag = Drag::Idle;
                self.events.push(DndEvent::DragCanceled);
            }
            (Drag::Starting { .. } | Drag::Active { .. }, None) => {
                self.drag = Drag::Idle;
                self.events
                    .push(DndEvent::DragAborted { error, description });
            }
            (Drag::Starting { .. } | Drag::Active { .. }, Some(_)) => {
                self.fail_drag(DndError::Einval);
            }
        }
    }

    fn drag_entry(&mut self, meta: &Meta, payload: &[u8]) {
        if !matches!(self.drag, Drag::Active { .. }) {
            self.fail_drag(DndError::Einval);
            return;
        }
        let u = |n: Option<i32>| n.and_then(|n| u32::try_from(n).ok());
        let Some(index) = u(meta.x).filter(|&i| i >= 1) else {
            return;
        };
        let form = match meta.big_x.unwrap_or(0) {
            0 => DragEntryForm::File,
            1 => DragEntryForm::Symlink,
            h if h > 1 => DragEntryForm::Directory(h as u32),
            _ => return,
        };
        let key = DragEntryKey {
            index,
            sub_index: u(meta.y).unwrap_or(0),
            parent: u(meta.big_y).unwrap_or(0),
        };
        match decode(payload) {
            Some(data) => self.events.push(DndEvent::DragEntry { key, form, data }),
            None => {
                self.drag = Drag::Idle;
                self.events.push(DndEvent::DragAborted {
                    error: DndError::Einval.code().to_string(),
                    description: "invalid base64".to_string(),
                });
            }
        }
    }

    // ---- 始める側: GUI から ----

    pub(super) fn offers_enabled(&self) -> bool {
        self.offers
    }

    pub(super) fn offer_machine_id(&self) -> Option<&str> {
        self.offer_machine_id.as_deref()
    }

    pub(super) fn drag_gesture(&mut self, loc: DndLocation) -> bool {
        if !self.offers || !matches!(self.drag, Drag::Idle) || loc.x < 0 || loc.y < 0 {
            return false;
        }
        self.gesture = true;
        let meta = format!(
            "t=o:x={}:y={}:X={}:Y={}",
            loc.x, loc.y, loc.pixel_x, loc.pixel_y
        );
        packet(&mut self.out, &meta, self.offer_id, "");
        true
    }

    /// `DragStart` への答え（成功）。
    pub(super) fn drag_started(&mut self) -> bool {
        let Drag::Starting { mimes } = self.drag else {
            return false;
        };
        self.drag = Drag::Active { mimes };
        self.drag_error_packet(None, "OK");
        true
    }

    /// `DragStart` への答え（失敗）、またはドラッグを止める。`t=E` を出して、ドラッグを捨てる。
    pub(super) fn drag_error(&mut self, e: DndError, description: &str) -> bool {
        if matches!(self.drag, Drag::Idle) {
            return false;
        }
        self.drag = Drag::Idle;
        self.drag_error_packet(None, &error_payload(e, description));
        true
    }

    fn drag_active(&self) -> bool {
        matches!(self.drag, Drag::Active { .. })
    }

    pub(super) fn drag_accepted(&mut self, index: u32) -> bool {
        self.drag_event(&format!("x=1:y={index}"))
    }

    pub(super) fn drag_action(&mut self, op: Option<DndOperation>) -> bool {
        self.drag_event(&format!("x=2:o={}", op.map_or(0, DndOperation::code)))
    }

    pub(super) fn drag_dropped(&mut self) -> bool {
        self.drag_event("x=3")
    }

    pub(super) fn drag_finished(&mut self, canceled: bool) -> bool {
        if !self.drag_event(&format!("x=4:y={}", canceled as u8)) {
            return false;
        }
        self.drag = Drag::Idle;
        true
    }

    pub(super) fn drag_data_request(&mut self, index: u32) -> bool {
        let Drag::Active { mimes } = self.drag else {
            return false;
        };
        (index as usize) < mimes && self.drag_event(&format!("x=5:y={index}"))
    }

    fn drag_event(&mut self, keys: &str) -> bool {
        if !self.drag_active() {
            return false;
        }
        packet(&mut self.out, &format!("t=e:{keys}"), self.offer_id, "");
        true
    }

    /// 遠くの機械へのドラッグで、`text/uri-list` の `index` 番目（1 始まり）の項目を頼む。
    pub(super) fn drag_entry_request(&mut self, index: u32) -> bool {
        if !self.drag_active() || index == 0 {
            return false;
        }
        packet(&mut self.out, &format!("t=k:x={index}"), self.offer_id, "");
        true
    }
}

fn chain_cap(kind: char) -> usize {
    if matches!(kind, 'p' | 'e' | 'k') {
        MAX_DATA_CHAIN
    } else {
        MAX_TEXT_CHAIN
    }
}
