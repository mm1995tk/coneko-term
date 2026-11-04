//! Unicode placeholder（U+10EEEE）の読み取り。
//!
//! 仕様の「Unicode placeholders」の節のとおり、画像の ID は前景色（256 色なら 8 ビット、
//! 24 ビット色なら 24 ビット。3 つ目の結合文字が上位 8 ビット）、配置の番号は下線の色、
//! 行と列は 1 つ目と 2 つ目の結合文字で表す。
//!
//! placeholder の字は、特別なセルの種類にせず、ふつうの字（`Content::Char` / `Content::Cluster`）
//! として持つ。画面のコピー、組み直し、消去がほかの字と同じに働くため。画像は、画面を読む
//! たびにセルから導き、[`PlacedImage`] にして返す（画面には保存しない）。

use super::diacritics::DIACRITICS;
use super::{Graphics, PlacedImage, Placement};
use crate::screen::{Cell, Color, Content};

/// placeholder の字。
pub const PLACEHOLDER: char = '\u{10EEEE}';

/// 色が表す番号。既定の色は番号を持たない。
fn color_number(c: Color) -> Option<u32> {
    match c {
        Color::Palette(n) => Some(u32::from(n)),
        Color::Rgb(r, g, b) => Some(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)),
        _ => None,
    }
}

fn diacritic_index(c: char) -> Option<u32> {
    DIACRITICS.iter().position(|&d| d == c).map(|i| i as u32)
}

/// 読み取ったセル 1 つ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Holder {
    /// 前景色の番号（上位 8 ビットを足す前）。
    fg: u32,
    /// 下線の色の番号（0 は無い）。
    underline: u32,
    row: u32,
    col: u32,
    /// 画像の ID の上位 8 ビット。
    msb: u32,
}

/// セルが placeholder なら、結合文字から取れた値（行・列・上位 8 ビット。無ければ `None`）と色。
fn read_cell(cell: &Cell) -> Option<(u32, u32, [Option<u32>; 3])> {
    let text: &str = match &cell.content {
        Content::Char(c) if *c == PLACEHOLDER => "",
        Content::Cluster(s) if s.starts_with(PLACEHOLDER) => &s[PLACEHOLDER.len_utf8()..],
        _ => return None,
    };
    let fg = color_number(cell.attrs.fg)?;
    let underline = color_number(cell.attrs.underline.color).unwrap_or(0);
    let mut marks = [None; 3];
    // 表にある字だけを数える（知らない結合文字は捨てる）。4 つ目以降も捨てる。
    for (slot, n) in marks
        .iter_mut()
        .zip(text.chars().filter_map(diacritic_index))
    {
        *slot = Some(n);
    }
    Some((fg, underline, marks))
}

/// 左のセル `left` を踏まえて、省かれた値を埋める（仕様の 3 つの規則を左から順に当てる）。
fn resolve(fg: u32, underline: u32, marks: [Option<u32>; 3], left: Option<Holder>) -> Holder {
    let same = |l: &Holder| l.fg == fg && l.underline == underline;
    let (row, col, msb) = match (marks, left) {
        // 結合文字が無い: 左のセルの行、1 つ右の列。
        ([None, ..], Some(l)) if same(&l) => (l.row, l.col + 1, l.msb),
        ([None, ..], _) => (0, 0, 0),
        // 行だけ: 左のセルと同じ行なら 1 つ右の列。
        ([Some(r), None, _], Some(l)) if same(&l) && l.row == r => (r, l.col + 1, l.msb),
        ([Some(r), None, m], _) => (r, 0, m.unwrap_or(0)),
        // 行と列: 左のセルと同じ行で列が 1 つ違うなら、上位 8 ビットを引き継ぐ。
        ([Some(r), Some(c), None], Some(l)) if same(&l) && l.row == r && l.col + 1 == c => {
            (r, c, l.msb)
        }
        ([Some(r), Some(c), m], _) => (r, c, m.unwrap_or(0)),
    };
    Holder {
        fg,
        underline,
        row,
        col,
        msb,
    }
}

/// 同じ行で続く、同じ画像・同じ配置の、列が 1 つずつ増える placeholder の連なり。
struct Run {
    x: usize,
    image_id: u32,
    placement_id: u32,
    row: u32,
    col: u32,
    len: u32,
}

/// 見えている行と、スクロールバックの行を読み、placeholder が表す画像を配置にして返す。
pub(crate) fn derive<'a>(
    graphics: &Graphics,
    rows: impl Iterator<Item = (i64, &'a [Cell])>,
) -> Vec<PlacedImage> {
    let mut out = Vec::new();
    if !graphics.has_virtuals() {
        return out;
    }
    for (row, cells) in rows {
        let mut left: Option<Holder> = None;
        let mut run: Option<Run> = None;
        for (x, cell) in cells.iter().enumerate() {
            let holder = read_cell(cell).map(|(fg, ul, marks)| resolve(fg, ul, marks, left));
            left = holder;
            let next = holder.map(|h| (h.msb << 24 | h.fg, h.underline, h.row, h.col));
            if let (Some(r), Some((image_id, placement_id, hr, hc))) = (&mut run, next) {
                if r.image_id == image_id
                    && r.placement_id == placement_id
                    && r.row == hr
                    && r.col + r.len == hc
                {
                    r.len += 1;
                    continue;
                }
            }
            if let Some(r) = run.take() {
                finish(graphics, row, &r, &mut out);
            }
            if let Some((image_id, placement_id, hr, hc)) = next {
                run = Some(Run {
                    x,
                    image_id,
                    placement_id,
                    row: hr,
                    col: hc,
                    len: 1,
                });
            }
        }
        if let Some(r) = run.take() {
            finish(graphics, row, &r, &mut out);
        }
    }
    out
}

fn div_ceil(a: u64, b: u64) -> u64 {
    a.div_ceil(b.max(1))
}

/// 連なり 1 つを、画像の該当する部分を描く配置にする。
///
/// 仮の配置の `cols` x `rows` のセルの枠に、画像（`src_*` の部分）を縦横比を保って収め、
/// 中央に置く。連なりのセルがその枠のどこにあたるかから、使う画素の部分と描く場所を決める。
fn finish(graphics: &Graphics, row: i64, run: &Run, out: &mut Vec<PlacedImage>) {
    let Some(vp) = graphics.virtual_placement(run.image_id, run.placement_id) else {
        return;
    };
    if graphics.get(run.image_id).is_none() || vp.src_w == 0 || vp.src_h == 0 {
        return;
    }
    if run.row >= vp.rows || run.col >= vp.cols {
        return;
    }
    // 枠からはみ出す列は捨てる。
    let len = run.len.min(vp.cols - run.col);
    let (cw, ch) = graphics.cell_size();
    let (cw, ch) = (u64::from(cw), u64::from(ch));
    let (grid_w, grid_h) = (u64::from(vp.cols) * cw, u64::from(vp.rows) * ch);
    let (sw, sh) = (u64::from(vp.src_w), u64::from(vp.src_h));
    // 枠に収まる大きさ（切り捨てて、枠を超えない）。
    let (iw, ih) = if grid_w * sh <= grid_h * sw {
        (grid_w, (grid_w * sh / sw).max(1))
    } else {
        ((grid_h * sw / sh).max(1), grid_h)
    };
    let (ox, oy) = ((grid_w - iw) / 2, (grid_h - ih) / 2);
    // 連なりが枠のどこか（画素）。
    let (x0, x1) = (u64::from(run.col) * cw, u64::from(run.col + len) * cw);
    let (y0, y1) = (u64::from(run.row) * ch, u64::from(run.row + 1) * ch);
    let (ix0, ix1) = (x0.max(ox), x1.min(ox + iw));
    let (iy0, iy1) = (y0.max(oy), y1.min(oy + ih));
    if ix1 <= ix0 || iy1 <= iy0 {
        return;
    }
    // 画像の中の位置（画素）。四捨五入する。
    let to_src = |d: u64, total: u64, size: u64| (d * size * 2 + total) / (total * 2);
    let sx0 = to_src(ix0 - ox, iw, sw);
    let sx1 = to_src(ix1 - ox, iw, sw).max(sx0 + 1).min(sw);
    let sy0 = to_src(iy0 - oy, ih, sh);
    let sy1 = to_src(iy1 - oy, ih, sh).max(sy0 + 1).min(sh);
    let sx0 = sx0.min(sx1 - 1);
    let sy0 = sy0.min(sy1 - 1);
    // 描く場所: 連なりの最初のセルから見たずれ。ずれがセルの幅を超えるぶんは列を進める。
    let skip = (ix0 - x0) / cw;
    let offset_x = (ix0 - x0) - skip * cw;
    let offset_y = iy0 - y0;
    let (dest_w, dest_h) = (ix1 - ix0, iy1 - iy0);
    let placement = Placement {
        image_id: run.image_id,
        placement_id: run.placement_id,
        col: run.x + skip as usize,
        src_x: vp.src_x + sx0 as u32,
        src_y: vp.src_y + sy0 as u32,
        src_w: (sx1 - sx0) as u32,
        src_h: (sy1 - sy0) as u32,
        offset_x: offset_x as u32,
        offset_y: offset_y as u32,
        cols: div_ceil(offset_x + dest_w, cw) as u32,
        rows: div_ceil(offset_y + dest_h, ch) as u32,
        z: 0,
        native_size: false,
        dest_size: Some((dest_w as u32, dest_h as u32)),
    };
    out.push(PlacedImage { row, placement });
}

#[cfg(test)]
mod tests;
