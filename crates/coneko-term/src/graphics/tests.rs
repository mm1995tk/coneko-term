use super::*;
use crate::screen::Screen;
use crate::Terminal;

fn apc(t: &mut Terminal, body: &str) {
    t.write(format!("\x1b_G{body}\x1b\\").as_bytes());
}

fn reply(t: &mut Terminal) -> String {
    String::from_utf8(t.take_output()).unwrap()
}

fn ok(id: &str) -> String {
    format!("\x1b_G{id};OK\x1b\\")
}

// 仕様の例: 1x2 の RGB（"AAAA..." は 0 が 6 バイト）。
const RGB_1X2: &str = "AAAAAAAA";

#[test]
fn transmit_rgb_and_lookup() {
    let mut t = Terminal::new(5, 20);
    apc(&mut t, &format!("a=t,f=24,s=1,v=2,i=31;{RGB_1X2}"));
    assert_eq!(reply(&mut t), ok("i=31"));
    let img = t.image(31).unwrap();
    assert_eq!(img.format, ImageFormat::Rgb);
    assert_eq!((img.width, img.height), (1, 2));
    assert_eq!(img.data, vec![0; 6]);
    assert!(!img.compressed);
}

#[test]
fn default_action_and_format() {
    let mut t = Terminal::new(5, 20);
    // f 既定は 32、a 既定は t。
    apc(&mut t, "s=1,v=1,i=1;/wAA/w==");
    assert_eq!(reply(&mut t), ok("i=1"));
    let img = t.image(1).unwrap();
    assert_eq!(img.format, ImageFormat::Rgba);
    assert_eq!(img.data, vec![0xff, 0, 0, 0xff]);
}

#[test]
fn query_does_not_store() {
    let mut t = Terminal::new(5, 20);
    apc(&mut t, &format!("i=31,s=1,v=2,t=d,f=24,a=q;{RGB_1X2}"));
    assert_eq!(reply(&mut t), ok("i=31"));
    assert!(t.image(31).is_none());
    assert_eq!(t.images().count(), 0);
}

#[test]
fn chunked_transmission() {
    let mut t = Terminal::new(5, 20);
    apc(&mut t, "a=t,f=24,s=1,v=2,i=7,m=1;AAAA");
    assert_eq!(reply(&mut t), "");
    assert!(t.image(7).is_none());
    apc(&mut t, "m=1;AAA");
    apc(&mut t, "m=0;A");
    assert_eq!(reply(&mut t), ok("i=7"));
    assert_eq!(t.image(7).unwrap().data, vec![0; 6]);
}

#[test]
fn no_reply_without_id() {
    let mut t = Terminal::new(5, 20);
    apc(&mut t, &format!("a=t,f=24,s=1,v=2;{RGB_1X2}"));
    assert_eq!(reply(&mut t), "");
    assert_eq!(t.images().count(), 1);
    apc(&mut t, "a=t,f=24,s=1,v=2;AAAA");
    assert_eq!(reply(&mut t), "");
}

#[test]
fn image_number_assigns_id() {
    let mut t = Terminal::new(5, 20);
    apc(&mut t, &format!("f=24,s=1,v=2,I=13;{RGB_1X2}"));
    let r = reply(&mut t);
    let first = t.image_by_number(13).unwrap().id;
    assert_eq!(r, format!("\x1b_Gi={first},I=13;OK\x1b\\"));
    assert!(t.image(first).is_some());
    // 同じ番号でもう一度送ると、新しい画像が番号で引ける。
    apc(&mut t, &format!("f=24,s=1,v=2,I=13;{RGB_1X2}"));
    let second = t.image_by_number(13).unwrap().id;
    assert_ne!(second, first);
}

#[test]
fn i_and_big_i_together_is_error() {
    let mut t = Terminal::new(5, 20);
    apc(&mut t, &format!("f=24,s=1,v=2,i=1,I=2;{RGB_1X2}"));
    assert!(reply(&mut t).starts_with("\x1b_Gi=1,I=2;EINVAL:"));
    assert_eq!(t.images().count(), 0);
}

#[test]
fn quiet_levels() {
    let mut t = Terminal::new(5, 20);
    apc(&mut t, &format!("f=24,s=1,v=2,i=1,q=1;{RGB_1X2}"));
    assert_eq!(reply(&mut t), "");
    assert!(t.image(1).is_some());
    // q=1 でもエラーは返す。
    apc(&mut t, "f=24,s=1,v=2,i=2,q=1;AAAA");
    assert!(reply(&mut t).starts_with("\x1b_Gi=2;EINVAL:"));
    // q=2 はエラーも返さない。
    apc(&mut t, "f=24,s=1,v=2,i=3,q=2;AAAA");
    assert_eq!(reply(&mut t), "");
    assert!(t.image(3).is_none());
}

#[test]
fn unsupported_transmission_media() {
    for m in ["f", "t", "s"] {
        let mut t = Terminal::new(5, 20);
        apc(&mut t, &format!("a=t,t={m},f=100,i=5;L3RtcC9hLnBuZw=="));
        assert!(reply(&mut t).starts_with("\x1b_Gi=5;EBADF:"), "t={m}");
        assert!(t.image(5).is_none());
    }
    // 分割されていても、続きは読み捨てる。
    let mut t = Terminal::new(5, 20);
    apc(&mut t, "t=f,i=5,m=1;AAAA");
    assert!(reply(&mut t).starts_with("\x1b_Gi=5;EBADF:"));
    apc(&mut t, "m=0;AAAA");
    assert_eq!(reply(&mut t), "");
    assert_eq!(t.images().count(), 0);
}

#[test]
fn malformed_data() {
    let mut t = Terminal::new(5, 20);
    // base64 でない。
    apc(&mut t, "f=24,s=1,v=2,i=1;!!!!");
    assert!(reply(&mut t).starts_with("\x1b_Gi=1;EINVAL:"));
    // 長さが s・v と合わない。
    apc(&mut t, "f=24,s=2,v=2,i=2;AAAAAAAA");
    assert!(reply(&mut t).starts_with("\x1b_Gi=2;EINVAL:"));
    // s・v が無い。
    apc(&mut t, "f=32,i=3;AAAAAAAA");
    assert!(reply(&mut t).starts_with("\x1b_Gi=3;EINVAL:"));
    // データが無い。
    apc(&mut t, "f=24,s=1,v=1,i=4");
    assert!(reply(&mut t).starts_with("\x1b_Gi=4;ENODATA:"));
    // 知らない形式・圧縮・数でない値。
    apc(&mut t, "f=99,s=1,v=1,i=5;AAAA");
    assert!(reply(&mut t).starts_with("\x1b_Gi=5;EINVAL:"));
    apc(&mut t, "f=24,s=1,v=1,o=x,i=6;AAAA");
    assert!(reply(&mut t).starts_with("\x1b_Gi=6;EINVAL:"));
    apc(&mut t, "f=24,s=abc,v=1,i=7;AAAA");
    assert!(reply(&mut t).starts_with("\x1b_Gi=7;EINVAL:"));
    // 大きすぎる寸法。
    apc(&mut t, "f=24,s=10001,v=1,o=z,i=8;AAAA");
    assert!(reply(&mut t).starts_with("\x1b_Gi=8;EINVAL:"));
    assert_eq!(t.images().count(), 0);
}

fn png_1x1() -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
    v.extend_from_slice(&[0, 0, 0, 13]);
    v.extend_from_slice(b"IHDR");
    v.extend_from_slice(&3u32.to_be_bytes());
    v.extend_from_slice(&5u32.to_be_bytes());
    v.extend_from_slice(&[8, 6, 0, 0, 0]);
    v
}

fn b64(data: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

#[test]
fn png_is_kept_as_is_with_ihdr_size() {
    let mut t = Terminal::new(5, 20);
    let png = png_1x1();
    apc(&mut t, &format!("f=100,i=9;{}", b64(&png)));
    assert_eq!(reply(&mut t), ok("i=9"));
    let img = t.image(9).unwrap();
    assert_eq!(img.format, ImageFormat::Png);
    assert_eq!((img.width, img.height), (3, 5));
    assert_eq!(img.data, png);
    // PNG でないデータ。
    apc(&mut t, "f=100,i=10;AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    assert!(reply(&mut t).starts_with("\x1b_Gi=10;EINVAL:"));
}

#[test]
fn compressed_data_is_kept_as_is() {
    let mut t = Terminal::new(5, 20);
    // 圧縮されたデータは、長さを確かめず、読み解かない。
    apc(&mut t, "f=24,s=10,v=10,o=z,i=1;eJxjYAAAAAIAAQ==");
    assert_eq!(reply(&mut t), ok("i=1"));
    let img = t.image(1).unwrap();
    assert!(img.compressed);
    assert_eq!((img.width, img.height), (10, 10));
    assert_eq!(img.data[..2], [0x78, 0x9c]);
    // 圧縮された PNG は、送られた s・v をそのまま持つ。
    apc(&mut t, "f=100,o=z,s=4,v=6,i=2;eJxjYAAAAAIAAQ==");
    assert_eq!(reply(&mut t), ok("i=2"));
    assert_eq!(
        (t.image(2).unwrap().width, t.image(2).unwrap().height),
        (4, 6)
    );
}

#[test]
fn resend_replaces_image() {
    let mut t = Terminal::new(5, 20);
    apc(&mut t, "f=24,s=1,v=1,i=1;AAAA");
    apc(&mut t, "f=24,s=1,v=1,i=1;/wAA");
    t.take_output();
    assert_eq!(t.images().count(), 1);
    assert_eq!(t.image(1).unwrap().data, vec![0xff, 0, 0]);
    assert_eq!(t.image_bytes(), 3);
}

#[test]
fn quota_evicts_oldest() {
    let mut t = Terminal::new(5, 20);
    t.set_image_quota(7); // 3 バイトの画像が 2 つ入る。
    for id in 1..=3 {
        apc(&mut t, &format!("f=24,s=1,v=1,i={id};AAAA"));
    }
    t.take_output();
    assert!(t.image(1).is_none());
    assert!(t.image(2).is_some());
    assert!(t.image(3).is_some());
    assert_eq!(t.image_bytes(), 6);
    // 上限を下げると古いものから捨てる。
    t.set_image_quota(3);
    assert!(t.image(2).is_none());
    assert!(t.image(3).is_some());
}

#[test]
fn image_larger_than_quota_is_efbig() {
    let mut t = Terminal::new(5, 20);
    t.set_image_quota(2);
    apc(&mut t, "f=24,s=1,v=1,i=1;AAAA");
    assert!(reply(&mut t).starts_with("\x1b_Gi=1;EFBIG:"));
    assert_eq!(t.images().count(), 0);
    // 分割送信で上限を超えたら、続きは読み捨てる。
    apc(&mut t, "f=24,s=1,v=1,i=2,m=1;AAAAAAAA");
    assert!(reply(&mut t).starts_with("\x1b_Gi=2;EFBIG:"));
    apc(&mut t, "m=0;AAAA");
    assert_eq!(reply(&mut t), "");
}

#[test]
fn unsupported_actions_are_dropped() {
    let mut t = Terminal::new(5, 20);
    for a in ["x", "z"] {
        apc(&mut t, &format!("a={a},f=24,s=1,v=1,i=1;AAAA"));
        assert_eq!(reply(&mut t), "", "a={a}");
    }
    assert_eq!(t.images().count(), 0);
    // 分割された知らない動作の続きも、画像にならない。
    apc(&mut t, "a=x,f=24,s=1,v=1,i=1,m=1;AAAA");
    apc(&mut t, "m=0;");
    assert_eq!(t.images().count(), 0);
    assert_eq!(reply(&mut t), "");
}

#[test]
fn other_apc_and_garbage_are_ignored() {
    let mut t = Terminal::new(5, 20);
    t.write(b"\x1b_hello\x1b\\\x1b_G\x1b\\\x1b_G,,=,;;;\x1b\\\x1b_Gzzz\x1b\\");
    assert_eq!(reply(&mut t), "");
    assert_eq!(t.images().count(), 0);
    t.write(b"ok");
    assert_eq!(t.screen().row(0).cells()[0].text(), "o");
}

#[test]
fn reset_clears_images() {
    let mut t = Terminal::new(5, 20);
    apc(&mut t, "f=24,s=1,v=1,i=1;AAAA");
    t.write(b"\x1bc");
    t.take_output();
    assert_eq!(t.images().count(), 0);
    assert_eq!(t.image_bytes(), 0);
}

#[test]
fn missing_padding_is_accepted() {
    let mut t = Terminal::new(5, 20);
    apc(&mut t, "f=24,s=1,v=1,i=1;AAAA");
    apc(&mut t, "f=32,s=1,v=1,i=2;/wAA/w");
    assert_eq!(t.image(2).unwrap().data, vec![0xff, 0, 0, 0xff]);
}

// ---- 置く・消す ----

/// 20x40 の RGB（セルが 10x20 画素なら 2 列 2 行）。
fn send_big(t: &mut Terminal, extra: &str) {
    apc(t, &format!("f=24,s=20,v=40,{extra};{}", "A".repeat(3200)));
}

fn spots(t: &Terminal) -> Vec<(i64, usize, u32, u32)> {
    t.placements()
        .iter()
        .map(|p| {
            (
                p.row,
                p.placement.col,
                p.placement.image_id,
                p.placement.placement_id,
            )
        })
        .collect()
}

fn cursor(t: &Terminal) -> (usize, usize) {
    let c = t.screen().cursor();
    (c.x, c.y)
}

#[test]
fn transmit_and_place_moves_cursor() {
    let mut t = Terminal::new(5, 20);
    t.write(b"\x1b[2;3H");
    send_big(&mut t, "a=T,i=1");
    assert_eq!(reply(&mut t), ok("i=1"));
    let ps = t.placements();
    assert_eq!(ps.len(), 1);
    let p = &ps[0].placement;
    assert_eq!((ps[0].row, p.col, p.image_id), (1, 2, 1));
    assert_eq!((p.cols, p.rows, p.z, p.placement_id), (2, 2, 0, 0));
    assert_eq!((p.src_x, p.src_y, p.src_w, p.src_h), (0, 0, 20, 40));
    // 右へ c 列、下へ r - 1 行
    assert_eq!(cursor(&t), (4, 2));
    assert!(t.image(1).is_some());
}

#[test]
fn cursor_stays_with_c1() {
    let mut t = Terminal::new(5, 20);
    send_big(&mut t, "a=T,i=1,C=1");
    assert_eq!(cursor(&t), (0, 0));
    assert_eq!(t.placements().len(), 1);
}

#[test]
fn cursor_moving_scrolls_at_the_bottom() {
    let mut t = Terminal::new(3, 20);
    t.write(b"\x1b[3;1H");
    send_big(&mut t, "a=T,i=1");
    // 置いた行は 1 行上へ押し上がる
    assert_eq!(spots(&t), [(1, 0, 1, 0)]);
    assert_eq!(cursor(&t), (2, 2));
}

#[test]
fn place_existing_image_with_keys() {
    let mut t = Terminal::new(10, 40);
    send_big(&mut t, "a=t,i=7");
    assert!(t.placements().is_empty());
    t.write(b"\x1b[4;5H");
    reply(&mut t);
    apc(
        &mut t,
        "a=p,i=7,p=3,x=2,y=4,w=100,h=10,X=3,Y=50,c=6,r=2,z=-5,C=1",
    );
    assert_eq!(reply(&mut t), ok("i=7,p=3"));
    let p = &t.placements()[0].placement;
    assert_eq!((t.placements()[0].row, p.col), (3, 4));
    // 画像の外にはみ出す分は切る。X・Y はセルの大きさより小さくする
    assert_eq!((p.src_x, p.src_y, p.src_w, p.src_h), (2, 4, 18, 10));
    assert_eq!((p.offset_x, p.offset_y), (3, 19));
    assert_eq!((p.cols, p.rows, p.z, p.placement_id), (6, 2, -5, 3));
    assert_eq!(cursor(&t), (4, 3));
}

#[test]
fn size_follows_the_cell_size_and_aspect_ratio() {
    let mut t = Terminal::new(10, 40);
    t.set_cell_size(8, 16);
    send_big(&mut t, "a=t,i=1");
    apc(&mut t, "a=p,i=1,C=1");
    apc(&mut t, "a=p,i=1,C=1,X=5,Y=2");
    // c だけ: 幅 4 セル = 32 画素なので高さは 64 画素 = 4 行
    apc(&mut t, "a=p,i=1,C=1,c=4");
    // r だけ: 高さ 3 行 = 48 画素なので幅は 24 画素 = 3 列
    apc(&mut t, "a=p,i=1,C=1,r=3");
    let sizes: Vec<_> = t
        .placements()
        .iter()
        .map(|p| (p.placement.cols, p.placement.rows))
        .collect();
    assert_eq!(sizes, [(3, 3), (4, 3), (4, 4), (3, 3)]);
}

#[test]
fn place_by_number_and_errors() {
    let mut t = Terminal::new(5, 20);
    apc(&mut t, "a=p,i=9,q=0");
    assert_eq!(reply(&mut t), "\x1b_Gi=9;ENOENT:image not found\x1b\\");
    send_big(&mut t, "a=t,I=5,i=0");
    reply(&mut t);
    apc(&mut t, "a=p,I=5");
    let out = reply(&mut t);
    assert!(out.contains("I=5") && out.ends_with(";OK\x1b\\"), "{out:?}");
    assert_eq!(t.placements().len(), 1);
    // x が画像の外
    apc(&mut t, "a=p,I=5,x=20");
    assert!(reply(&mut t).contains("EINVAL"));
    assert_eq!(t.placements().len(), 1);
}

#[test]
fn same_placement_id_replaces() {
    let mut t = Terminal::new(5, 20);
    send_big(&mut t, "a=t,i=1");
    apc(&mut t, "a=p,i=1,p=2,C=1");
    t.write(b"\x1b[3;3H");
    apc(&mut t, "a=p,i=1,p=2,C=1");
    apc(&mut t, "a=p,i=1,C=1");
    apc(&mut t, "a=p,i=1,C=1");
    assert_eq!(spots(&t), [(2, 2, 1, 2), (2, 2, 1, 0), (2, 2, 1, 0)]);
}

#[test]
fn resending_an_image_drops_its_placements() {
    let mut t = Terminal::new(5, 20);
    send_big(&mut t, "a=T,i=1,C=1");
    send_big(&mut t, "a=T,i=2,C=1");
    send_big(&mut t, "a=t,i=1");
    assert_eq!(spots(&t), [(0, 0, 2, 0)]);
    // 上限を超えて捨てた画像の配置も消える
    t.set_image_quota(2400);
    assert_eq!(t.placements().len(), 0);
}

#[test]
fn placements_scroll_with_the_text() {
    let mut t = Terminal::new(4, 20);
    send_big(&mut t, "a=T,i=1,C=1");
    t.write(b"\x1b[4;1H\n\n");
    // 2 行上へ。スクロールバックの中は負の行
    assert_eq!(spots(&t), [(-2, 0, 1, 0)]);
    assert_eq!(t.screen().scrollback_len(), 2);
    // スクロールバックから出ると消える
    let mut s = Screen::with_scrollback(4, 20, 1);
    s.print('x');
    let mut t = Terminal::with_screen(s);
    send_big(&mut t, "a=T,i=1,C=1");
    t.write(b"\x1b[4;1H\n\n");
    assert!(t.placements().is_empty());
    assert!(t.image(1).is_some());
}

#[test]
fn insert_and_delete_lines_move_placements() {
    let mut t = Terminal::new(6, 20);
    t.write(b"\x1b[3;1H");
    send_big(&mut t, "a=T,i=1,C=1");
    t.write(b"\x1b[1;1H\x1b[2L");
    assert_eq!(spots(&t), [(4, 0, 1, 0)]);
    t.write(b"\x1b[1;1H\x1b[3M");
    assert_eq!(spots(&t), [(1, 0, 1, 0)]);
    t.write(b"\x1b[2;1H\x1b[M");
    assert!(t.placements().is_empty());
}

#[test]
fn clearing_the_screen_removes_visible_placements() {
    let mut t = Terminal::new(4, 20);
    send_big(&mut t, "a=T,i=1,C=1");
    // ED 0・EL は消さない
    t.write(b"\x1b[1;1H\x1b[K\x1b[J");
    assert_eq!(t.placements().len(), 1);
    t.write(b"\x1b[2J");
    assert!(t.placements().is_empty());
    assert!(t.image(1).is_some());
}

#[test]
fn clearing_scrollback_removes_placements_in_it() {
    let mut t = Terminal::new(2, 20);
    send_big(&mut t, "a=T,i=1,C=1");
    t.write(b"\x1b[2;1H\n\n");
    assert_eq!(spots(&t), [(-2, 0, 1, 0)]);
    t.write(b"\x1b[3J");
    assert!(t.placements().is_empty());
}

#[test]
fn alternate_screen_has_its_own_placements() {
    let mut t = Terminal::new(5, 20);
    send_big(&mut t, "a=T,i=1,C=1");
    t.write(b"\x1b[?1049h");
    assert!(t.placements().is_empty());
    send_big(&mut t, "a=T,i=2,C=1");
    assert_eq!(spots(&t), [(0, 0, 2, 0)]);
    t.write(b"\x1b[?1049l");
    assert_eq!(spots(&t), [(0, 0, 1, 0)]);
    t.write(b"\x1b[?1049h");
    assert!(t.placements().is_empty());
}

#[test]
fn full_reset_removes_everything() {
    let mut t = Terminal::new(5, 20);
    send_big(&mut t, "a=T,i=1,C=1");
    t.write(b"\x1bc");
    assert!(t.placements().is_empty());
    assert!(t.image(1).is_none());
}

/// 3 つの配置: 画像 1（z=0, 0 列目 1 行目）、画像 2（z=1, 5 列目 3 行目）、画像 3（z=1, 5 列目 6 行目）。
fn three(t: &mut Terminal) {
    for (id, pos, z) in [(1, "1;1", 0), (2, "3;6", 1), (3, "6;6", 1)] {
        send_big(t, &format!("a=t,i={id}"));
        t.write(format!("\x1b[{pos}H").as_bytes());
        apc(t, &format!("a=p,i={id},z={z},C=1"));
    }
    reply(t);
}

fn ids(t: &Terminal) -> Vec<u32> {
    t.placements()
        .iter()
        .map(|p| p.placement.image_id)
        .collect()
}

#[test]
fn delete_all_visible() {
    let mut t = Terminal::new(8, 20);
    three(&mut t);
    apc(&mut t, "a=d,d=a");
    assert!(t.placements().is_empty());
    assert_eq!(t.images().count(), 3);
    // d の既定は a
    three(&mut t);
    apc(&mut t, "a=d");
    assert!(t.placements().is_empty());
    // 大文字は画像のデータも捨てる
    three(&mut t);
    apc(&mut t, "a=d,d=A");
    assert_eq!(t.images().count(), 0);
}

#[test]
fn delete_by_id_and_number() {
    let mut t = Terminal::new(8, 20);
    three(&mut t);
    apc(&mut t, "a=d,d=i,i=2");
    assert_eq!(ids(&t), [1, 3]);
    assert!(t.image(2).is_some());
    apc(&mut t, "a=d,d=I,i=3");
    assert_eq!(ids(&t), [1]);
    assert!(t.image(3).is_none());
    // p を指定すると、その配置だけ
    t.write(b"\x1b[1;1H");
    apc(&mut t, "a=p,i=1,p=4,C=1");
    apc(&mut t, "a=d,d=i,i=1,p=4");
    assert_eq!(spots(&t), [(0, 0, 1, 0)]);
    // 配置の残る画像は、大文字でもデータを捨てない
    apc(&mut t, "a=p,i=1,p=5,C=1");
    apc(&mut t, "a=d,d=I,i=1,p=5");
    assert!(t.image(1).is_some());
    // 番号
    send_big(&mut t, "a=t,I=8,i=0");
    reply(&mut t);
    apc(&mut t, "a=p,I=8,C=1");
    reply(&mut t);
    assert_eq!(t.placements().len(), 2);
    apc(&mut t, "a=d,d=n,I=8");
    assert_eq!(t.placements().len(), 1);
    assert_eq!(t.images().count(), 3);
    apc(&mut t, "a=d,d=N,I=8");
    assert_eq!(t.images().count(), 2);
}

#[test]
fn delete_at_cursor_and_cell() {
    let mut t = Terminal::new(8, 20);
    three(&mut t);
    // 画像 1 は 0..2 列 × 0..2 行。カーソルをその中へ
    t.write(b"\x1b[2;2H");
    apc(&mut t, "a=d,d=c");
    assert_eq!(ids(&t), [2, 3]);
    // 画像 2 は 5..7 列 × 2..4 行。x・y は 1 始まり
    apc(&mut t, "a=d,d=p,x=6,y=3");
    assert_eq!(ids(&t), [3]);
    // 外れたセルでは消えない
    apc(&mut t, "a=d,d=p,x=8,y=6");
    apc(&mut t, "a=d,d=p,x=6,y=8");
    assert_eq!(ids(&t), [3]);
    apc(&mut t, "a=d,d=P,x=7,y=7");
    assert!(t.placements().is_empty());
    assert_eq!(t.images().count(), 2);
}

#[test]
fn delete_by_cell_and_z() {
    let mut t = Terminal::new(8, 20);
    three(&mut t);
    apc(&mut t, "a=d,d=q,x=6,y=3,z=0");
    assert_eq!(ids(&t), [1, 2, 3]);
    apc(&mut t, "a=d,d=q,x=6,y=3,z=1");
    assert_eq!(ids(&t), [1, 3]);
    apc(&mut t, "a=d,d=z,z=1");
    assert_eq!(ids(&t), [1]);
    apc(&mut t, "a=d,d=Z,z=0");
    assert!(t.placements().is_empty());
    assert!(t.image(1).is_none());
    assert!(t.image(2).is_some());
}

#[test]
fn delete_by_column_and_row() {
    let mut t = Terminal::new(8, 20);
    three(&mut t);
    apc(&mut t, "a=d,d=x,x=7");
    assert_eq!(ids(&t), [1]);
    three(&mut t);
    apc(&mut t, "a=d,d=y,y=4");
    assert_eq!(ids(&t), [1, 3]);
    apc(&mut t, "a=d,d=Y,y=1");
    assert_eq!(ids(&t), [3]);
}

#[test]
fn delete_only_touches_visible_placements() {
    let mut t = Terminal::new(3, 20);
    send_big(&mut t, "a=T,i=1,C=1");
    t.write(b"\x1b[3;1H\n\n\n");
    assert_eq!(spots(&t), [(-3, 0, 1, 0)]);
    apc(&mut t, "a=d,d=a");
    assert_eq!(t.placements().len(), 1);
    // 画像の ID で消すときは、スクロールバックの中のものも消える
    apc(&mut t, "a=d,d=i,i=1");
    assert!(t.placements().is_empty());
}

#[test]
fn unknown_and_frame_deletes_do_nothing() {
    let mut t = Terminal::new(8, 20);
    three(&mut t);
    apc(&mut t, "a=d,d=f");
    apc(&mut t, "a=d,d=F");
    apc(&mut t, "a=d,d=!");
    apc(&mut t, "a=d,d=ab");
    apc(&mut t, "a=d,d=n,I=99");
    assert_eq!(t.placements().len(), 3);
    assert_eq!(reply(&mut t), "");
}

#[test]
fn placements_follow_reflow() {
    let mut t = Terminal::new(4, 10);
    t.write(b"abcdefgh");
    send_big(&mut t, "a=T,i=1,C=1");
    assert_eq!(spots(&t), [(0, 8, 1, 0)]);
    t.resize(4, 5);
    // 8 列目の字（i）は 2 行目の 4 列目へ
    assert_eq!(spots(&t), [(1, 3, 1, 0)]);
    t.resize(4, 10);
    assert_eq!(spots(&t), [(0, 8, 1, 0)]);
    // 代替画面は組み直さず、はみ出した列の配置を捨てる
    t.write(b"\x1b[?1049h");
    t.write(b"\x1b[1;9H");
    send_big(&mut t, "a=T,i=2,C=1");
    t.resize(4, 5);
    assert!(t.placements().is_empty());
}

#[test]
fn resending_gives_the_image_a_new_serial() {
    let mut t = Terminal::new(5, 20);
    apc(&mut t, "f=24,s=1,v=1,i=1;AAAA");
    apc(&mut t, "f=24,s=1,v=1,i=2;AAAA");
    let (a, b) = (t.image(1).unwrap().serial, t.image(2).unwrap().serial);
    assert!(a > 0 && b > a);
    apc(&mut t, "f=24,s=1,v=1,i=1;/wAA");
    t.take_output();
    assert!(t.image(1).unwrap().serial > b);
}

#[test]
fn native_size_is_only_without_c_and_r() {
    let mut t = Terminal::new(10, 40);
    send_big(&mut t, "a=t,i=1");
    apc(&mut t, "a=p,i=1,C=1");
    apc(&mut t, "a=p,i=1,C=1,c=2");
    apc(&mut t, "a=p,i=1,C=1,r=2");
    apc(&mut t, "a=p,i=1,C=1,c=2,r=2");
    let native: Vec<_> = t
        .placements()
        .iter()
        .map(|p| p.placement.native_size)
        .collect();
    assert_eq!(native, [true, false, false, false]);
}
