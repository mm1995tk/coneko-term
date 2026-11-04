//! `a=p`・`a=T` の置き方の読み取りと、`a=d` の読み取り。

use super::protocol::{lenient, num_key, signed, Fail, Reply, EINVAL};
use super::{Graphics, Image, Placement};

/// 1 つの配置が使うセルの数の上限（行も列も）。
const MAX_CELLS: u32 = 10_000;

/// 置き方のキー（読んだままの値。画像の大きさでの補正は [`resolve`]）。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PlaceKeys {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    offset_x: u32,
    offset_y: u32,
    cols: u32,
    rows: u32,
    z: i32,
    placement_id: u32,
    no_move: bool,
}

/// 画面に頼む仕事。
#[derive(Clone, Debug)]
pub(crate) enum Effect {
    /// カーソルの位置に置く。`move_cursor` は、置いたあとカーソルを動かすか。
    Place {
        placement: Placement,
        move_cursor: bool,
    },
    /// 消す。
    Delete(DeleteSpec),
}

/// `a=d` の対象。`x`・`y` は 1 始まり（kitty と同じ）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeleteTarget {
    /// 見えている配置すべて。
    All,
    /// 画像の配置。`placement` が 0 でなければその配置だけ。
    Image { id: u32, placement: u32 },
    /// カーソルのセルにかかる配置。
    Cursor,
    /// セル `(x, y)` にかかる配置。
    Cell { x: u32, y: u32 },
    /// セル `(x, y)` にかかり、`z` が合う配置。
    CellZ { x: u32, y: u32, z: i32 },
    /// 列 `x` にかかる配置。
    Column(u32),
    /// 行 `y` にかかる配置。
    Row(u32),
    /// `z` が合う配置。
    Z(i32),
}

/// `a=d` の中身。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DeleteSpec {
    pub target: DeleteTarget,
    /// 大文字: 配置が無くなった画像のデータも捨てる。
    pub free: bool,
}

impl PlaceKeys {
    pub(super) fn no_move(&self) -> bool {
        self.no_move
    }

    pub(super) fn placement_id(&self) -> u32 {
        self.placement_id
    }
}

pub(super) fn parse_keys(keys: &[(u8, &[u8])]) -> Result<PlaceKeys, Fail> {
    let n = |key| num_key(keys, key).map_err(|_| (EINVAL, "invalid placement key"));
    let z = match keys.iter().rev().find(|(k, _)| *k == b'z') {
        None => 0,
        Some((_, v)) => signed(v).ok_or((EINVAL, "invalid z-index"))?,
    };
    Ok(PlaceKeys {
        x: n(b'x')?.unwrap_or(0),
        y: n(b'y')?.unwrap_or(0),
        w: n(b'w')?.unwrap_or(0),
        h: n(b'h')?.unwrap_or(0),
        offset_x: n(b'X')?.unwrap_or(0),
        offset_y: n(b'Y')?.unwrap_or(0),
        cols: n(b'c')?.unwrap_or(0),
        rows: n(b'r')?.unwrap_or(0),
        z,
        placement_id: n(b'p')?.unwrap_or(0),
        no_move: n(b'C')? == Some(1),
    })
}

fn div_ceil(a: u64, b: u64) -> u64 {
    a.div_ceil(b.max(1))
}

/// 画像と置き方のキーから、配置（`col` は 0。あとで画面が決める）を作る。
pub(super) fn resolve(
    image: &Image,
    k: &PlaceKeys,
    placement_id: u32,
    cell: (u32, u32),
) -> Result<Placement, Fail> {
    let (iw, ih) = (image.width, image.height);
    let known = iw > 0 && ih > 0;
    if known && (k.x >= iw || k.y >= ih) {
        return Err((EINVAL, "source rectangle is outside the image"));
    }
    let clip = |start: u32, len: u32, total: u32| {
        if !known {
            len
        } else if len == 0 {
            total - start
        } else {
            len.min(total - start)
        }
    };
    let (sw, sh) = (clip(k.x, k.w, iw), clip(k.y, k.h, ih));
    let (cw, ch) = cell;
    let (offset_x, offset_y) = (k.offset_x.min(cw - 1), k.offset_y.min(ch - 1));
    let (sw64, sh64) = (u64::from(sw), u64::from(sh));
    let (cw64, ch64) = (u64::from(cw), u64::from(ch));
    let by_size = |px: u64, off: u64, cell: u64| div_ceil(px + off, cell);
    let (cols, rows) = match (k.cols, k.rows) {
        (0, 0) => (
            by_size(sw64, offset_x.into(), cw64),
            by_size(sh64, offset_y.into(), ch64),
        ),
        (c, 0) => {
            let c = u64::from(c);
            let r = if sw > 0 && sh > 0 {
                div_ceil(sh64 * c * cw64, sw64 * ch64)
            } else {
                1
            };
            (c, r)
        }
        (0, r) => {
            let r = u64::from(r);
            let c = if sw > 0 && sh > 0 {
                div_ceil(sw64 * r * ch64, sh64 * cw64)
            } else {
                1
            };
            (c, r)
        }
        (c, r) => (c.into(), r.into()),
    };
    let clamp = |n: u64| n.clamp(1, MAX_CELLS.into()) as u32;
    Ok(Placement {
        image_id: image.id,
        placement_id,
        col: 0,
        src_x: k.x,
        src_y: k.y,
        src_w: sw,
        src_h: sh,
        offset_x,
        offset_y,
        cols: clamp(cols),
        rows: clamp(rows),
        z: k.z,
        native_size: k.cols == 0 && k.rows == 0,
    })
}

impl Graphics {
    /// `a=p`。画像を ID か番号で引いて置く。返事はここで返す。
    pub(super) fn place_command(&mut self, keys: &[(u8, &[u8])], reply: Reply, out: &mut Vec<u8>) {
        let result = self.place_inner(keys, &reply);
        match result {
            Ok((id, effect)) => {
                self.effects.push(effect);
                reply.send(id, Ok(()), out);
            }
            Err((id, f)) => reply.send(id, Err(f), out),
        }
    }

    fn place_inner(
        &self,
        keys: &[(u8, &[u8])],
        reply: &Reply,
    ) -> Result<(u32, Effect), (u32, Fail)> {
        if reply.id != 0 && reply.number != 0 {
            return Err((0, (EINVAL, "cannot specify both i and I")));
        }
        let image = if reply.id != 0 {
            self.get(reply.id)
        } else if reply.number != 0 {
            self.get_by_number(reply.number)
        } else {
            return Err((0, (EINVAL, "image id or number is required")));
        };
        let id = image.map_or(0, |i| i.id);
        let Some(image) = image else {
            return Err((0, ("ENOENT", "image not found")));
        };
        let k = parse_keys(keys).map_err(|f| (id, f))?;
        let placement = resolve(image, &k, k.placement_id, self.cell_size).map_err(|f| (id, f))?;
        Ok((
            id,
            Effect::Place {
                placement,
                move_cursor: !k.no_move,
            },
        ))
    }

    /// `a=d`。返事はしない（kitty も返さない）。
    pub(super) fn delete_command(&mut self, keys: &[(u8, &[u8])]) {
        let key = |k| lenient(keys, k);
        let signed_z = || {
            keys.iter()
                .rev()
                .find(|(k, _)| *k == b'z')
                .and_then(|(_, v)| signed(v))
                .unwrap_or(0)
        };
        let d = match keys.iter().rev().find(|(k, _)| *k == b'd') {
            None => b'a',
            Some((_, v)) if v.len() == 1 => v[0],
            Some(_) => return,
        };
        let free = d.is_ascii_uppercase();
        let target = match d.to_ascii_lowercase() {
            b'a' => DeleteTarget::All,
            b'i' => DeleteTarget::Image {
                id: key(b'i'),
                placement: key(b'p'),
            },
            b'n' => {
                let Some(img) = self.get_by_number(key(b'I')) else {
                    return;
                };
                DeleteTarget::Image {
                    id: img.id,
                    placement: key(b'p'),
                }
            }
            b'c' => DeleteTarget::Cursor,
            b'p' => DeleteTarget::Cell {
                x: key(b'x'),
                y: key(b'y'),
            },
            b'q' => DeleteTarget::CellZ {
                x: key(b'x'),
                y: key(b'y'),
                z: signed_z(),
            },
            b'x' => DeleteTarget::Column(key(b'x')),
            b'y' => DeleteTarget::Row(key(b'y')),
            b'z' => DeleteTarget::Z(signed_z()),
            // アニメーションのフレームは持たないので、消すものが無い。知らない値も何もしない。
            _ => return,
        };
        self.effects
            .push(Effect::Delete(DeleteSpec { target, free }));
    }
}
