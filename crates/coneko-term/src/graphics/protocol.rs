//! APC `G...` の読み取りと、命令の実行。

use super::animation::{self, FrameKeys};
use super::place::{self, Effect, PlaceKeys};
use super::{Graphics, Image, ImageFormat};

/// 幅・高さの上限（kitty と同じ）。
const MAX_DIMENSION: u32 = 10000;

/// 分割送信の途中の状態。
#[derive(Debug)]
pub(crate) struct Pending {
    /// 最初の塊の命令。`None` は、読み捨てている（最初の塊でもう返事をした）。
    cmd: Option<Command>,
    /// base64 のまま溜める。
    b64: Vec<u8>,
}

#[derive(Debug)]
struct Command {
    action: Action,
    format: ImageFormat,
    compressed: bool,
    width: u32,
    height: u32,
    id: u32,
    number: u32,
    quiet: u32,
    /// `a=T` の置き方。
    place: PlaceKeys,
    /// `a=f` のキー。
    frame: FrameKeys,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Transmit,
    /// `a=T`: 送って置く。
    TransmitPlace,
    Query,
    /// `a=f`: アニメーションのコマ。
    Frame,
}

pub(super) type Fail = (&'static str, &'static str);

pub(super) const EINVAL: &str = "EINVAL";

/// 制御データ（`,` で区切った `key=value`）を分ける。
fn split_keys(control: &[u8]) -> Vec<(u8, &[u8])> {
    control
        .split(|&b| b == b',')
        .filter_map(|kv| {
            let (k, v) = kv.split_at_checked(1)?;
            let v = v.strip_prefix(b"=")?;
            Some((k[0], v))
        })
        .collect()
}

pub(super) fn last<'a>(keys: &[(u8, &'a [u8])], key: u8) -> Option<&'a [u8]> {
    keys.iter().rev().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// 符号つきの数（`z` 用）。
pub(super) fn signed(v: &[u8]) -> Option<i32> {
    let (neg, digits) = match v.strip_prefix(b"-") {
        Some(rest) => (true, rest),
        None => (false, v),
    };
    let n = i64::from(number(digits)?.min(i32::MAX as u32 + 1));
    i32::try_from(if neg { -n } else { n }).ok()
}

fn number(v: &[u8]) -> Option<u32> {
    if v.is_empty() || v.len() > 10 || !v.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(v).ok()?.parse().ok()
}

/// 数のキー。無ければ `Ok(None)`、数でなければ `Err`。
pub(super) fn num_key(keys: &[(u8, &[u8])], key: u8) -> Result<Option<u32>, ()> {
    match last(keys, key) {
        None => Ok(None),
        Some(v) => number(v).map(Some).ok_or(()),
    }
}

/// 数のキー。読めなければ 0（返事の宛先を決めるために、ゆるく読む）。
pub(super) fn lenient(keys: &[(u8, &[u8])], key: u8) -> u32 {
    last(keys, key).and_then(number).unwrap_or(0)
}

/// 最初の塊の命令を読む。`Err` は返事に使うので ID と `q` は先に読んである。
fn parse_command(keys: &[(u8, &[u8])]) -> Result<Option<Command>, (Reply, Fail)> {
    let reply = Reply::from_keys(keys);
    let fail = |m: Fail| (reply, m);
    let action = match last(keys, b'a') {
        None | Some(b"t") => Action::Transmit,
        Some(b"f") => Action::Frame,
        Some(b"T") => Action::TransmitPlace,
        Some(b"q") => Action::Query,
        // 知らない動作は捨てる（a=p と a=d は呼ぶ前に分けてある）。
        Some(_) => return Ok(None),
    };
    if reply.id != 0 && reply.number != 0 {
        return Err(fail((EINVAL, "cannot specify both i and I")));
    }
    match last(keys, b't') {
        None | Some(b"d") => {}
        Some(b"f") | Some(b"t") | Some(b"s") => {
            return Err(fail((
                "EBADF",
                "this transmission medium is not supported, use t=d",
            )))
        }
        Some(_) => return Err(fail((EINVAL, "unknown transmission medium"))),
    }
    let format = match num_key(keys, b'f') {
        Ok(None) | Ok(Some(32)) => ImageFormat::Rgba,
        Ok(Some(24)) => ImageFormat::Rgb,
        Ok(Some(100)) => ImageFormat::Png,
        _ => return Err(fail((EINVAL, "unknown format"))),
    };
    let compressed = match last(keys, b'o') {
        None => false,
        Some(b"z") => true,
        Some(_) => return Err(fail((EINVAL, "unknown compression"))),
    };
    let size = |key| num_key(keys, key).map_err(|_| fail((EINVAL, "invalid size")));
    let width = size(b's')?.unwrap_or(0);
    let height = size(b'v')?.unwrap_or(0);
    let place = if action == Action::TransmitPlace {
        place::parse_keys(keys).map_err(fail)?
    } else {
        PlaceKeys::default()
    };
    let frame = if action == Action::Frame {
        animation::parse_frame_keys(keys).map_err(fail)?
    } else {
        FrameKeys::default()
    };
    Ok(Some(Command {
        action,
        format,
        compressed,
        width,
        height,
        id: reply.id,
        number: reply.number,
        quiet: reply.quiet,
        place,
        frame,
    }))
}

/// 返事の宛先。
#[derive(Clone, Copy, Debug)]
pub(super) struct Reply {
    pub(super) id: u32,
    pub(super) number: u32,
    pub(super) quiet: u32,
    /// 配置の番号（`p=`）。0 は返事に書かない。
    pub(super) placement: u32,
}

impl Reply {
    /// キーから宛先を読む。読めない値は 0。
    pub(super) fn from_keys(keys: &[(u8, &[u8])]) -> Reply {
        Reply {
            id: lenient(keys, b'i'),
            number: lenient(keys, b'I'),
            quiet: lenient(keys, b'q'),
            placement: lenient(keys, b'p'),
        }
    }

    /// 返事のバイト列を `out` に入れる。`i=` も `I=` も無ければ返事をしない。
    pub(super) fn send(&self, assigned_id: u32, result: Result<(), Fail>, out: &mut Vec<u8>) {
        if self.id == 0 && self.number == 0 {
            return;
        }
        let quiet = match result {
            Ok(()) => self.quiet >= 1,
            Err(_) => self.quiet >= 2,
        };
        if quiet {
            return;
        }
        let id = if self.id != 0 { self.id } else { assigned_id };
        let mut keys = Vec::new();
        if id != 0 {
            keys.push(format!("i={id}"));
        }
        if self.number != 0 {
            keys.push(format!("I={}", self.number));
        }
        if self.placement != 0 {
            keys.push(format!("p={}", self.placement));
        }
        let mut s = format!("\x1b_G{};", keys.join(","));
        match result {
            Ok(()) => s.push_str("OK"),
            Err((code, msg)) => s.push_str(&format!("{code}:{msg}")),
        }
        s.push_str("\x1b\\");
        out.extend_from_slice(s.as_bytes());
    }
}

fn decode_base64(src: &[u8]) -> Option<Vec<u8>> {
    let mut src = src;
    while let Some(rest) = src.strip_suffix(b"=") {
        src = rest;
    }
    if src.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(src.len() / 4 * 3 + 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in src {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// PNG の IHDR から幅と高さを読む。
fn png_size(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 24 || &data[..8] != b"\x89PNG\r\n\x1a\n" || &data[12..16] != b"IHDR" {
        return None;
    }
    let be = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    Some((be(&data[16..20]), be(&data[20..24])))
}

impl Graphics {
    /// APC の本体（`ESC _` のあとから ST の前まで）を処理する。返事は `out` に入れる。
    pub(crate) fn apc(&mut self, data: &[u8], out: &mut Vec<u8>) {
        let Some(body) = data.strip_prefix(b"G") else {
            return;
        };
        let (control, payload) = match body.iter().position(|&b| b == b';') {
            Some(p) => (&body[..p], &body[p + 1..]),
            None => (body, &body[body.len()..]),
        };
        let keys = split_keys(control);
        let more = lenient(&keys, b'm') == 1;

        if let Some(mut p) = self.pending.take() {
            // 分割送信の続き。使うのは m だけ（q は最初の塊のものを使う）。
            self.push_chunk(&mut p, payload, out);
            if more {
                self.pending = Some(p);
            } else if let Some(cmd) = p.cmd {
                self.finish(cmd, &p.b64, out);
            }
            return;
        }

        match last(&keys, b'a') {
            Some(b"p") => {
                self.place_command(&keys, Reply::from_keys(&keys), out);
                return;
            }
            Some(b"d") => {
                self.delete_command(&keys);
                return;
            }
            Some(b"a") => {
                self.animate_command(&keys, out);
                return;
            }
            Some(b"c") => {
                self.compose_command(&keys, out);
                return;
            }
            _ => {}
        }

        let cmd = match parse_command(&keys) {
            Ok(Some(c)) => c,
            Ok(None) => {
                if more {
                    self.pending = Some(Pending {
                        cmd: None,
                        b64: Vec::new(),
                    });
                }
                return;
            }
            Err((reply, fail)) => {
                reply.send(0, Err(fail), out);
                if more {
                    self.pending = Some(Pending {
                        cmd: None,
                        b64: Vec::new(),
                    });
                }
                return;
            }
        };
        if more {
            let mut p = Pending {
                cmd: Some(cmd),
                b64: Vec::new(),
            };
            self.push_chunk(&mut p, payload, out);
            self.pending = Some(p);
        } else {
            self.finish(cmd, payload, out);
        }
    }

    /// 塊を足す。溜まった大きさが上限を超えたら、エラーを返して、続きは読み捨てる。
    fn push_chunk(&self, p: &mut Pending, payload: &[u8], out: &mut Vec<u8>) {
        let Some(cmd) = &p.cmd else { return };
        p.b64.extend_from_slice(payload);
        if p.b64.len() / 4 * 3 > self.quota {
            cmd.reply().send(0, Err(too_big()), out);
            p.cmd = None;
            p.b64 = Vec::new();
        }
    }

    fn finish(&mut self, cmd: Command, b64: &[u8], out: &mut Vec<u8>) {
        let reply = cmd.reply();
        if cmd.action == Action::Frame {
            let frame = cmd.frame;
            let result = match self.build(cmd, 0, b64) {
                Ok(Some(data)) => self.add_frame(&reply, frame, data),
                Ok(None) => return,
                Err(f) => Err((0, f)),
            };
            match result {
                Ok(id) => reply.send(id, Ok(()), out),
                Err((id, f)) => reply.send(id, Err(f), out),
            }
            return;
        }
        let place = cmd.action == Action::TransmitPlace;
        let place_keys = cmd.place;
        let assigned = if cmd.id == 0 && cmd.action == Action::Transmit {
            self.fresh_id()
        } else {
            cmd.id
        };
        match self.build(cmd, assigned, b64) {
            Ok(image) => {
                if let Some(image) = image {
                    if !self.insert(image) {
                        reply.send(assigned, Err(too_big()), out);
                        return;
                    }
                }
                if place {
                    let Some(image) = self.get(assigned) else {
                        return;
                    };
                    match place::resolve(
                        image,
                        &place_keys,
                        place_keys.placement_id(),
                        self.cell_size,
                    ) {
                        Ok(placement) if place_keys.is_virtual() => self.add_virtual(placement),
                        Ok(placement) => self.effects.push(Effect::Place {
                            placement,
                            move_cursor: !place_keys.no_move(),
                        }),
                        Err(f) => {
                            reply.send(assigned, Err(f), out);
                            return;
                        }
                    }
                }
                reply.send(assigned, Ok(()), out);
            }
            Err(f) => reply.send(assigned, Err(f), out),
        }
    }

    /// 検査して画像にする。問い合わせなら `Ok(None)`。
    fn build(&self, cmd: Command, id: u32, b64: &[u8]) -> Result<Option<Image>, Fail> {
        if b64.is_empty() {
            return Err(("ENODATA", "no image data"));
        }
        if b64.len() / 4 * 3 > self.quota {
            return Err(too_big());
        }
        let data = decode_base64(b64).ok_or((EINVAL, "invalid base64 data"))?;
        let (mut width, mut height) = (cmd.width, cmd.height);
        match cmd.format {
            ImageFormat::Png => {
                if !cmd.compressed {
                    let (w, h) = png_size(&data).ok_or((EINVAL, "invalid PNG data"))?;
                    (width, height) = (w, h);
                }
            }
            ImageFormat::Rgb | ImageFormat::Rgba => {
                if width == 0 || height == 0 {
                    return Err((EINVAL, "width and height (s and v) are required"));
                }
                if !cmd.compressed {
                    let bpp: usize = if cmd.format == ImageFormat::Rgb { 3 } else { 4 };
                    let expected = (width as usize)
                        .checked_mul(height as usize)
                        .and_then(|n| n.checked_mul(bpp));
                    if expected != Some(data.len()) {
                        return Err((
                            EINVAL,
                            "data length does not match width, height and format",
                        ));
                    }
                }
            }
        }
        if width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err((EINVAL, "image dimensions too large"));
        }
        if cmd.action == Action::Query {
            return Ok(None);
        }
        Ok(Some(Image {
            id,
            number: (cmd.number != 0).then_some(cmd.number),
            format: cmd.format,
            compressed: cmd.compressed,
            width,
            height,
            data,
            serial: 0,
            animation: Default::default(),
        }))
    }
}

impl Command {
    fn reply(&self) -> Reply {
        Reply {
            id: self.id,
            number: self.number,
            quiet: self.quiet,
            placement: if self.action == Action::TransmitPlace {
                self.place.placement_id()
            } else {
                0
            },
        }
    }
}

fn too_big() -> Fail {
    ("EFBIG", "image is larger than the storage quota")
}
