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
//! - 置く（`a=T`・`a=p`）と消す（`a=d`）は、次の「置く」「消す」の節。
//! - `U=1` の配置は画面に置かず、仮の配置として持つ。字（U+10EEEE）が引く。次の「Unicode placeholder」の節。
//! - アニメーション（`a=f`・`a=a`・`a=c`）は次の「アニメーション」の節。
//! - 親子の配置（`P=`・`Q=`）は扱わない。扱わない動作は捨てる。
//!
//! # 置く（配置）
//!
//! 配置（[`Placement`]）は、画像のどの部分を、どのセルから何セルぶんの大きさで、どの `z` で
//! 置くかを表す。画面の行（`Line`）に付けて持つ。行が動けば配置も動き（スクロール、IL・DL、
//! スクロールバックへの押し出し）、行が捨てられれば配置も消える。主画面と代替画面は別々に持つ
//! （代替画面に入ると空から始まり、出ると捨てる）。画像のデータ自体は 1 つの表で共有する。
//! 配置は [`crate::Terminal::placements`] で読む。
//!
//! - 位置は、命令を受けたときのカーソルのセル（左上）。`C=1` で無ければ、置いたあと
//!   カーソルを `r - 1` 行ぶん LF で下げ（スクロール範囲の下端ならスクロールする）、
//!   列を `c` だけ右へ動かす（右端を超えるときは右端）。
//! - `c`・`r` が無いときは、元の画像の部分の画素の大きさと、セルの画素の大きさ
//!   （[`crate::Terminal::set_cell_size`]）から決める。片方だけあるときは、縦横比を保つように
//!   もう片方を決める。
//! - `X`・`Y` はセルの大きさより小さい値に収める。`x`・`y`・`w`・`h` は画像の外にはみ出す分を
//!   切る（`x`・`y` が画像の外なら `EINVAL`）。`w`・`h` が 0 のときは端まで。
//! - `p` があり、同じ画像に同じ `p` の配置が同じ画面にあれば、置き換える。
//! - 同じ ID で画像を送り直したとき、古い画像を捨てたとき（上限を超えたとき）は、その画像の
//!   配置も消える。
//! - ED 2（画面の消去）と DECALN は、見えている画面の配置を消す。ED 0・ED 1・EL・ECH などの
//!   セルの消去は、配置を消さない（kitty と同じ）。
//! - 大きさを変えて組み直す（reflow）ときは、配置は元のセルの位置（字の並びの中の位置）と
//!   一緒に動く。代替画面で列が減るときは、はみ出した列の配置を捨てる。
//!
//! # 消す（`a=d`）
//!
//! `d=` の値（`a`・`i`・`n`・`c`・`p`・`q`・`x`・`y`・`z`。`f` は何もしない）を扱う。大文字は、
//! 配置が無くなった画像のデータも捨てる（別の配置が残っていれば捨てない）。`i`・`n` は
//! スクロールバックを含む画面全体の配置が対象で、そのほかは見えている画面（の一部でも
//! 見えている配置）が対象。`d` が無いときは `a`。知らない値は何もしない。
//!
//! # Unicode placeholder
//!
//! `U=1` の `a=p`（`a=T` でもよい）は、画面に置かず、仮の配置（`c` x `r` セルの枠）として持つ。
//! カーソルは動かない。U+10EEEE の字（前景色が画像の ID、下線の色が配置の番号、結合文字が行・列・
//! ID の上位 8 ビット）は、ふつうの字としてセルに持ち、[`crate::Terminal::placements`] が読むたびに、
//! 画像の該当する部分を描く配置（[`Placement::dest_size`] つき）にして返す。結合文字を省いたとき
//! の推し量りは、仕様の 3 つの規則を左から順に当てる。画像は枠に縦横比を保って収め、中央に置く。
//! 仮の配置は、同じ画像の同じ番号で置き直すと置き換わる。`d=i`・`d=n`（と大文字）で消え、画像が
//! 消えても消える。ほかの `d=` は触らない。
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

mod animation;
mod diacritics;
mod place;
mod placeholder;
mod protocol;

pub use animation::{Animation, FrameBase, FrameOp, PlayState, DEFAULT_GAP, MAX_FRAMES, MAX_OPS};
pub(crate) use place::{DeleteSpec, DeleteTarget, Effect};
pub(crate) use placeholder::derive as derive_placeholders;
pub use placeholder::PLACEHOLDER;

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
    /// 持った順に増える通し番号。同じ ID で送り直すと変わる。描く側が、読み解いた画像を
    /// 使い回してよいか（同じ ID で別の画像になっていないか）を確かめるのに使う。
    pub serial: u64,
    /// アニメーション（コマの操作と再生の状態）。コマを足していなければ空で、止まっている。
    pub animation: Animation,
}

impl Image {
    /// 上限に数えるバイト数。画像のデータと、コマの操作。
    pub fn stored_bytes(&self) -> usize {
        self.data.len() + self.animation.bytes()
    }
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
    /// 次に画像へ付ける通し番号（[`Image::serial`]）。
    next_serial: u64,
    /// セルの大きさ（画素。幅, 高さ）。`c`・`r` が無い配置の大きさを決めるのに使う。
    cell_size: (u32, u32),
    /// 捨てた（または送り直して置き換えた）画像の ID。配置を消すために取り出す。
    removed: Vec<u32>,
    /// 画面に頼む仕事。
    effects: Vec<Effect>,
    /// 仮の配置（`U=1`）。画面には置かず、Unicode placeholder が引く。
    virtuals: Vec<Placement>,
}

/// セルの大きさの既定（画素。幅, 高さ）。
pub const DEFAULT_CELL_SIZE: (u32, u32) = (10, 20);

/// 画像の置き方（配置）。位置は、持っている行と `col` で決まる。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    /// 置く画像の ID。
    pub image_id: u32,
    /// 配置の番号（`p=`）。0 は番号が無い。
    pub placement_id: u32,
    /// 左上のセルの列（0 始まり）。
    pub col: usize,
    /// 元の画像の使う部分の左端（画素）。
    pub src_x: u32,
    /// 元の画像の使う部分の上端（画素）。
    pub src_y: u32,
    /// 元の画像の使う部分の幅（画素）。0 は右端まで（画像の大きさが分からないときだけ）。
    pub src_w: u32,
    /// 元の画像の使う部分の高さ（画素）。0 は下端まで（画像の大きさが分からないときだけ）。
    pub src_h: u32,
    /// 左上のセルの中でのずれ、横（画素）。セルの幅より小さい。
    pub offset_x: u32,
    /// 左上のセルの中でのずれ、縦（画素）。セルの高さより小さい。
    pub offset_y: u32,
    /// 使うセルの列数（1 以上）。
    pub cols: u32,
    /// 使うセルの行数（1 以上）。
    pub rows: u32,
    /// 重ねる順（`z=`）。負の値は字の下に描く。
    pub z: i32,
    /// `c` も `r` も無く、元の部分を拡大も縮小もせず（画素のまま）置くか。そうでなければ、
    /// `cols` x `rows` のセルいっぱいに引き伸ばして描く。
    pub native_size: bool,
    /// 画素で決めた描く大きさ（幅, 高さ）。Unicode placeholder の配置が使う。`None` は
    /// `native_size` と `cols` x `rows` で決める。
    pub dest_size: Option<(u32, u32)>,
}

/// 画面に置いてある配置と、その位置。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlacedImage {
    /// 見えている画面の一番上の行を 0 とした行。スクロールバックの中は負。
    pub row: i64,
    /// 配置。
    pub placement: Placement,
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
            next_serial: 1,
            cell_size: DEFAULT_CELL_SIZE,
            removed: Vec::new(),
            effects: Vec::new(),
            virtuals: Vec::new(),
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

    pub(crate) fn cell_size(&self) -> (u32, u32) {
        self.cell_size
    }

    pub(crate) fn set_cell_size(&mut self, w: u32, h: u32) {
        if w > 0 && h > 0 {
            self.cell_size = (w, h);
        }
    }

    /// 仮の配置を足す。同じ画像の同じ番号のものは置き換える。
    pub(crate) fn add_virtual(&mut self, p: Placement) {
        self.virtuals
            .retain(|v| !(v.image_id == p.image_id && v.placement_id == p.placement_id));
        self.virtuals.push(p);
    }

    /// 仮の配置があるか。
    pub(crate) fn has_virtuals(&self) -> bool {
        !self.virtuals.is_empty()
    }

    /// 画像 `image_id` の仮の配置。`placement_id` が 0 なら、その画像のどれでもよい（新しいもの）。
    pub(crate) fn virtual_placement(&self, image_id: u32, placement_id: u32) -> Option<&Placement> {
        self.virtuals.iter().rev().find(|v| {
            v.image_id == image_id && (placement_id == 0 || v.placement_id == placement_id)
        })
    }

    pub(crate) fn take_removed(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.removed)
    }

    pub(crate) fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
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
        self.virtuals.clear();
    }

    fn evict_to(&mut self, limit: usize) {
        while self.bytes > limit {
            match self.images.pop_front() {
                Some(img) => {
                    self.bytes -= img.stored_bytes();
                    self.removed.push(img.id);
                    self.virtuals.retain(|v| v.image_id != img.id);
                }
                None => break,
            }
        }
    }

    /// `keep` 以外の画像を古い順に、合計が `limit` 以下になるまで捨てる。
    pub(super) fn evict_keeping(&mut self, limit: usize, keep: u32) {
        while self.bytes > limit {
            let Some(pos) = self.images.iter().position(|i| i.id != keep) else {
                break;
            };
            if let Some(img) = self.images.remove(pos) {
                self.bytes -= img.stored_bytes();
                self.removed.push(img.id);
                self.virtuals.retain(|v| v.image_id != img.id);
            }
        }
    }

    /// `id` の画像があれば捨てる。
    pub(crate) fn remove(&mut self, id: u32) {
        if let Some(pos) = self.images.iter().position(|i| i.id == id) {
            if let Some(img) = self.images.remove(pos) {
                self.bytes -= img.stored_bytes();
                self.removed.push(id);
                self.virtuals.retain(|v| v.image_id != id);
            }
        }
    }

    /// 画像を入れる。同じ ID の画像は置き換える。上限を超えるなら古いものから捨てる。
    /// 1 つだけで上限を超えるときは入れずに `false`。
    fn insert(&mut self, mut image: Image) -> bool {
        if image.stored_bytes() > self.quota {
            return false;
        }
        self.remove(image.id);
        self.evict_to(self.quota - image.stored_bytes());
        self.bytes += image.stored_bytes();
        image.serial = self.next_serial;
        self.next_serial += 1;
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
