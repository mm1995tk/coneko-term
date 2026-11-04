//! kitty の drag and drop protocol（OSC 72）の単体テスト。仕様の例に沿う。

use super::*;

fn term() -> Terminal {
    Terminal::new(10, 40)
}

fn send(t: &mut Terminal, meta: &str, payload: &str) {
    let sep = if payload.is_empty() { "" } else { ";" };
    t.write(format!("\x1b]72;{meta}{sep}{payload}\x1b\\").as_bytes());
}

fn out(t: &mut Terminal) -> String {
    String::from_utf8(t.take_output()).unwrap()
}

fn dnd(t: &mut Terminal) -> Vec<DndEvent> {
    t.take_events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Dnd(d) => Some(*d),
            _ => None,
        })
        .collect()
}

fn b64(s: &[u8]) -> String {
    clipboard::encode_base64(s)
}

fn loc(x: i32, y: i32) -> DndLocation {
    DndLocation {
        x,
        y,
        pixel_x: x * 10 + 3,
        pixel_y: y * 20 + 5,
    }
}

const BOTH: DndOperations = DndOperations {
    copy: true,
    r#move: true,
};

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

// ---- 対応の問い合わせ ----

#[test]
fn query_is_answered_before_da1() {
    let mut t = term();
    send(&mut t, "t=q:i=7", "");
    t.write(b"\x1b[c");
    assert_eq!(out(&mut t), "\x1b]72;t=q:i=7\x1b\\\x1b[?1;2c");
    send(&mut t, "t=q", "");
    assert_eq!(out(&mut t), "\x1b]72;t=q\x1b\\");
}

#[test]
fn query_is_answered_in_the_middle_of_a_chunked_transfer() {
    let mut t = term();
    send(&mut t, "t=a:m=1", "text/pl");
    send(&mut t, "t=q", "");
    send(&mut t, "m=0", "ain");
    assert_eq!(out(&mut t), "\x1b]72;t=q\x1b\\");
    assert_eq!(
        dnd(&mut t),
        vec![DndEvent::DropEnabled {
            mimes: s(&["text/plain"])
        }]
    );
}

// ---- ドロップを受ける ----

#[test]
fn accept_and_stop() {
    let mut t = term();
    assert!(!t.drop_accepting());
    send(&mut t, "t=a", "text/plain application/x-private");
    assert!(t.drop_accepting());
    assert_eq!(
        t.drop_accepted_mimes(),
        ["text/plain", "application/x-private"]
    );
    send(&mut t, "t=A", "");
    assert!(!t.drop_accepting());
    assert_eq!(
        dnd(&mut t),
        vec![
            DndEvent::DropEnabled {
                mimes: s(&["text/plain", "application/x-private"])
            },
            DndEvent::DropDisabled
        ]
    );
    // 受け付けていない間は何も出ない。
    assert!(!t.drop_move(loc(1, 2), &s(&["text/plain"]), BOTH));
    assert_eq!(out(&mut t), "");
}

#[test]
fn move_events_send_the_mime_list_only_when_it_changes() {
    let mut t = term();
    send(&mut t, "t=a", "");
    assert!(t.drop_move(loc(3, 4), &s(&["text/plain", "text/uri-list"]), BOTH));
    assert_eq!(
        out(&mut t),
        "\x1b]72;t=m:x=3:y=4:X=33:Y=85:o=3;text/plain text/uri-list\x1b\\"
    );
    t.drop_move(loc(4, 4), &s(&["text/plain", "text/uri-list"]), BOTH);
    assert_eq!(out(&mut t), "\x1b]72;t=m:x=4:y=4:X=43:Y=85:o=3\x1b\\");
    t.drop_move(
        loc(4, 4),
        &s(&["text/plain"]),
        DndOperations::from_bits(1).unwrap(),
    );
    assert_eq!(
        out(&mut t),
        "\x1b]72;t=m:x=4:y=4:X=43:Y=85:o=1;text/plain\x1b\\"
    );
}

#[test]
fn leaving_sends_minus_one_and_the_list_is_sent_again_next_time() {
    let mut t = term();
    send(&mut t, "t=a", "");
    assert!(!t.drop_leave());
    t.drop_move(loc(0, 0), &s(&["text/plain"]), BOTH);
    out(&mut t);
    assert!(t.drop_leave());
    assert_eq!(out(&mut t), "\x1b]72;t=m:x=-1:y=-1:X=0:Y=0:o=0\x1b\\");
    t.drop_move(loc(0, 0), &s(&["text/plain"]), BOTH);
    assert!(out(&mut t).ends_with(";text/plain\x1b\\"));
    // 負の位置の移動は出さない。
    assert!(!t.drop_move(loc(-1, 0), &s(&[]), BOTH));
}

#[test]
fn response_to_a_move_becomes_an_event() {
    let mut t = term();
    send(&mut t, "t=a", "");
    // ドラッグが無いうちの答えは捨てる。
    send(&mut t, "t=m:o=1", "text/plain");
    assert!(dnd(&mut t).len() == 1);
    t.drop_move(loc(0, 0), &s(&["text/plain", "text/uri-list"]), BOTH);
    send(&mut t, "t=m:o=1", "text/uri-list text/plain");
    send(&mut t, "t=m:o=2", "");
    send(&mut t, "t=m:o=0", "");
    send(&mut t, "t=m:o=9", "");
    assert_eq!(
        dnd(&mut t),
        vec![
            DndEvent::DropResponse {
                operation: Some(DndOperation::Copy),
                mimes: Some(s(&["text/uri-list", "text/plain"]))
            },
            DndEvent::DropResponse {
                operation: Some(DndOperation::Move),
                mimes: None
            },
            DndEvent::DropResponse {
                operation: None,
                mimes: None
            },
        ]
    );
}

fn dropped(t: &mut Terminal, mimes: &[&str], from_self: bool) {
    send(t, "t=a", "");
    t.drop_move(loc(1, 1), &s(mimes), BOTH);
    assert!(t.drop_drop(loc(2, 3), &s(mimes), BOTH, from_self));
    dnd(t);
    out(t);
}

#[test]
fn drop_sends_the_full_list_and_requests_become_events() {
    let mut t = term();
    send(&mut t, "t=a", "");
    t.drop_move(loc(1, 1), &s(&["text/plain"]), BOTH);
    dnd(&mut t);
    out(&mut t);
    // 空の一覧のドロップは出さない。
    assert!(!t.drop_drop(loc(2, 3), &[], BOTH, false));
    assert!(t.drop_drop(loc(2, 3), &s(&["text/plain", "text/uri-list"]), BOTH, false));
    assert_eq!(
        out(&mut t),
        "\x1b]72;t=M:x=2:y=3:X=23:Y=65:o=3;text/plain text/uri-list\x1b\\"
    );
    // ドロップのあとの移動は、新しいドラッグ。
    send(&mut t, "t=r:x=2", "");
    send(&mut t, "t=r:x=1", "");
    assert_eq!(
        dnd(&mut t),
        vec![
            DndEvent::DropRequest(DropRequest::Data { index: 2 }),
            DndEvent::DropRequest(DropRequest::Data { index: 1 })
        ]
    );
}

#[test]
fn data_before_the_drop_is_refused_with_eperm() {
    let mut t = term();
    send(&mut t, "t=a", "");
    dnd(&mut t);
    t.drop_move(loc(1, 1), &s(&["text/plain"]), BOTH);
    out(&mut t);
    send(&mut t, "t=r:x=1", "");
    assert_eq!(out(&mut t), "\x1b]72;t=R:x=1;EPERM\x1b\\");
    assert!(dnd(&mut t).is_empty());
    // ドロップのあとなら通る。
    t.drop_drop(loc(1, 1), &s(&["text/plain"]), BOTH, false);
    out(&mut t);
    send(&mut t, "t=r:x=1", "");
    assert_eq!(dnd(&mut t).len(), 1);
}

#[test]
fn out_of_range_index_is_enoent() {
    let mut t = term();
    dropped(&mut t, &["text/plain"], false);
    send(&mut t, "t=r:x=2", "");
    assert_eq!(out(&mut t), "\x1b]72;t=R:x=2;ENOENT\x1b\\");
    send(&mut t, "t=r:x=0", "");
    assert_eq!(out(&mut t), "\x1b]72;t=R:x=0;ENOENT\x1b\\");
}

#[test]
fn data_reply_is_chunked_and_ends_with_an_empty_packet() {
    let mut t = term();
    dropped(&mut t, &["text/plain"], false);
    send(&mut t, "t=r:x=1", "");
    let req = match dnd(&mut t).remove(0) {
        DndEvent::DropRequest(r) => r,
        e => panic!("{e:?}"),
    };
    t.drop_data_reply(&req, DropForm::Mime { remote: false }, b"hello");
    assert_eq!(
        out(&mut t),
        format!(
            "\x1b]72;t=r:x=1:m=1;{}\x1b\\\x1b]72;t=r:x=1:m=0\x1b\\",
            b64(b"hello")
        )
    );
    // 4096 バイトを超えたら分ける。
    send(&mut t, "t=r:x=1", "");
    dnd(&mut t);
    let big = vec![b'a'; 3072 + 10];
    t.drop_data_reply(&req, DropForm::Mime { remote: false }, &big);
    let o = out(&mut t);
    let packets: Vec<&str> = o.split("\x1b\\").filter(|p| !p.is_empty()).collect();
    assert_eq!(packets.len(), 3);
    assert!(packets[0].starts_with("\x1b]72;t=r:x=1:m=1;"));
    assert_eq!(packets[0].split(';').last().unwrap().len(), 4096);
    assert!(packets[1].starts_with("\x1b]72;t=r:x=1:m=1;"));
    assert_eq!(packets[2], "\x1b]72;t=r:x=1:m=0");
}

#[test]
fn error_reply_has_a_posix_name_and_a_description() {
    let mut t = term();
    dropped(&mut t, &["text/plain"], false);
    send(&mut t, "t=r:x=1", "");
    let DndEvent::DropRequest(req) = dnd(&mut t).remove(0) else {
        panic!()
    };
    t.drop_error_reply(&req, DndError::Eio, "disk\x1b gone");
    assert_eq!(out(&mut t), "\x1b]72;t=R:x=1;EIO:disk gone\x1b\\");
}

#[test]
fn finishing_the_drop_ends_it() {
    let mut t = term();
    dropped(&mut t, &["text/plain"], false);
    send(&mut t, "t=r:o=1", "");
    assert_eq!(
        dnd(&mut t),
        vec![DndEvent::DropFinished {
            operation: Some(DndOperation::Copy)
        }]
    );
    // 終わったあとの頼みは断る。
    send(&mut t, "t=r:x=1", "");
    assert_eq!(out(&mut t), "\x1b]72;t=R:x=1;EPERM\x1b\\");
    // `o` が無ければ取り消し。
    dropped(&mut t, &["text/plain"], false);
    send(&mut t, "t=r", "");
    assert_eq!(
        dnd(&mut t),
        vec![DndEvent::DropFinished { operation: None }]
    );
}

#[test]
fn leaving_without_a_drop_discards_the_session() {
    let mut t = term();
    send(&mut t, "t=a", "");
    t.drop_move(loc(1, 1), &s(&["text/plain"]), BOTH);
    t.drop_leave();
    out(&mut t);
    send(&mut t, "t=r:x=1", "");
    assert_eq!(out(&mut t), "\x1b]72;t=R:x=1;EPERM\x1b\\");
}

// ---- 遠くの機械からのドロップ ----

#[test]
fn machine_id_is_kept() {
    let mut t = term();
    send(&mut t, "t=a", "");
    send(&mut t, "t=a:x=1", "1:abcdef0123");
    assert_eq!(t.drop_machine_id(), Some("1:abcdef0123"));
    // 字が不正なら捨てる。
    send(&mut t, "t=a:x=1", "bad id");
    assert_eq!(t.drop_machine_id(), Some("1:abcdef0123"));
}

#[test]
fn remote_drop_of_files_and_directories() {
    let mut t = term();
    dropped(&mut t, &["text/plain", "text/uri-list"], false);
    // text/uri-list を先に頼まないと EINVAL。
    send(&mut t, "t=r:x=2:y=1", "");
    assert_eq!(out(&mut t), "\x1b]72;t=R:x=2:y=1;EINVAL\x1b\\");
    send(&mut t, "t=r:x=2", "");
    let DndEvent::DropRequest(list) = dnd(&mut t).remove(0) else {
        panic!()
    };
    t.drop_data_reply(
        &list,
        DropForm::Mime { remote: true },
        b"file:///a\r\nfile:///d/\r\n",
    );
    assert!(out(&mut t).starts_with("\x1b]72;t=r:x=2:X=1:m=1;"));
    // text/plain の項目は指せない。
    send(&mut t, "t=r:x=1:y=1", "");
    assert_eq!(out(&mut t), "\x1b]72;t=R:x=1:y=1;EINVAL\x1b\\");
    // ファイル、シンボリックリンク。
    send(&mut t, "t=r:x=2:y=1", "");
    send(&mut t, "t=r:x=2:y=2", "");
    let reqs: Vec<_> = dnd(&mut t)
        .into_iter()
        .map(|e| match e {
            DndEvent::DropRequest(r) => r,
            e => panic!("{e:?}"),
        })
        .collect();
    assert_eq!(
        reqs,
        vec![
            DropRequest::Entry {
                index: 2,
                sub_index: 1
            },
            DropRequest::Entry {
                index: 2,
                sub_index: 2
            }
        ]
    );
    t.drop_data_reply(&reqs[0], DropForm::File, b"abc");
    assert_eq!(
        out(&mut t),
        format!(
            "\x1b]72;t=r:x=2:y=1:m=1;{}\x1b\\\x1b]72;t=r:x=2:y=1:m=0\x1b\\",
            b64(b"abc")
        )
    );
    // ディレクトリ: X=handle で、中の項目の一覧を返す。
    t.drop_data_reply(&reqs[1], DropForm::Directory(5), b"a.txt\0sub\0");
    assert!(out(&mut t).starts_with("\x1b]72;t=r:x=2:y=2:X=5:m=1;"));
    send(&mut t, "t=r:Y=5:x=1", "");
    send(&mut t, "t=r:Y=5:x=2", "");
    let reqs: Vec<_> = dnd(&mut t)
        .into_iter()
        .map(|e| match e {
            DndEvent::DropRequest(r) => r,
            e => panic!("{e:?}"),
        })
        .collect();
    assert_eq!(
        reqs,
        vec![
            DropRequest::DirEntry {
                handle: 5,
                index: 1
            },
            DropRequest::DirEntry {
                handle: 5,
                index: 2
            }
        ]
    );
    t.drop_data_reply(&reqs[0], DropForm::Symlink, b"../target");
    assert!(out(&mut t).starts_with("\x1b]72;t=r:Y=5:x=1:X=1:m=1;"));
    t.drop_error_reply(&reqs[1], DndError::Enoent, "");
    assert_eq!(out(&mut t), "\x1b]72;t=R:Y=5:x=2;ENOENT\x1b\\");
    // ディレクトリを閉じると handle は無効になる。
    send(&mut t, "t=r:Y=5", "");
    assert_eq!(
        dnd(&mut t),
        vec![DndEvent::DropRequest(DropRequest::DirClose { handle: 5 })]
    );
    send(&mut t, "t=r:Y=5:x=1", "");
    assert_eq!(out(&mut t), "\x1b]72;t=R:Y=5:x=1;EINVAL\x1b\\");
    send(&mut t, "t=r:Y=9", "");
    assert_eq!(out(&mut t), "\x1b]72;t=R:Y=9;EINVAL\x1b\\");
}

#[test]
fn files_cannot_be_read_when_the_drag_started_in_this_window() {
    let mut t = term();
    dropped(&mut t, &["text/uri-list"], true);
    send(&mut t, "t=r:x=1", "");
    dnd(&mut t);
    send(&mut t, "t=r:x=1:y=1", "");
    assert_eq!(out(&mut t), "\x1b]72;t=R:x=1:y=1;EPERM\x1b\\");
}

#[test]
fn too_many_requests_end_the_drop_with_emfile() {
    let mut t = term();
    dropped(&mut t, &["text/plain"], false);
    for _ in 0..dnd::MAX_PENDING {
        send(&mut t, "t=r:x=1", "");
    }
    assert_eq!(dnd(&mut t).len(), dnd::MAX_PENDING);
    send(&mut t, "t=r:x=1", "");
    assert_eq!(out(&mut t), "\x1b]72;t=R:x=1;EMFILE\x1b\\");
    assert_eq!(
        dnd(&mut t),
        vec![DndEvent::DropFinished { operation: None }]
    );
}

#[test]
fn multiplexer_id_is_echoed() {
    let mut t = term();
    send(&mut t, "t=a:i=42", "");
    t.drop_move(loc(0, 0), &s(&[]), BOTH);
    assert_eq!(out(&mut t), "\x1b]72;t=m:x=0:y=0:X=3:Y=5:o=3:i=42\x1b\\");
    t.drop_drop(loc(0, 0), &s(&["text/plain"]), BOTH, false);
    out(&mut t);
    send(&mut t, "t=r:x=5", "");
    assert_eq!(out(&mut t), "\x1b]72;t=R:x=5:i=42;ENOENT\x1b\\");
}

#[test]
fn long_mime_list_is_chunked() {
    let mut t = term();
    send(&mut t, "t=a", "");
    let mimes: Vec<String> = (0..200)
        .map(|i| format!("application/x-long-type-{i:04}"))
        .collect();
    t.drop_move(loc(0, 0), &mimes, BOTH);
    let o = out(&mut t);
    let packets: Vec<&str> = o.split("\x1b\\").filter(|p| !p.is_empty()).collect();
    assert!(packets.len() > 1);
    assert!(packets[0].contains(":m=1;"));
    assert!(packets.last().unwrap().contains(":m=0;"));
}

#[test]
fn reset_reports_disabled_and_forgets() {
    let mut t = term();
    send(&mut t, "t=a", "");
    send(&mut t, "t=o:x=1", "");
    dnd(&mut t);
    t.write(b"\x1bc");
    assert_eq!(
        dnd(&mut t),
        vec![DndEvent::DropDisabled, DndEvent::DragDisabled]
    );
    assert!(!t.drop_accepting());
    assert!(!t.drag_offers_enabled());
}

// ---- ドラッグを始める ----

fn enable_offers(t: &mut Terminal) {
    send(t, "t=o:x=1", "1:machine");
    assert_eq!(t.drag_machine_id(), Some("1:machine"));
    dnd(t);
}

/// 身振りから `t=P:x=-1` まで。
fn start_drag(t: &mut Terminal) -> Box<DragOffer> {
    enable_offers(t);
    assert!(t.drag_gesture(loc(5, 6)));
    assert_eq!(out(t), "\x1b]72;t=o:x=5:y=6:X=53:Y=125\x1b\\");
    send(t, "t=o:o=3", "text/plain text/uri-list");
    send(t, "t=p:x=0", &b64(b"hello"));
    send(t, "t=P:x=-1", "");
    match dnd(t).remove(0) {
        DndEvent::DragStart(o) => o,
        e => panic!("{e:?}"),
    }
}

#[test]
fn enabling_and_disabling_offers() {
    let mut t = term();
    send(&mut t, "t=o:x=1", "");
    assert!(t.drag_offers_enabled());
    send(&mut t, "t=o:x=2", "");
    assert!(!t.drag_offers_enabled());
    assert_eq!(
        dnd(&mut t),
        vec![
            DndEvent::DragEnabled { machine_id: None },
            DndEvent::DragDisabled
        ]
    );
    assert!(!t.drag_gesture(loc(0, 0)));
}

#[test]
fn full_drag_flow() {
    let mut t = term();
    let o = start_drag(&mut t);
    assert_eq!(
        *o,
        DragOffer {
            operations: BOTH,
            mimes: s(&["text/plain", "text/uri-list"]),
            data: vec![Some(b"hello".to_vec()), None],
            images: vec![],
            image: None,
            machine_id: Some("1:machine".to_string()),
        }
    );
    assert_eq!(out(&mut t), "");
    assert!(t.drag_started());
    assert_eq!(out(&mut t), "\x1b]72;t=E;OK\x1b\\");
    // 進み具合。
    assert!(t.drag_accepted(1));
    assert!(t.drag_action(Some(DndOperation::Move)));
    assert!(t.drag_dropped());
    assert!(t.drag_data_request(1));
    assert!(!t.drag_data_request(2));
    assert_eq!(
        out(&mut t),
        "\x1b]72;t=e:x=1:y=1\x1b\\\x1b]72;t=e:x=2:o=2\x1b\\\x1b]72;t=e:x=3\x1b\\\x1b]72;t=e:x=5:y=1\x1b\\"
    );
    // データ。
    send(&mut t, "t=e:y=1:m=1", &b64(b"abc"));
    send(&mut t, "t=e:y=1:m=0", "");
    send(&mut t, "t=E:y=0", "ENOENT:gone");
    assert_eq!(
        dnd(&mut t),
        vec![
            DndEvent::DragData {
                index: 1,
                data: b"abc".to_vec()
            },
            DndEvent::DragDataError {
                index: 0,
                error: "ENOENT".to_string(),
                description: "gone".to_string()
            }
        ]
    );
    assert!(t.drag_finished(false));
    assert_eq!(out(&mut t), "\x1b]72;t=e:x=4:y=0\x1b\\");
    // 終わったあとは何も出ない。
    assert!(!t.drag_dropped());
}

#[test]
fn drag_finished_canceled() {
    let mut t = term();
    start_drag(&mut t);
    t.drag_started();
    out(&mut t);
    assert!(t.drag_finished(true));
    assert_eq!(out(&mut t), "\x1b]72;t=e:x=4:y=1\x1b\\");
}

#[test]
fn offer_without_a_gesture_is_refused() {
    let mut t = term();
    enable_offers(&mut t);
    send(&mut t, "t=o:o=1", "text/plain");
    assert_eq!(out(&mut t), "\x1b]72;t=E;EPERM\x1b\\");
    // 身振りは 1 回だけ。
    t.drag_gesture(loc(0, 0));
    out(&mut t);
    send(&mut t, "t=o:o=1", "text/plain");
    send(&mut t, "t=P:x=-1", "");
    assert!(matches!(dnd(&mut t).as_slice(), [DndEvent::DragStart(_)]));
    t.drag_error(DndError::Eperm, "");
    out(&mut t);
    send(&mut t, "t=o:o=1", "text/plain");
    assert_eq!(out(&mut t), "\x1b]72;t=E;EPERM\x1b\\");
}

#[test]
fn start_without_an_offer_is_refused() {
    let mut t = term();
    enable_offers(&mut t);
    send(&mut t, "t=P:x=-1", "");
    assert_eq!(out(&mut t), "\x1b]72;t=E;EPERM\x1b\\");
    assert!(dnd(&mut t).is_empty());
}

#[test]
fn start_failure_reply() {
    let mut t = term();
    start_drag(&mut t);
    assert!(t.drag_error(DndError::Efbig, "image too large"));
    assert_eq!(out(&mut t), "\x1b]72;t=E;EFBIG:image too large\x1b\\");
    assert!(!t.drag_started());
}

#[test]
fn presented_data_errors() {
    let mut t = term();
    enable_offers(&mut t);
    t.drag_gesture(loc(0, 0));
    out(&mut t);
    send(&mut t, "t=o:o=1", "text/plain");
    // 範囲外の番号。
    send(&mut t, "t=p:x=1", &b64(b"x"));
    assert_eq!(out(&mut t), "\x1b]72;t=E;EINVAL\x1b\\");
    // 取り消されたので、始められない。
    send(&mut t, "t=P:x=-1", "");
    assert_eq!(out(&mut t), "\x1b]72;t=E;EPERM\x1b\\");
}

#[test]
fn presented_data_can_be_chunked_and_empty() {
    let mut t = term();
    enable_offers(&mut t);
    t.drag_gesture(loc(0, 0));
    send(&mut t, "t=o:o=2", "text/plain text/uri-list");
    let all = b64(b"hello world");
    send(&mut t, "t=p:x=0:m=1", &all[..8]);
    send(&mut t, "m=1", &all[8..]);
    send(&mut t, "m=0", "");
    send(&mut t, "t=p:x=1:m=0", "");
    send(&mut t, "t=P:x=-1", "");
    let o = match dnd(&mut t).remove(0) {
        DndEvent::DragStart(o) => o,
        e => panic!("{e:?}"),
    };
    assert_eq!(o.data, vec![Some(b"hello world".to_vec()), Some(vec![])]);
    assert_eq!(
        o.operations,
        DndOperations {
            copy: false,
            r#move: true
        }
    );
}

#[test]
fn images() {
    let mut t = term();
    enable_offers(&mut t);
    t.drag_gesture(loc(0, 0));
    out(&mut t);
    send(&mut t, "t=o:o=1", "text/plain");
    // 1 つめ: RGB 2x1。
    send(&mut t, "t=p:x=-1:y=24:X=2:Y=1", &b64(&[1, 2, 3, 4, 5, 6]));
    // 2 つめ: 絵文字のテキスト。背景は不透明。
    send(&mut t, "t=p:x=-2:y=0:X=2:Y=1:o=1024", &b64("🐱".as_bytes()));
    // 3 つめ: PNG。
    send(&mut t, "t=p:x=-3:y=100:X=1:Y=1", &b64(b"\x89PNG"));
    send(&mut t, "t=P:x=1", "");
    send(&mut t, "t=P:x=-1", "");
    let DndEvent::DragStart(o) = dnd(&mut t).remove(0) else {
        panic!()
    };
    assert_eq!(out(&mut t), "");
    assert_eq!(o.image, Some(1));
    assert_eq!(
        o.images,
        vec![
            DragImage {
                format: DragImageFormat::Rgb,
                width: 2,
                height: 1,
                opacity: 0,
                data: vec![1, 2, 3, 4, 5, 6]
            },
            DragImage {
                format: DragImageFormat::Text,
                width: 2,
                height: 1,
                opacity: 1024,
                data: "🐱".as_bytes().to_vec()
            },
            DragImage {
                format: DragImageFormat::Png,
                width: 1,
                height: 1,
                opacity: 0,
                data: b"\x89PNG".to_vec()
            },
        ]
    );
    // 始まったあとに画像を変える。範囲外は外す。
    t.drag_started();
    out(&mut t);
    send(&mut t, "t=P:x=0", "");
    send(&mut t, "t=P:x=9", "");
    assert_eq!(
        dnd(&mut t),
        vec![
            DndEvent::DragImage { index: Some(0) },
            DndEvent::DragImage { index: None }
        ]
    );
}

#[test]
fn first_image_is_the_default() {
    let mut t = term();
    enable_offers(&mut t);
    t.drag_gesture(loc(0, 0));
    send(&mut t, "t=o:o=1", "text/plain");
    send(&mut t, "t=p:x=-1:y=32:X=1:Y=1", &b64(&[1, 2, 3, 4]));
    send(&mut t, "t=P:x=-1", "");
    let DndEvent::DragStart(o) = dnd(&mut t).remove(0) else {
        panic!()
    };
    assert_eq!(o.image, Some(0));
}

#[test]
fn image_errors() {
    for (meta, payload) in [
        // 大きさが合わない。
        ("t=p:x=-1:y=24:X=2:Y=2", b64(&[0; 6])),
        ("t=p:x=-1:y=32:X=1:Y=2", b64(&[0; 4])),
        // 最初が -1 でない。
        ("t=p:x=-2:y=32:X=1:Y=1", b64(&[0; 4])),
        // 知らない形式。
        ("t=p:x=-1:y=7:X=1:Y=1", b64(&[0; 4])),
        // 不透明度が範囲外。
        ("t=p:x=-1:y=0:X=1:Y=1:o=2000", b64(b"a")),
    ] {
        let mut t = term();
        enable_offers(&mut t);
        t.drag_gesture(loc(0, 0));
        out(&mut t);
        send(&mut t, "t=o:o=1", "text/plain");
        send(&mut t, meta, &payload);
        let first = out(&mut t);
        let mut got = first.clone();
        if first.is_empty() {
            send(&mut t, meta, "");
            got = out(&mut t);
        }
        assert_eq!(got, "\x1b]72;t=E;EINVAL\x1b\\", "{meta}");
    }
}

#[test]
fn too_much_presented_data_is_efbig() {
    let mut t = term();
    enable_offers(&mut t);
    t.drag_gesture(loc(0, 0));
    out(&mut t);
    send(&mut t, "t=o:o=1", "text/plain");
    let chunk = b64(&vec![0u8; 3072]);
    let n = dnd::MAX_PRESENT_BYTES / 3072 + 2;
    let mut got = String::new();
    send(&mut t, "t=p:x=0:m=1", &chunk);
    for _ in 0..n {
        send(&mut t, "m=1", &chunk);
        got = out(&mut t);
        if !got.is_empty() {
            break;
        }
    }
    assert_eq!(got, "\x1b]72;t=E;EFBIG\x1b\\");
}

#[test]
fn too_many_mime_types_is_efbig() {
    let mut t = term();
    enable_offers(&mut t);
    t.drag_gesture(loc(0, 0));
    out(&mut t);
    let mimes: Vec<String> = (0..300).map(|i| format!("x/{i}")).collect();
    send(&mut t, "t=o:o=1", &mimes.join(" "));
    assert_eq!(out(&mut t), "\x1b]72;t=E;EFBIG\x1b\\");
}

#[test]
fn client_can_cancel_or_abort() {
    let mut t = term();
    start_drag(&mut t);
    t.drag_started();
    out(&mut t);
    send(&mut t, "t=E:y=-1", "");
    assert_eq!(dnd(&mut t), vec![DndEvent::DragCanceled]);
    assert!(!t.drag_dropped());

    start_drag(&mut t);
    t.drag_started();
    out(&mut t);
    send(&mut t, "t=E", "EIO:read failed");
    assert_eq!(
        dnd(&mut t),
        vec![DndEvent::DragAborted {
            error: "EIO".to_string(),
            description: "read failed".to_string()
        }]
    );
}

#[test]
fn data_before_the_drag_started_is_einval_and_aborts() {
    let mut t = term();
    start_drag(&mut t);
    // GUI の答えの前。
    send(&mut t, "t=e:y=0:m=0", &b64(b"x"));
    assert_eq!(out(&mut t), "\x1b]72;t=E;EINVAL\x1b\\");
    assert_eq!(dnd(&mut t), vec![DndEvent::DragCanceled]);
    assert!(!t.drag_started());
}

#[test]
fn data_with_a_bad_index_is_refused() {
    let mut t = term();
    start_drag(&mut t);
    t.drag_started();
    out(&mut t);
    send(&mut t, "t=e:y=5:m=0", &b64(b"x"));
    assert_eq!(out(&mut t), "\x1b]72;t=E:y=5;EINVAL\x1b\\");
    assert!(dnd(&mut t).is_empty());
}

#[test]
fn remote_drag_entries() {
    let mut t = term();
    start_drag(&mut t);
    t.drag_started();
    out(&mut t);
    assert!(t.drag_entry_request(1));
    assert!(!t.drag_entry_request(0));
    assert_eq!(out(&mut t), "\x1b]72;t=k:x=1\x1b\\");
    send(&mut t, "t=k:x=1:m=1", &b64(b"ab"));
    send(&mut t, "t=k:x=1:m=0", "");
    send(&mut t, "t=k:x=2:X=1:m=0", &b64(b"../x"));
    send(&mut t, "t=k:x=2:X=1", "");
    send(&mut t, "t=k:x=3:X=7:m=0", &b64(b"a\0b\0"));
    send(&mut t, "t=k:x=3:X=7", "");
    send(&mut t, "t=k:x=3:Y=7:y=2:m=0", &b64(b"data"));
    send(&mut t, "t=k:x=3:Y=7:y=2", "");
    let key = |index, sub_index, parent| DragEntryKey {
        index,
        sub_index,
        parent,
    };
    assert_eq!(
        dnd(&mut t),
        vec![
            DndEvent::DragEntry {
                key: key(1, 0, 0),
                form: DragEntryForm::File,
                data: b"ab".to_vec()
            },
            DndEvent::DragEntry {
                key: key(2, 0, 0),
                form: DragEntryForm::Symlink,
                data: b"../x".to_vec()
            },
            DndEvent::DragEntry {
                key: key(3, 0, 0),
                form: DragEntryForm::Directory(7),
                data: b"a\0b\0".to_vec()
            },
            DndEvent::DragEntry {
                key: key(3, 2, 7),
                form: DragEntryForm::File,
                data: b"data".to_vec()
            },
        ]
    );
}

#[test]
fn multiplexer_id_is_echoed_on_the_drag_side() {
    let mut t = term();
    send(&mut t, "t=o:x=1:i=9", "");
    t.drag_gesture(loc(0, 0));
    assert_eq!(out(&mut t), "\x1b]72;t=o:x=0:y=0:X=3:Y=5:i=9\x1b\\");
    send(&mut t, "t=P:x=-1", "");
    assert_eq!(out(&mut t), "\x1b]72;t=E:i=9;EPERM\x1b\\");
}

#[test]
fn another_kind_interrupting_a_chunked_transfer_drops_it() {
    let mut t = term();
    send(&mut t, "t=a:m=1", "text/pl");
    send(&mut t, "t=A", "");
    send(&mut t, "m=0", "ain");
    // `t` の無いパケットは `t=a` の既定ではなく、続きでもないので、`t=a` として読まれる。
    assert!(dnd(&mut t)
        .iter()
        .all(|e| !format!("{e:?}").contains("text/plain")));
}

#[test]
fn unknown_and_invalid_packets_are_dropped() {
    let mut t = term();
    for (meta, payload) in [
        ("t=Z", ""),
        ("t=ab", ""),
        ("t=r:x=abc", ""),
        ("x=99999999999", ""),
    ] {
        send(&mut t, meta, payload);
    }
    assert!(dnd(&mut t).is_empty());
    assert_eq!(out(&mut t), "");
}

#[test]
fn kitten_style_end_of_data_is_an_empty_packet() {
    let mut t = term();
    enable_offers(&mut t);
    t.drag_gesture(loc(0, 0));
    out(&mut t);
    send(&mut t, "t=o:o=1", "text/plain text/uri-list");
    // データつきの `m=0` のあとに、空のパケット。先に送ったデータは消えない。
    send(&mut t, "t=p:x=0:m=0", &b64(b"hello"));
    send(&mut t, "t=p:x=0", "");
    send(&mut t, "t=p:x=1:m=0", &b64(b"a"));
    send(&mut t, "t=p:x=1:m=0", &b64(b"b"));
    send(&mut t, "t=p:x=1", "");
    // 画像も同じ。
    send(&mut t, "t=p:x=-1:y=32:X=1:Y=1:m=0", &b64(&[1, 2, 3, 4]));
    send(&mut t, "t=p:x=-1:y=32:X=1:Y=1", "");
    send(&mut t, "t=P:x=-1", "");
    assert_eq!(out(&mut t), "");
    let DndEvent::DragStart(o) = dnd(&mut t).remove(0) else {
        panic!()
    };
    assert_eq!(o.data, vec![Some(b"hello".to_vec()), Some(b"ab".to_vec())]);
    assert_eq!(o.images.len(), 1);
    assert_eq!(o.images[0].data, vec![1, 2, 3, 4]);
    t.drag_started();
    out(&mut t);
    // `y` が無ければ 0。分けて来たデータは、空のパケットでそろう。
    send(&mut t, "t=e:m=1", &b64(b"abc"));
    send(&mut t, "t=e:m=0", &b64(b"def"));
    send(&mut t, "t=e:y=1:m=0", &b64(b"x"));
    assert!(dnd(&mut t).is_empty());
    send(&mut t, "t=e", "");
    send(&mut t, "t=e:y=1", "");
    assert_eq!(
        dnd(&mut t),
        vec![
            DndEvent::DragData {
                index: 0,
                data: b"abcdef".to_vec()
            },
            DndEvent::DragData {
                index: 1,
                data: b"x".to_vec()
            },
        ]
    );
    assert_eq!(out(&mut t), "");
}

#[test]
fn image_index_out_of_range_removes_the_image() {
    let mut t = term();
    enable_offers(&mut t);
    t.drag_gesture(loc(0, 0));
    send(&mut t, "t=o:o=1", "text/plain");
    send(&mut t, "t=p:x=-1:y=32:X=1:Y=1", &b64(&[1, 2, 3, 4]));
    send(&mut t, "t=P:x=-1", "");
    dnd(&mut t);
    t.drag_started();
    out(&mut t);
    send(&mut t, "t=P:x=1", "");
    send(&mut t, "t=P:x=-1", "");
    assert_eq!(dnd(&mut t), vec![DndEvent::DragImage { index: None }]);
    assert_eq!(out(&mut t), "");
}

#[test]
fn offer_errors_tell_too_many_from_invalid() {
    let mut t = term();
    enable_offers(&mut t);
    t.drag_gesture(loc(0, 0));
    out(&mut t);
    send(
        &mut t,
        "t=o:o=1",
        &format!("text/plain {}", "x".repeat(300)),
    );
    assert_eq!(out(&mut t), "\x1b]72;t=E;EINVAL\x1b\\");
}

#[test]
fn disabling_during_a_drop_finishes_it() {
    let mut t = term();
    dropped(&mut t, &["text/plain"], false);
    send(&mut t, "t=A", "");
    assert_eq!(
        dnd(&mut t),
        vec![
            DndEvent::DropFinished { operation: None },
            DndEvent::DropDisabled
        ]
    );
}
