//! kitty の Program Status Protocol（OSC 7501、改訂 0.4）のテスト。
//! 例は仕様（<https://www.superlogical.com/rex/docs/build/program-status>）から。

use super::*;

fn term() -> Terminal {
    Terminal::new(3, 20)
}

fn w(t: &mut Terminal, s: &str) {
    t.write(s.as_bytes());
}

fn report(t: &mut Terminal, body: &str) {
    w(t, &format!("\x1b]7501;{body}\x1b\\"));
}

fn changes(t: &mut Terminal) -> Vec<ProgramStatusChange> {
    t.take_events()
        .into_iter()
        .filter_map(|e| match e {
            Event::ProgramStatus(c) => Some(*c),
            _ => None,
        })
        .collect()
}

fn status(t: &Terminal, id: &str) -> Option<ProgramStatus> {
    t.program_status()
        .into_iter()
        .find(|(i, _)| i == id)
        .map(|(_, s)| s)
}

fn b64(s: &str) -> String {
    let t: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::new();
    for c in s.as_bytes().chunks(3) {
        let n = c
            .iter()
            .enumerate()
            .fold(0u32, |a, (i, &b)| a | (b as u32) << (16 - 8 * i));
        for i in 0..=c.len() {
            o.push(t[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
        for _ in c.len()..3 {
            o.push('=');
        }
    }
    o
}

#[test]
fn spec_example_working_with_progress() {
    let mut t = term();
    // 仕様の例: printf '\e]7501;state=working:app=rsync:progress=40\e\\'
    report(&mut t, "state=working:app=rsync:progress=40");
    let c = changes(&mut t);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].id, "");
    let s = c[0].status.clone().unwrap();
    assert_eq!(s.state, ProgramState::Working);
    assert_eq!(s.app.as_deref(), Some("rsync"));
    assert_eq!(s.progress, Some(40));
    assert_eq!(status(&t, ""), Some(s));
}

#[test]
fn spec_example_blocked_with_message() {
    let mut t = term();
    // 仕様の例（terraform）。メッセージは詰め物つきの base64。
    report(
        &mut t,
        "state=blocked:kind=permission:app=terraform:msg=QXBwbHkgMyB0byBhZGQsIDEgdG8gY2hhbmdlLCAwIHRvIGRlc3Ryb3k/",
    );
    let s = status(&t, "").unwrap();
    assert_eq!(s.state, ProgramState::Blocked);
    assert_eq!(s.kind, Some(ProgramKind::Permission));
    assert_eq!(
        s.msg.as_deref(),
        Some("Apply 3 to add, 1 to change, 0 to destroy?")
    );
}

#[test]
fn padding_is_optional() {
    let mut t = term();
    report(&mut t, "state=done:msg=VXBncmFkZWQgMTIgcGFja2FnZXM");
    assert_eq!(
        status(&t, "").unwrap().msg.as_deref(),
        Some("Upgraded 12 packages")
    );
    report(&mut t, "state=done:msg=VXBncmFkZWQgMTIgcGFja2FnZXM=");
    assert_eq!(
        status(&t, "").unwrap().msg.as_deref(),
        Some("Upgraded 12 packages")
    );
}

#[test]
fn each_report_replaces_the_record() {
    let mut t = term();
    report(&mut t, "state=working:app=brew:progress=10");
    report(&mut t, "state=working");
    let s = status(&t, "").unwrap();
    assert_eq!(s.app, None);
    assert_eq!(s.progress, None);
    assert_eq!(t.program_status().len(), 1);
}

#[test]
fn unknown_state_or_no_state_is_ignored() {
    let mut t = term();
    report(&mut t, "state=sleeping");
    report(&mut t, "app=x");
    report(&mut t, "state=");
    assert!(t.program_status().is_empty());
    assert!(changes(&mut t).is_empty());
}

#[test]
fn unknown_keys_and_malformed_pairs_are_skipped() {
    let mut t = term();
    report(
        &mut t,
        "state=working:future=1:novalue:=x:app=a b:Bad=1:progress=5",
    );
    let s = status(&t, "").unwrap();
    assert_eq!(s.state, ProgramState::Working);
    assert_eq!(s.progress, Some(5));
}

#[test]
fn last_value_wins_and_whitespace_is_trimmed() {
    let mut t = term();
    report(&mut t, " state = idle : state = working ");
    assert_eq!(status(&t, "").unwrap().state, ProgramState::Working);
}

#[test]
fn kind_only_with_blocked_and_unknown_kind_is_absent() {
    let mut t = term();
    report(&mut t, "state=working:kind=auth");
    assert_eq!(status(&t, "").unwrap().kind, None);
    report(&mut t, "state=blocked:kind=weird");
    assert_eq!(status(&t, "").unwrap().kind, None);
    report(&mut t, "state=blocked:kind=question");
    assert_eq!(status(&t, "").unwrap().kind, Some(ProgramKind::Question));
}

#[test]
fn progress_rules() {
    let mut t = term();
    for (body, want) in [
        ("state=working:progress=0", Some(0)),
        ("state=working:progress=100", Some(100)),
        ("state=working:progress=101", None),
        ("state=working:progress=-1", None),
        ("state=working:progress=4.5", None),
        ("state=working:progress=", None),
        ("state=blocked:progress=7", Some(7)),
        ("state=done:progress=7", None),
        ("state=idle:progress=7", None),
    ] {
        report(&mut t, body);
        assert_eq!(status(&t, "").unwrap().progress, want, "{body}");
    }
}

#[test]
fn app_outside_its_character_set_is_absent() {
    let mut t = term();
    report(&mut t, "state=working:app=a/b");
    assert_eq!(status(&t, "").unwrap().app, None);
    report(&mut t, "state=working:app=claude-code");
    assert_eq!(status(&t, "").unwrap().app.as_deref(), Some("claude-code"));
    // 長さが上限（32）を超えると報告ごと捨てる
    let long = "a".repeat(33);
    report(&mut t, &format!("state=done:app={long}"));
    assert_eq!(status(&t, "").unwrap().state, ProgramState::Working);
}

#[test]
fn text_with_control_characters_discards_the_whole_report() {
    let mut t = term();
    report(&mut t, "state=working");
    changes(&mut t);
    report(&mut t, &format!("state=done:msg={}", b64("a\nb")));
    report(&mut t, &format!("state=done:title={}", b64("a\tb")));
    report(&mut t, &format!("state=done:msg={}", b64("a\u{85}b")));
    report(&mut t, &format!("state=done:msg={}", b64("a\u{7f}")));
    report(&mut t, "state=done:msg=A");
    assert_eq!(status(&t, "").unwrap().state, ProgramState::Working);
    assert!(changes(&mut t).is_empty());
    report(
        &mut t,
        &format!("state=done:msg={}:title={}", b64("日本語"), b64("題")),
    );
    let s = status(&t, "").unwrap();
    assert_eq!(s.msg.as_deref(), Some("日本語"));
    assert_eq!(s.title.as_deref(), Some("題"));
}

#[test]
fn limits_discard_the_whole_report() {
    let mut t = term();
    report(
        &mut t,
        &format!("state=done:msg={}", b64(&"x".repeat(2048))),
    );
    assert_eq!(status(&t, "").unwrap().msg.as_ref().unwrap().len(), 2048);
    report(
        &mut t,
        &format!("state=working:msg={}", b64(&"x".repeat(2049))),
    );
    assert_eq!(status(&t, "").unwrap().state, ProgramState::Done);
    report(
        &mut t,
        &format!("state=working:title={}", b64(&"x".repeat(192))),
    );
    assert_eq!(status(&t, "").unwrap().state, ProgramState::Working);
    report(
        &mut t,
        &format!("state=done:title={}", b64(&"x".repeat(193))),
    );
    assert_eq!(status(&t, "").unwrap().state, ProgramState::Working);
    // 鍵が 16 バイトを超える
    report(&mut t, "state=done:abcdefghijklmnopq=1");
    assert_eq!(status(&t, "").unwrap().state, ProgramState::Working);
}

#[test]
fn sequence_over_4096_bytes_is_discarded() {
    let mut t = term();
    let pad = "x=".to_string() + &"a".repeat(4090);
    report(&mut t, &format!("state=done:{pad}"));
    assert!(t.program_status().is_empty());
}

#[test]
fn spec_example_several_records() {
    let mut t = term();
    report(
        &mut t,
        &format!("state=working:app=deploy:msg={}", b64("Deploying v2.4.1")),
    );
    report(
        &mut t,
        &format!(
            "state=working:id=us-east:title={}:progress=40",
            b64("US East")
        ),
    );
    report(&mut t, "state=blocked:kind=permission:id=eu-west");
    assert_eq!(t.program_status().len(), 3);
    // 子の記録は、app を根から引き継ぐ
    assert_eq!(
        status(&t, "us-east").unwrap().app.as_deref(),
        Some("deploy")
    );
    assert_eq!(status(&t, "").unwrap().state, ProgramState::Working);
    changes(&mut t);
    report(&mut t, "state=clear:id=us-east");
    report(&mut t, "state=clear:id=eu-west");
    let c = changes(&mut t);
    assert_eq!(
        c.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        ["us-east", "eu-west"]
    );
    assert!(c.iter().all(|c| c.status.is_none()));
    assert_eq!(t.program_status().len(), 1);
}

#[test]
fn app_is_inherited_from_the_nearest_ancestor() {
    let mut t = term();
    report(&mut t, "state=working:app=top");
    report(&mut t, "state=working:id=a:app=mid");
    report(&mut t, "state=working:id=a/b/c");
    report(&mut t, "state=working:id=x/y");
    assert_eq!(status(&t, "a/b/c").unwrap().app.as_deref(), Some("mid"));
    assert_eq!(status(&t, "x/y").unwrap().app.as_deref(), Some("top"));
}

#[test]
fn clear_removes_the_record_and_everything_beneath() {
    let mut t = term();
    for id in ["a", "a/b", "a/b/c", "ab", "z"] {
        report(&mut t, &format!("state=working:id={id}"));
    }
    report(&mut t, "state=clear:id=a");
    let ids: Vec<String> = t.program_status().into_iter().map(|(i, _)| i).collect();
    assert_eq!(ids, ["ab", "z"]);
}

#[test]
fn clear_without_id_removes_everything() {
    let mut t = term();
    report(&mut t, "state=working");
    report(&mut t, "state=working:id=a");
    changes(&mut t);
    report(&mut t, "state=clear");
    assert!(t.program_status().is_empty());
    assert_eq!(changes(&mut t).len(), 2);
}

#[test]
fn invalid_id_is_ignored_not_the_root() {
    let mut t = term();
    report(&mut t, "state=working:app=root");
    for id in [
        "a//b",
        "/a",
        "a/",
        "a b",
        &"s".repeat(33),
        "1/2/3/4/5/6/7/8/9",
    ] {
        report(&mut t, &format!("state=done:id={id}"));
    }
    report(&mut t, "state=done:id=");
    assert_eq!(t.program_status().len(), 1);
    assert_eq!(status(&t, "").unwrap().state, ProgramState::Working);
    // 上限ちょうど（8 段）は通る
    report(&mut t, "state=done:id=1/2/3/4/5/6/7/8");
    assert_eq!(t.program_status().len(), 2);
}

#[test]
fn id_over_128_bytes_is_ignored() {
    let mut t = term();
    let seg = "a".repeat(32);
    let ok = format!("{seg}/{seg}/{seg}/{}", "b".repeat(31)); // 32*3 + 3 + 31 = 130 は超える
    report(&mut t, &format!("state=working:id={ok}"));
    assert!(t.program_status().is_empty());
}

#[test]
fn record_cap_evicts_the_least_recently_updated() {
    let mut t = term();
    for i in 0..256 {
        report(&mut t, &format!("state=working:id=r{i}"));
    }
    // r0 を更新して最新にする
    report(&mut t, "state=done:id=r0");
    changes(&mut t);
    report(&mut t, "state=working:id=extra");
    let c = changes(&mut t);
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].id, "extra");
    assert_eq!((c[1].id.as_str(), c[1].status.is_none()), ("r1", true));
    assert_eq!(t.program_status().len(), 256);
    assert!(status(&t, "r0").is_some());
}

#[test]
fn feature_detection() {
    let mut t = term();
    w(&mut t, "\x1b]7501;?\x1b\\");
    assert_eq!(
        String::from_utf8(t.take_output()).unwrap(),
        "\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\"
    );
    // BEL で終えた問い合わせには BEL で答える
    w(&mut t, "\x1b]7501;?\x07");
    assert!(String::from_utf8(t.take_output())
        .unwrap()
        .ends_with('\x07'));
    // 報告は何も返さない
    report(&mut t, "state=working");
    assert!(t.take_output().is_empty());
}

#[test]
fn survives_screen_switch_and_soft_reset_but_not_ris() {
    let mut t = term();
    report(&mut t, "state=done");
    w(&mut t, "\x1b[?1049h\x1b[?1049l\x1b[!p");
    assert_eq!(t.program_status().len(), 1);
    changes(&mut t);
    w(&mut t, "\x1bc");
    assert!(t.program_status().is_empty());
    assert_eq!(changes(&mut t).len(), 1);
}

#[test]
fn shell_prompt_drops_running_idle_and_blocked_but_not_finished() {
    let mut t = term();
    report(&mut t, "state=working:id=w");
    report(&mut t, "state=blocked:id=b");
    report(&mut t, "state=idle:id=i");
    report(&mut t, "state=done:id=d");
    report(&mut t, "state=error:id=e");
    changes(&mut t);
    w(&mut t, "\x1b]133;A\x1b\\");
    let ids: Vec<String> = t.program_status().into_iter().map(|(i, _)| i).collect();
    assert_eq!(ids, ["d", "e"]);
    assert_eq!(changes(&mut t).len(), 3);
    // 他の 133 は何もしない
    report(&mut t, "state=working:id=w");
    w(&mut t, "\x1b]133;C\x1b\\");
    assert_eq!(t.program_status().len(), 3);
}

#[test]
fn process_exit_is_told_by_the_host() {
    let mut t = term();
    report(&mut t, "state=working:id=w");
    report(&mut t, "state=error:id=e");
    t.program_status_drop_running();
    let ids: Vec<String> = t.program_status().into_iter().map(|(i, _)| i).collect();
    assert_eq!(ids, ["e"]);
}

#[test]
fn done_and_error_are_dropped_on_focus_or_key() {
    let mut t = term();
    report(&mut t, "state=done:id=d");
    report(&mut t, "state=working:id=w");
    t.set_focus(false);
    assert_eq!(t.program_status().len(), 2);
    t.set_focus(true);
    let ids: Vec<String> = t.program_status().into_iter().map(|(i, _)| i).collect();
    assert_eq!(ids, ["w"]);
    report(&mut t, "state=error:id=e");
    t.program_status_key_pressed();
    let ids: Vec<String> = t.program_status().into_iter().map(|(i, _)| i).collect();
    assert_eq!(ids, ["w"]);
}

#[test]
fn registered_osc_7501_goes_to_the_host_instead() {
    let mut t = term();
    t.register_osc(7501);
    report(&mut t, "state=working");
    assert!(t.program_status().is_empty());
}

#[test]
fn osc_133_registered_by_the_host_is_not_handled_here() {
    let mut t = term();
    t.register_osc(133);
    report(&mut t, "state=working");
    w(&mut t, "\x1b]133;A\x1b\\");
    assert_eq!(t.program_status().len(), 1);
}
