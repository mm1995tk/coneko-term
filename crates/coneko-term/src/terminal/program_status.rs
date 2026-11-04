//! kitty の Program Status Protocol（OSC 7501、改訂 0.4）。プログラムが「いま何をしているか」
//! （idle・working・done・blocked・error）を端末へ伝える。
//!
//! ここでは、報告を読み、記録（id ごとに 1 つ）を持ち、記録が変わったら [`ProgramStatusChange`] を
//! [`Event`](super::Event) として出すところまで受け持つ。何をどう見せるか（タブの印、通知など）は
//! 呼ぶ側が決める。
//!
//! 記録の寿命:
//!
//! - RIS は全部の記録を消す。DECSTR は消さない。
//! - 新しいシェルのプロンプト（OSC 133 の `A`）が来ると、working・blocked・idle の記録を消す。
//!   プロセスの終了（kitty の `launch --hold` のプログラムの終了）は端末の外のことなので、
//!   呼ぶ側が [`Terminal::program_status_drop_running`](super::Terminal::program_status_drop_running)
//!   で知らせる。
//! - done・error の記録は、窓がフォーカスを得たとき（[`Terminal::set_focus`](super::Terminal::set_focus)）
//!   か、キーを押したとき（[`Terminal::program_status_key_pressed`](super::Terminal::program_status_key_pressed)）に消える。

use super::clipboard::decode_base64;

/// 報告全体（OSC の `7501;` から ST の手前まで）の上限に足す、`ESC ]` と ST の分。
const FRAMING: usize = 4;
const MAX_SEQUENCE: usize = 4096;
const MAX_KEY: usize = 16;
const MAX_MSG_ENCODED: usize = 2732;
const MAX_MSG_DECODED: usize = 2048;
const MAX_TITLE_ENCODED: usize = 256;
const MAX_TITLE_DECODED: usize = 192;
const MAX_APP: usize = 32;
const MAX_ID: usize = 128;
const MAX_SEGMENT: usize = 32;
const MAX_DEPTH: usize = 8;
/// 端末ごとの記録の数の上限（仕様の下限は 64）。
pub(super) const MAX_RECORDS: usize = 256;

/// プログラムの状態。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProgramState {
    /// 待っている（次の指示を）。
    Idle,
    /// 動いている。
    Working,
    /// 一区切りの仕事が終わった。結果はまだ見られていない。
    Done,
    /// 利用者が何かするまで進めない。
    Blocked,
    /// 失敗して止まった。
    Error,
}

impl ProgramState {
    /// 報告での名前。
    pub fn name(self) -> &'static str {
        match self {
            ProgramState::Idle => "idle",
            ProgramState::Working => "working",
            ProgramState::Done => "done",
            ProgramState::Blocked => "blocked",
            ProgramState::Error => "error",
        }
    }

    fn parse(s: &str) -> Option<ProgramState> {
        Some(match s {
            "idle" => ProgramState::Idle,
            "working" => ProgramState::Working,
            "done" => ProgramState::Done,
            "blocked" => ProgramState::Blocked,
            "error" => ProgramState::Error,
            _ => return None,
        })
    }
}

/// `blocked` の理由の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProgramKind {
    /// 何かをする許可。
    Permission,
    /// 答えを打つ。
    Question,
    /// ログイン・トークン・資格情報。
    Auth,
}

impl ProgramKind {
    /// 報告での名前。
    pub fn name(self) -> &'static str {
        match self {
            ProgramKind::Permission => "permission",
            ProgramKind::Question => "question",
            ProgramKind::Auth => "auth",
        }
    }

    fn parse(s: &str) -> Option<ProgramKind> {
        Some(match s {
            "permission" => ProgramKind::Permission,
            "question" => ProgramKind::Question,
            "auth" => ProgramKind::Auth,
            _ => return None,
        })
    }
}

/// 記録の中身。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramStatus {
    /// 状態。
    pub state: ProgramState,
    /// `blocked` の理由（`blocked` のときだけ）。
    pub kind: Option<ProgramKind>,
    /// 進み具合 0〜100（`working` と `blocked` のときだけ。無ければ分からない）。
    pub progress: Option<u8>,
    /// プログラムの識別子。報告に無ければ、いちばん近い親の記録のものを使う。
    pub app: Option<String>,
    /// 短い名札。
    pub title: Option<String>,
    /// 1 行の説明。
    pub msg: Option<String>,
}

/// 記録が変わったこと。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramStatusChange {
    /// 記録の id（根の記録は空）。
    pub id: String,
    /// 新しい中身。`None` は記録が消えた。`app` は、変わった時点で親から引き継いだものを入れてある。
    pub status: Option<ProgramStatus>,
}

/// 読んだ報告。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Report {
    /// 問い合わせ（`?`）。
    Query,
    /// 記録を置き換える。
    Set { id: String, status: ProgramStatus },
    /// `state=clear`。
    Clear { id: Option<String> },
}

fn value_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b',' | b'+' | b'/' | b'=' | b'-')
}

fn segment_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'+' | b'-')
}

/// id の文法と上限を確かめる。
fn valid_id(id: &str) -> bool {
    if id.len() > MAX_ID {
        return false;
    }
    let mut depth = 0;
    for seg in id.split('/') {
        depth += 1;
        if depth > MAX_DEPTH
            || seg.is_empty()
            || seg.len() > MAX_SEGMENT
            || !seg.bytes().all(segment_byte)
        {
            return false;
        }
    }
    true
}

/// 制御文字（C0・DEL・C1）を含まない 1 行の文字列に直す。base64 は詰め物を省いてもよい。
fn text(value: &str, max_encoded: usize, max_decoded: usize) -> Option<String> {
    if value.len() > max_encoded {
        return None;
    }
    let mut padded = value.as_bytes().to_vec();
    while padded.len() % 4 != 0 {
        padded.push(b'=');
    }
    let bytes = decode_base64(&padded)?;
    if bytes.len() > max_decoded {
        return None;
    }
    let s = String::from_utf8(bytes).ok()?;
    if s.chars().any(char::is_control) {
        return None;
    }
    Some(s)
}

/// 本文を `key=value` の対に割る。値に使えない文字を含む対や、`=` の無い対は読み飛ばす。同じ鍵は
/// 最後が勝つ。報告ごと捨てるときは `None`（鍵が長すぎる。壊れた `id` を読み飛ばすと根の記録を
/// 書き換えてしまうので、それも捨てる）。
fn pairs(body: &str) -> Option<Vec<(&str, &str)>> {
    let mut pairs: Vec<(&str, &str)> = Vec::new();
    for pair in body.split(':') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        if k.is_empty() || !k.bytes().all(|b| b.is_ascii_lowercase()) {
            continue;
        }
        if !v.bytes().all(value_byte) {
            if k == "id" {
                return None;
            }
            continue;
        }
        if k.len() > MAX_KEY {
            return None;
        }
        pairs.retain(|(pk, _)| *pk != k);
        pairs.push((k, v));
    }
    Some(pairs)
}

/// 対から記録の中身を作る。先に全部の対を確かめ、上限を超えたものや壊れた文字列があれば `None`
/// （何も適用しない）。
fn status_of(pairs: &[(&str, &str)], state: ProgramState) -> Option<ProgramStatus> {
    let get = |key: &str| pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
    let decode = |key: &str, enc: usize, dec: usize| match get(key) {
        None => Some(None),
        Some(v) => text(v, enc, dec).map(Some),
    };
    let msg = decode("msg", MAX_MSG_ENCODED, MAX_MSG_DECODED)?;
    let title = decode("title", MAX_TITLE_ENCODED, MAX_TITLE_DECODED)?;
    if get("app").is_some_and(|a| a.len() > MAX_APP) {
        return None;
    }
    let running = matches!(state, ProgramState::Working | ProgramState::Blocked);
    let kind = get("kind")
        .and_then(ProgramKind::parse)
        .filter(|_| state == ProgramState::Blocked);
    let progress = get("progress")
        .filter(|_| running)
        .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|v| v.parse::<u8>().ok())
        .filter(|&p| p <= 100);
    let app = get("app")
        .filter(|a| !a.is_empty() && a.bytes().all(segment_byte))
        .map(str::to_string);
    Some(ProgramStatus {
        state,
        kind,
        progress,
        app,
        title,
        msg,
    })
}

/// OSC 7501 の `data`（`7501;` のあと）を読む。捨てる報告は `None`。
/// `sequence_len` は、OSC の本体（`7501;` を含む）の長さ。
pub(super) fn parse(body: &[u8], sequence_len: usize) -> Option<Report> {
    if sequence_len + FRAMING > MAX_SEQUENCE {
        return None;
    }
    let body = std::str::from_utf8(body).ok()?;
    if body.trim() == "?" {
        return Some(Report::Query);
    }
    let pairs = pairs(body)?;
    let get = |key: &str| pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
    // 不正な id は、根の記録へ落とさず、報告ごと無視する。
    let id = match get("id") {
        Some(id) if valid_id(id) => Some(id.to_string()),
        Some(_) => return None,
        None => None,
    };
    let state = get("state")?;
    if state == "clear" {
        // 他の対も、上限と文字列は確かめる。
        status_of(&pairs, ProgramState::Idle)?;
        return Some(Report::Clear { id });
    }
    let status = status_of(&pairs, ProgramState::parse(state)?)?;
    Some(Report::Set {
        id: id.unwrap_or_default(),
        status,
    })
}

/// `OSC 7501 ; ?` への返事の本体（`ESC ]` と ST は含まない）。
pub(super) const QUERY_REPLY: &str =
    "7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth";

/// 記録の入れ物。更新が古い順に並べる。
#[derive(Debug, Default)]
pub(super) struct Records {
    list: Vec<(String, ProgramStatus)>,
}

fn is_descendant(id: &str, of: &str) -> bool {
    id.len() > of.len() && id.starts_with(of) && id.as_bytes()[of.len()] == b'/'
}

impl Records {
    /// 親をたどって、`app` の無い記録に引き継ぐ。
    fn resolve(&self, id: &str, mut status: ProgramStatus) -> ProgramStatus {
        if status.app.is_none() {
            let mut cur = id;
            while let Some((parent, _)) = cur.rsplit_once('/') {
                cur = parent;
                if let Some(app) = self.find(cur).and_then(|s| s.app.clone()) {
                    status.app = Some(app);
                    return status;
                }
            }
            if !id.is_empty() {
                status.app = self.find("").and_then(|s| s.app.clone());
            }
        }
        status
    }

    fn find(&self, id: &str) -> Option<&ProgramStatus> {
        self.list.iter().find(|(i, _)| i == id).map(|(_, s)| s)
    }

    /// 記録を置き換える。消えた記録（上限で追い出したもの）があれば、あとに続ける。
    pub(super) fn set(&mut self, id: String, status: ProgramStatus) -> Vec<ProgramStatusChange> {
        self.list.retain(|(i, _)| *i != id);
        let mut changes = Vec::new();
        if self.list.len() >= MAX_RECORDS {
            let (old, _) = self.list.remove(0);
            changes.push(ProgramStatusChange {
                id: old,
                status: None,
            });
        }
        self.list.push((id.clone(), status.clone()));
        let resolved = self.resolve(&id, status);
        changes.insert(
            0,
            ProgramStatusChange {
                id,
                status: Some(resolved),
            },
        );
        changes
    }

    /// `id` とその下の記録を消す。`None` は全部。
    pub(super) fn clear(&mut self, id: Option<&str>) -> Vec<ProgramStatusChange> {
        self.remove_where(|i, _| match id {
            None => true,
            Some(id) => i == id || is_descendant(i, id),
        })
    }

    pub(super) fn remove_where(
        &mut self,
        mut f: impl FnMut(&str, &ProgramStatus) -> bool,
    ) -> Vec<ProgramStatusChange> {
        let mut gone = Vec::new();
        self.list.retain(|(i, s)| {
            if f(i, s) {
                gone.push(ProgramStatusChange {
                    id: i.clone(),
                    status: None,
                });
                false
            } else {
                true
            }
        });
        gone
    }

    /// 持っている記録を、更新が古い順に。`app` は親から引き継いだもの。
    pub(super) fn all(&self) -> Vec<(String, ProgramStatus)> {
        self.list
            .iter()
            .map(|(i, s)| (i.clone(), self.resolve(i, s.clone())))
            .collect()
    }
}
