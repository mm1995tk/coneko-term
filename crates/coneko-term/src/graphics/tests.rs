use super::*;
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
    for a in ["T", "p", "d", "x"] {
        apc(&mut t, &format!("a={a},f=24,s=1,v=1,i=1;AAAA"));
        assert_eq!(reply(&mut t), "", "a={a}");
    }
    assert_eq!(t.images().count(), 0);
    // 分割された a=T の続きも、画像にならない。
    apc(&mut t, "a=T,f=24,s=1,v=1,i=1,m=1;AAAA");
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
