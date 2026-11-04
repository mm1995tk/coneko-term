//! 仕様の「Unicode placeholders」の例に沿ったテスト。セルの大きさは既定の 10 x 20 画素。

use crate::{graphics::PlacedImage, Terminal};

const P: char = '\u{10EEEE}';
// 仕様の例が使う結合文字: 0 は U+0305、1 は U+030D、2 は U+030E。
const D0: char = '\u{0305}';
const D1: char = '\u{030D}';
const D2: char = '\u{030E}';

fn apc(t: &mut Terminal, body: &str) {
    t.write(format!("\x1b_G{body}\x1b\\").as_bytes());
}

/// 幅 `w`、高さ `h` 画素の RGB 画像（全部 0）を `id` で送る。
fn send(t: &mut Terminal, id: u32, w: usize, h: usize) {
    let data = "A".repeat(w * h * 4);
    apc(t, &format!("a=t,f=24,s={w},v={h},i={id},q=2;{data}"));
}

fn rows(t: &Terminal) -> Vec<PlacedImage> {
    t.placements()
}

/// `(行, 列, src_x, src_y, src_w, src_h, offset_x, offset_y, 描く幅, 描く高さ)`
type Summary = (i64, usize, u32, u32, u32, u32, u32, u32, u32, u32);

fn summary(t: &Terminal) -> Vec<Summary> {
    rows(t)
        .iter()
        .map(|p| {
            let q = &p.placement;
            let (dw, dh) = q.dest_size.unwrap();
            (
                p.row, q.col, q.src_x, q.src_y, q.src_w, q.src_h, q.offset_x, q.offset_y, dw, dh,
            )
        })
        .collect()
}

#[test]
fn spec_example_2x2() {
    let mut t = Terminal::new(5, 20);
    // 2 列 x 2 行 = 20 x 40 画素。同じ縦横比の画像なら、枠いっぱいに伸ばす。
    send(&mut t, 42, 20, 40);
    apc(&mut t, "a=p,U=1,i=42,c=2,r=2,q=2");
    t.write(format!("\x1b[38;5;42m{P}{D0}{D0}{P}{D0}{D1}\r\n{P}{D1}{D0}{P}{D1}{D1}").as_bytes());
    assert_eq!(
        summary(&t),
        [
            (0, 0, 0, 0, 20, 20, 0, 0, 20, 20),
            (1, 0, 0, 20, 20, 20, 0, 0, 20, 20),
        ]
    );
    let first = &rows(&t)[0].placement;
    assert_eq!((first.cols, first.rows, first.image_id), (2, 1, 42));
    // 画面には置かない仮の配置なので、カーソルは文字の分だけ動く（画像の大きさでは動かない）。
    assert_eq!(cursor(&t), (2, 1));
}

#[test]
fn virtual_placement_alone_draws_nothing_and_keeps_cursor() {
    let mut t = Terminal::new(5, 20);
    send(&mut t, 42, 20, 40);
    apc(&mut t, "a=p,U=1,i=42,c=2,r=2,q=2");
    assert!(rows(&t).is_empty());
    assert_eq!(cursor(&t), (0, 0));
    // 仮の配置はあるが、placeholder の字が無ければ何も無い。
    t.write(b"hello");
    assert!(rows(&t).is_empty());
}

#[test]
fn transmit_and_place_in_one_command() {
    let mut t = Terminal::new(5, 20);
    let data = "A".repeat(20 * 40 * 4);
    apc(
        &mut t,
        &format!("a=T,U=1,f=24,s=20,v=40,i=42,c=2,r=2,q=2;{data}"),
    );
    t.write(format!("\x1b[38;5;42m{P}{D0}{D0}{P}{D0}{D1}").as_bytes());
    assert_eq!(summary(&t), [(0, 0, 0, 0, 20, 20, 0, 0, 20, 20)]);
    assert_eq!(cursor(&t), (2, 0));
}

#[test]
fn omitted_diacritics_follow_the_left_cell() {
    let mut t = Terminal::new(5, 20);
    // 3 列 x 2 行 = 30 x 40 画素。
    send(&mut t, 42, 30, 40);
    apc(&mut t, "a=p,U=1,i=42,c=3,r=2,q=2");
    // 仕様の例: 1 行目は先頭だけ行と列、2 行目は先頭だけ行（列は 0）で、残りは省く。
    t.write(format!("\x1b[38;5;42m{P}{D0}{D0}{P}{P}\r\n{P}{D1}{P}{P}").as_bytes());
    assert_eq!(
        summary(&t),
        [
            (0, 0, 0, 0, 30, 20, 0, 0, 30, 20),
            (1, 0, 0, 20, 30, 20, 0, 0, 30, 20),
        ]
    );
}

#[test]
fn row_only_with_a_different_row_restarts_at_column_0() {
    let mut t = Terminal::new(5, 20);
    send(&mut t, 42, 30, 40);
    apc(&mut t, "a=p,U=1,i=42,c=3,r=2,q=2");
    // 左のセルと行が違う（行だけ）ときは、列は 0 から。
    t.write(format!("\x1b[38;5;42m{P}{D0}{D1}{P}{D1}").as_bytes());
    assert_eq!(
        summary(&t),
        [
            (0, 0, 10, 0, 10, 20, 0, 0, 10, 20),
            (0, 1, 0, 20, 10, 20, 0, 0, 10, 20),
        ]
    );
}

#[test]
fn first_cell_without_diacritics_is_row_0_column_0() {
    let mut t = Terminal::new(5, 20);
    send(&mut t, 42, 30, 40);
    apc(&mut t, "a=p,U=1,i=42,c=3,r=2,q=2");
    t.write(format!("\x1b[38;5;42m{P}{P}").as_bytes());
    assert_eq!(summary(&t), [(0, 0, 0, 0, 20, 20, 0, 0, 20, 20)]);
}

#[test]
fn colors_must_match_to_follow_the_left_cell() {
    let mut t = Terminal::new(5, 20);
    send(&mut t, 42, 30, 40);
    apc(&mut t, "a=p,U=1,i=42,c=3,r=2,q=2");
    // 2 つ目は前景色が違う: 左の続きではないので、色が表す画像（41）の行 0 列 0。画像が無いので何も描かない。
    t.write(format!("\x1b[38;5;42m{P}{D0}{D0}\x1b[38;5;41m{P}").as_bytes());
    assert_eq!(summary(&t), [(0, 0, 0, 0, 10, 20, 0, 0, 10, 20)]);
}

#[test]
fn image_id_high_byte_from_the_third_diacritic() {
    let mut t = Terminal::new(5, 20);
    let id = 42 + (2 << 24);
    send(&mut t, id, 20, 40);
    apc(&mut t, &format!("a=p,U=1,i={id},c=2,r=2,q=2"));
    // 24 ビット色の前景で下位 24 ビット、3 つ目の結合文字で上位 8 ビット（仕様の 33554474）。
    // 2 つ目のセルは省いた上位 8 ビットを左から引き継ぐ。
    t.write(format!("\x1b[38;2;0;0;42m{P}{D0}{D0}{D2}{P}{D0}{D1}").as_bytes());
    let got = rows(&t);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].placement.image_id, id);
    assert_eq!(got[0].placement.cols, 2);
}

#[test]
fn underline_color_picks_the_placement() {
    let mut t = Terminal::new(5, 20);
    send(&mut t, 1, 20, 20);
    apc(&mut t, "a=p,U=1,i=1,p=7,c=1,r=1,q=2");
    apc(&mut t, "a=p,U=1,i=1,p=8,c=2,r=1,q=2");
    t.write(format!("\x1b[38;5;1m\x1b[58;5;8m{P}{D0}{D0}{P}{D0}{D1}").as_bytes());
    let got = rows(&t);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].placement.placement_id, 8);
    assert_eq!(got[0].placement.cols, 2);
    // 番号が 0（下線の色が無い）なら、その画像のどの仮の配置でもよい（新しいもの）。
    let mut t = Terminal::new(5, 20);
    send(&mut t, 1, 20, 20);
    apc(&mut t, "a=p,U=1,i=1,p=7,c=1,r=1,q=2");
    t.write(format!("\x1b[38;5;1m{P}{D0}{D0}").as_bytes());
    assert_eq!(rows(&t).len(), 1);
    // 番号が合わなければ描かない。
    let mut t = Terminal::new(5, 20);
    send(&mut t, 1, 20, 20);
    apc(&mut t, "a=p,U=1,i=1,p=7,c=1,r=1,q=2");
    t.write(format!("\x1b[38;5;1m\x1b[58;5;9m{P}{D0}{D0}").as_bytes());
    assert!(rows(&t).is_empty());
}

#[test]
fn image_is_fitted_and_centered_in_the_grid() {
    let mut t = Terminal::new(5, 20);
    // 2 x 2 セル = 20 x 40 画素の枠に正方形の画像: 20 x 20 で、上下に 10 画素ずつの余白。
    send(&mut t, 1, 4, 4);
    apc(&mut t, "a=p,U=1,i=1,c=2,r=2,q=2");
    t.write(format!("\x1b[38;5;1m{P}{D0}{D0}{P}{D0}{D1}\r\n{P}{D1}{D0}{P}{D1}{D1}").as_bytes());
    // 1 行目は下の半分 10 画素（画像の上半分 = 2 画素ぶん）、2 行目は上から 10 画素の高さ。
    assert_eq!(
        summary(&t),
        [
            (0, 0, 0, 0, 4, 2, 0, 10, 20, 10),
            (1, 0, 0, 2, 4, 2, 0, 0, 20, 10),
        ]
    );
}

#[test]
fn rows_outside_the_grid_and_missing_images_are_dropped() {
    let mut t = Terminal::new(5, 20);
    send(&mut t, 1, 10, 20);
    apc(&mut t, "a=p,U=1,i=1,c=1,r=1,q=2");
    // 行 1 と列 1 は枠の外。画像 2 は無い。
    t.write(format!("\x1b[38;5;1m{P}{D1}{D0}{P}{D0}{D1}\x1b[38;5;2m{P}{D0}{D0}").as_bytes());
    assert!(rows(&t).is_empty());
}

#[test]
fn deleting_by_id_removes_the_virtual_placement_but_a_deletes_do_not() {
    let mut t = Terminal::new(5, 20);
    send(&mut t, 1, 10, 20);
    apc(&mut t, "a=p,U=1,i=1,c=1,r=1,q=2");
    t.write(format!("\x1b[38;5;1m{P}{D0}{D0}").as_bytes());
    assert_eq!(rows(&t).len(), 1);
    apc(&mut t, "a=d,d=a");
    apc(&mut t, "a=d,d=z");
    assert_eq!(rows(&t).len(), 1);
    apc(&mut t, "a=d,d=i,i=1");
    assert!(rows(&t).is_empty());
}

#[test]
fn replacing_the_image_drops_its_virtual_placements() {
    let mut t = Terminal::new(5, 20);
    send(&mut t, 1, 10, 20);
    apc(&mut t, "a=p,U=1,i=1,c=1,r=1,q=2");
    t.write(format!("\x1b[38;5;1m{P}{D0}{D0}").as_bytes());
    send(&mut t, 1, 10, 20);
    assert!(rows(&t).is_empty());
}

#[test]
fn placeholders_move_with_the_text() {
    let mut t = Terminal::new(3, 20);
    send(&mut t, 1, 10, 20);
    apc(&mut t, "a=p,U=1,i=1,c=1,r=1,q=2");
    t.write(format!("\x1b[38;5;1m{P}{D0}{D0}").as_bytes());
    t.write(b"\x1b[0m\r\n\r\n\r\n\r\n");
    // 1 行ぶんスクロールバックに入る: 行は -2。
    assert_eq!(summary(&t)[0].0, -2);
}

#[test]
fn unknown_combining_marks_are_ignored_and_the_cell_stays_text() {
    let mut t = Terminal::new(5, 20);
    send(&mut t, 1, 10, 20);
    apc(&mut t, "a=p,U=1,i=1,c=1,r=1,q=2");
    t.write(format!("\x1b[38;5;1m{P}\u{0301}{D0}{D0}").as_bytes());
    assert_eq!(summary(&t).len(), 1);
    // 字として持つ。
    assert!(t.screen().row(0).cells()[0].text().starts_with(P));
}

fn cursor(t: &Terminal) -> (usize, usize) {
    let c = t.screen().cursor();
    (c.x, c.y)
}
