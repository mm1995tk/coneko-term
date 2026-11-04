//! kitty graphics protocol の最初の段。APC で送られた画像を受け取り、持つ。
//!
//! 仕様: <https://sw.kovidgoyal.net/kitty/graphics-protocol/>
//!
//! # この段で扱うもの
//!
//! - 動作 `a=t`（送るだけ）と `a=q`（問い合わせ）。分割送信（`m=1` / `m=0`）、画像の番号
//!   （`i=`、`I=`）、返事の抑制（`q=1`、`q=2`）。
//! - 送り方（`t=`）は直接（`t=d`、既定）だけ。`t=f`・`t=t`・`t=s` には `EBADF` を返す。
//!   coneko-term は OS のファイルにも共有メモリにも触らない。
//! - 置く（`a=T`・`a=p`）、消す（`a=d`）、描く、Unicode placeholder、アニメーションは扱わない。
//!   扱わない動作は捨てる。
//!
//! # 画素を読み解かない
//!
//! PNG（`f=100`）と zlib の圧縮（`o=z`）のデータは、ここでは読み解かず、base64 を戻した
//! バイト列と `f`・`s`・`v`・`o` の値をそのまま [`Image`] に持つ。読み解くのは描く側
//! （ブラウザーの機能を使う）。生の画素（`f=24`、`f=32`）は、`s`・`v` と長さが合うかだけ確かめる。
//! PNG は IHDR から幅と高さを読む（圧縮されていて読めないときは、送られた `s`・`v`）。
//!
//! # 持つ量の上限
//!
//! 持つ画像のバイト列の合計に上限（[`DEFAULT_QUOTA`]）を置く。超えるときは、古い画像から
//! 捨てる。画像 1 つだけで上限を超えるときは `EFBIG` を返す。
//!
//! # 返事
//!
//! 返事は [`crate::Terminal::take_output`] の出口に入る。`i=` も `I=` も無い命令には返事をしない。

mod protocol;

use std::collections::VecDeque;

/// 持つ画像の合計の大きさの既定の上限（バイト）。kitty の既定と同じ 320 MiB。
pub const DEFAULT_QUOTA: usize = 320 * 1024 * 1024;

/// 画像のデータの形式（`f=`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    /// 生の RGB（`f=24`）。1 画素 3 バイト。
    Rgb,
    /// 生の RGBA（`f=32`）。1 画素 4 バイト。
    Rgba,
    /// PNG（`f=100`）。
    Png,
}

/// 受け取った画像。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    /// 画像の ID（`i=`）。`i=` が無かったときは、ターミナルが決めた ID。
    pub id: u32,
    /// 画像の番号（`I=`）。
    pub number: Option<u32>,
    /// データの形式。
    pub format: ImageFormat,
    /// zlib で圧縮されているか（`o=z`）。
    pub compressed: bool,
    /// 幅（画素）。0 は分からない（圧縮された PNG で `s=` が無かったとき）。
    pub width: u32,
    /// 高さ（画素）。0 は分からない。
    pub height: u32,
    /// base64 を戻したバイト列。圧縮も PNG もそのまま。
    pub data: Vec<u8>,
}

/// 持っている画像。
#[derive(Debug)]
pub(crate) struct Graphics {
    /// 古い順。
    images: VecDeque<Image>,
    bytes: usize,
    quota: usize,
    /// 分割送信の途中。
    pending: Option<protocol::Pending>,
    /// 次に ID を決めるときの候補。
    next_auto_id: u32,
}

/// ターミナルが決める ID はここから。クライアントが選ぶ小さな ID と重ならないようにする。
const AUTO_ID_BASE: u32 = 0x8000_0000;

impl Default for Graphics {
    fn default() -> Graphics {
        Graphics {
            images: VecDeque::new(),
            bytes: 0,
            quota: DEFAULT_QUOTA,
            pending: None,
            next_auto_id: AUTO_ID_BASE,
        }
    }
}

impl Graphics {
    pub(crate) fn get(&self, id: u32) -> Option<&Image> {
        self.images.iter().find(|i| i.id == id)
    }

    /// 同じ番号の画像のうち、いちばん新しいもの。
    pub(crate) fn get_by_number(&self, number: u32) -> Option<&Image> {
        self.images.iter().rev().find(|i| i.number == Some(number))
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &Image> {
        self.images.iter()
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    pub(crate) fn quota(&self) -> usize {
        self.quota
    }

    pub(crate) fn set_quota(&mut self, quota: usize) {
        self.quota = quota;
        self.evict_to(quota);
    }

    /// 全部捨てる。分割送信の途中も捨てる。
    pub(crate) fn clear(&mut self) {
        self.images.clear();
        self.bytes = 0;
        self.pending = None;
    }

    fn evict_to(&mut self, limit: usize) {
        while self.bytes > limit {
            match self.images.pop_front() {
                Some(img) => self.bytes -= img.data.len(),
                None => break,
            }
        }
    }

    /// `id` の画像があれば捨てる。
    fn remove(&mut self, id: u32) {
        if let Some(pos) = self.images.iter().position(|i| i.id == id) {
            if let Some(img) = self.images.remove(pos) {
                self.bytes -= img.data.len();
            }
        }
    }

    /// 画像を入れる。同じ ID の画像は置き換える。上限を超えるなら古いものから捨てる。
    /// 1 つだけで上限を超えるときは入れずに `false`。
    fn insert(&mut self, image: Image) -> bool {
        if image.data.len() > self.quota {
            return false;
        }
        self.remove(image.id);
        self.evict_to(self.quota - image.data.len());
        self.bytes += image.data.len();
        self.images.push_back(image);
        true
    }

    fn fresh_id(&mut self) -> u32 {
        loop {
            let id = self.next_auto_id;
            self.next_auto_id = self.next_auto_id.checked_add(1).unwrap_or(AUTO_ID_BASE);
            if self.get(id).is_none() {
                return id;
            }
        }
    }
}

#[cfg(test)]
mod tests;
