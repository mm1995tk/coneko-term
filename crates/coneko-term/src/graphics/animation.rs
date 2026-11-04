//! kitty graphics のアニメーション（`a=f`・`a=a`・`a=c`）。
//!
//! コマの絵は合成しない。Rust の本体は、コマを作る「操作」（データを置く、別のコマの四角を
//! 重ねる）を届いた順に持ち、コマごとの表示時間と再生の状態を持つだけ。PNG と zlib を読み
//! 解けるのは描く側（ブラウザー）だけなので、合成も描く側が、操作を順に当てて行う。
//! 時計も持たない。次のコマへ進める時刻は描く側が決め、[`Animation::step`] で知らせる。

use super::protocol::{last, num_key, signed, Fail, Reply, EINVAL};
use super::{Graphics, Image, ImageFormat};

/// コマの表示時間の既定（ミリ秒）。根のコマ（1 コマ目）は 0。
pub const DEFAULT_GAP: i32 = 40;

/// 1 つの画像が持てるコマの数の上限。
pub const MAX_FRAMES: usize = 1000;

/// 1 つの画像が持てる操作の数の上限。
pub const MAX_OPS: usize = 10_000;

/// 操作 1 つが、データのほかに上限へ数えるバイト数。
const OP_COST: usize = 64;

/// 新しい（または編集する）コマの下地。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameBase {
    /// 編集するコマそのもの（`r=`）。
    Edit,
    /// 別のコマの絵（`c=`、1 始まり）。
    Frame(u32),
    /// 1 色（`Y=`、0xRRGGBBAA）。
    Color(u32),
}

/// コマを作る操作。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameOp {
    /// コマ `target`（1 始まり。いまのコマ数 + 1 なら新しいコマ）の下地に、データを `(x, y)` から置く。
    Load {
        /// 作る（編集する）コマ。
        target: u32,
        /// 下地。
        base: FrameBase,
        /// データを置く左端（画素）。
        x: u32,
        /// データを置く上端（画素）。
        y: u32,
        /// データの幅（画素）。0 は分からない（圧縮された PNG）。読み解いた大きさを使う。
        width: u32,
        /// データの高さ（画素）。0 は分からない。
        height: u32,
        /// 置き換え（`X=1`）か。そうでなければアルファ合成。
        overwrite: bool,
        /// データの形式。
        format: ImageFormat,
        /// zlib で圧縮されているか。
        compressed: bool,
        /// base64 を戻したバイト列（圧縮も PNG もそのまま）。
        data: Vec<u8>,
    },
    /// コマ `src` の四角を、コマ `dst` の四角に重ねる（`a=c`）。
    Compose {
        /// 重ねる元のコマ。
        src: u32,
        /// 重ねられるコマ。
        dst: u32,
        /// 元の四角の左端。
        src_x: u32,
        /// 元の四角の上端。
        src_y: u32,
        /// 重ねる先の四角の左端。
        dst_x: u32,
        /// 重ねる先の四角の上端。
        dst_y: u32,
        /// 四角の幅。
        width: u32,
        /// 四角の高さ。
        height: u32,
        /// 置き換え（`C=1`）か。
        overwrite: bool,
    },
}

impl FrameOp {
    fn cost(&self) -> usize {
        OP_COST
            + match self {
                FrameOp::Load { data, .. } => data.len(),
                FrameOp::Compose { .. } => 0,
            }
    }
}

/// 再生の状態（`a=a` の `s=`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    /// 止まっている（`s=1`）。はじめはこれ。
    Stopped,
    /// 回す。最後のコマに着いたら、次のコマが届くまで待つ（`s=2`）。
    Loading,
    /// 回す。最後のコマの次は最初に戻る（`s=3`）。
    Running,
}

/// 画像のアニメーション。画像の 1 コマ目（根）は画像そのもの。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Animation {
    /// コマを作る操作を、届いた順に。
    pub ops: Vec<FrameOp>,
    /// コマごとの表示時間（ミリ秒）。1 つ目が根。負は、すぐ次へ進む（見せない）コマ。
    pub gaps: Vec<i32>,
    /// 再生の状態。
    pub state: PlayState,
    /// いまのコマ（1 始まり）。
    pub current: u32,
    /// 回す回数。0 は止めるまで回し続ける。
    pub loops: u32,
    /// 回し終えた回数。
    pub played: u32,
}

impl Default for Animation {
    fn default() -> Animation {
        Animation {
            ops: Vec::new(),
            gaps: vec![0],
            state: PlayState::Stopped,
            current: 1,
            loops: 0,
            played: 0,
        }
    }
}

impl Animation {
    /// コマの数（根を含む）。
    pub fn frame_count(&self) -> usize {
        self.gaps.len()
    }

    /// いまのコマの表示時間（ミリ秒）。
    pub fn current_gap(&self) -> i32 {
        self.gaps
            .get(self.current as usize - 1)
            .copied()
            .unwrap_or(0)
    }

    /// 操作が使うバイト数（画像の上限に数える）。
    pub(super) fn bytes(&self) -> usize {
        self.ops.iter().map(FrameOp::cost).sum()
    }

    /// いまのコマの表示時間が過ぎたと知らされたとき、次のコマへ進める。進んだら `true`。
    ///
    /// 表示時間が負のコマは飛ばす。最後のコマの次は、`Running` なら最初に戻る（決めた回数を
    /// 回し終えていたら止まる）。`Loading` なら動かず待つ。止まっていれば何もしない。
    pub fn step(&mut self) -> bool {
        if self.state == PlayState::Stopped {
            return false;
        }
        let n = self.frame_count() as u32;
        let mut cur = self.current;
        let mut played = self.played;
        for _ in 0..=n {
            cur += 1;
            if cur > n {
                if self.state == PlayState::Loading {
                    return false;
                }
                played += 1;
                if self.loops != 0 && played >= self.loops {
                    self.state = PlayState::Stopped;
                    self.played = 0;
                    return false;
                }
                cur = 1;
            }
            if self.gaps[cur as usize - 1] >= 0 {
                self.current = cur;
                self.played = played;
                return true;
            }
        }
        false
    }
}

/// `a=f` の、画像の送り方以外のキー。
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct FrameKeys {
    x: u32,
    y: u32,
    base: u32,
    edit: u32,
    gap: i32,
    overwrite: bool,
    background: u32,
}

pub(super) fn parse_frame_keys(keys: &[(u8, &[u8])]) -> Result<FrameKeys, Fail> {
    let n = |key| num_key(keys, key).map_err(|_| (EINVAL, "invalid frame key"));
    let gap = match last(keys, b'z') {
        None => 0,
        Some(v) => signed(v).ok_or((EINVAL, "invalid gap"))?,
    };
    Ok(FrameKeys {
        x: n(b'x')?.unwrap_or(0),
        y: n(b'y')?.unwrap_or(0),
        base: n(b'c')?.unwrap_or(0),
        edit: n(b'r')?.unwrap_or(0),
        gap,
        overwrite: n(b'X')? == Some(1),
        background: n(b'Y')?.unwrap_or(0),
    })
}

const ENOENT: Fail = ("ENOENT", "image or frame not found");

impl Graphics {
    /// `i=` か `I=` で画像を引く。
    fn target_image(&self, reply: &Reply) -> Result<u32, Fail> {
        if reply.id != 0 && reply.number != 0 {
            return Err((EINVAL, "cannot specify both i and I"));
        }
        let image = if reply.id != 0 {
            self.get(reply.id)
        } else if reply.number != 0 {
            self.get_by_number(reply.number)
        } else {
            return Err((EINVAL, "image id or number is required"));
        };
        image.map(|i| i.id).ok_or(ENOENT)
    }

    fn image_mut(&mut self, id: u32) -> Option<&mut Image> {
        self.images.iter_mut().find(|i| i.id == id)
    }

    /// `a=f` の本体。`data` は検査済みの、コマのデータ（`id` と番号は使わない）。
    pub(super) fn add_frame(
        &mut self,
        reply: &Reply,
        k: FrameKeys,
        data: Image,
    ) -> Result<u32, (u32, Fail)> {
        let id = self.target_image(reply).map_err(|f| (0, f))?;
        let img = self.get(id).ok_or((0, ENOENT))?;
        let count = img.animation.frame_count() as u32;
        // 四角が画像からはみ出さないか（画像の大きさが分からないときは確かめない）。
        let fits = |start: u32, len: u32, total: u32| {
            total == 0 || start.checked_add(len).is_some_and(|e| e <= total)
        };
        if !fits(k.x, data.width, img.width) || !fits(k.y, data.height, img.height) {
            return Err((id, (EINVAL, "frame data is outside the image")));
        }
        let (target, base) = if k.edit == 0 {
            if count as usize >= MAX_FRAMES || img.animation.ops.len() >= MAX_OPS {
                return Err((id, ("ENOSPC", "too many frames")));
            }
            let base = if k.base == 0 {
                FrameBase::Color(k.background)
            } else if k.base <= count {
                FrameBase::Frame(k.base)
            } else {
                return Err((id, ENOENT));
            };
            (count + 1, base)
        } else if k.edit <= count {
            if img.animation.ops.len() >= MAX_OPS {
                return Err((id, ("ENOSPC", "too many frames")));
            }
            (k.edit, FrameBase::Edit)
        } else {
            return Err((id, ENOENT));
        };
        let op = FrameOp::Load {
            target,
            base,
            x: k.x,
            y: k.y,
            width: data.width,
            height: data.height,
            overwrite: k.overwrite,
            format: data.format,
            compressed: data.compressed,
            data: data.data,
        };
        self.push_op(id, op).map_err(|f| (id, f))?;
        if let Some(img) = self.image_mut(id) {
            let gaps = &mut img.animation.gaps;
            if target > count {
                gaps.push(if k.gap == 0 { DEFAULT_GAP } else { k.gap });
            } else if k.gap != 0 {
                gaps[target as usize - 1] = k.gap;
            }
        }
        Ok(id)
    }

    /// 操作を足す。上限を超えるなら、ほかの画像を古いものから捨てる。足せなければ `EFBIG`。
    fn push_op(&mut self, id: u32, op: FrameOp) -> Result<(), Fail> {
        let cost = op.cost();
        let own = self.get(id).map_or(0, Image::stored_bytes);
        if own + cost > self.quota {
            return Err(("EFBIG", "image is larger than the storage quota"));
        }
        self.evict_keeping(self.quota - cost, id);
        if let Some(img) = self.image_mut(id) {
            img.animation.ops.push(op);
            self.bytes += cost;
        }
        Ok(())
    }

    /// `a=a`。
    pub(super) fn animate_command(&mut self, keys: &[(u8, &[u8])], out: &mut Vec<u8>) {
        let reply = Reply::from_keys(keys);
        match self.animate(keys, &reply) {
            Ok(id) => reply.send(id, Ok(()), out),
            Err((id, f)) => reply.send(id, Err(f), out),
        }
    }

    fn animate(&mut self, keys: &[(u8, &[u8])], reply: &Reply) -> Result<u32, (u32, Fail)> {
        let id = self.target_image(reply).map_err(|f| (0, f))?;
        let n = |key| num_key(keys, key).map_err(|_| (id, (EINVAL, "invalid animation key")));
        let state = n(b's')?;
        let edit = n(b'r')?.unwrap_or(0);
        let current = n(b'c')?.unwrap_or(0);
        let loops = n(b'v')?.unwrap_or(0);
        let gap = match last(keys, b'z') {
            None => 0,
            Some(v) => signed(v).ok_or((id, (EINVAL, "invalid gap")))?,
        };
        let img = self.image_mut(id).ok_or((0, ENOENT))?;
        let anim = &mut img.animation;
        let count = anim.frame_count() as u32;
        // 先に全部確かめてから変える。
        if (edit != 0 && gap != 0 && edit > count) || current > count {
            return Err((id, ENOENT));
        }
        if edit != 0 && gap != 0 {
            anim.gaps[edit as usize - 1] = gap;
        }
        if current != 0 {
            anim.current = current;
        }
        if loops != 0 {
            anim.loops = if loops == 1 { 0 } else { loops - 1 };
            anim.played = 0;
        }
        match state {
            Some(1) => {
                anim.state = PlayState::Stopped;
                anim.played = 0;
            }
            Some(2) => anim.state = PlayState::Loading,
            Some(3) => anim.state = PlayState::Running,
            // 知らない値と、無いときは変えない。
            _ => {}
        }
        Ok(id)
    }

    /// `a=c`。
    pub(super) fn compose_command(&mut self, keys: &[(u8, &[u8])], out: &mut Vec<u8>) {
        let reply = Reply::from_keys(keys);
        match self.compose(keys, &reply) {
            Ok(id) => reply.send(id, Ok(()), out),
            Err((id, f)) => reply.send(id, Err(f), out),
        }
    }

    fn compose(&mut self, keys: &[(u8, &[u8])], reply: &Reply) -> Result<u32, (u32, Fail)> {
        let id = self.target_image(reply).map_err(|f| (0, f))?;
        let n = |key| {
            num_key(keys, key)
                .map(|v| v.unwrap_or(0))
                .map_err(|_| (id, (EINVAL, "invalid composition key")))
        };
        let (src, dst) = (n(b'r')?, n(b'c')?);
        let (dst_x, dst_y) = (n(b'x')?, n(b'y')?);
        let (src_x, src_y) = (n(b'X')?, n(b'Y')?);
        let (w, h) = (n(b'w')?, n(b'h')?);
        let overwrite = n(b'C')? == 1;
        let img = self.get(id).ok_or((0, ENOENT))?;
        let count = img.animation.frame_count() as u32;
        if src == 0 || dst == 0 || src > count || dst > count {
            return Err((id, ENOENT));
        }
        if img.animation.ops.len() >= MAX_OPS {
            return Err((id, ("ENOSPC", "too many frames")));
        }
        let (iw, ih) = (img.width, img.height);
        if iw == 0 || ih == 0 {
            return Err((id, (EINVAL, "the image size is unknown")));
        }
        let (width, height) = (if w == 0 { iw } else { w }, if h == 0 { ih } else { h });
        let inside =
            |start: u32, len: u32, total: u32| start.checked_add(len).is_some_and(|e| e <= total);
        if !inside(src_x, width, iw)
            || !inside(src_y, height, ih)
            || !inside(dst_x, width, iw)
            || !inside(dst_y, height, ih)
        {
            return Err((id, (EINVAL, "rectangle is outside the image")));
        }
        let overlap = |a: u32, b: u32, len: u32| a < b + len && b < a + len;
        if src == dst && overlap(src_x, dst_x, width) && overlap(src_y, dst_y, height) {
            return Err((id, (EINVAL, "source and destination rectangles overlap")));
        }
        self.push_op(
            id,
            FrameOp::Compose {
                src,
                dst,
                src_x,
                src_y,
                dst_x,
                dst_y,
                width,
                height,
                overwrite,
            },
        )
        .map_err(|f| (id, f))?;
        Ok(id)
    }

    /// 画像 `id` のいまのコマの表示時間が過ぎたと知らされて、次のコマへ進める。進んだら `true`。
    pub(crate) fn step_animation(&mut self, id: u32) -> bool {
        self.image_mut(id).is_some_and(|i| i.animation.step())
    }
}
