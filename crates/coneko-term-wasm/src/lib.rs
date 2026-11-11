//! coneko-term を WebAssembly にするための wasm-bindgen の層。中身は `coneko-term` に置く。
//!
//! 画面の中身は、WASM のメモリにあるバッファー（[`WasmTerminal::cells_ptr`]）に 1 行ずつ
//! 書き出す。TypeScript は `memory.buffer` から直接読む。

use coneko_term::mouse::{MouseAction, MouseButton, MouseEvent};
use coneko_term::screen::cell::{Cell, Color, Content, UnderlineStyle};
use coneko_term::screen::Line;
use coneko_term::selection::{self, Pos};
use coneko_term::terminal::{
    ClipboardItem, ClipboardLocation, ClipboardReadError, ClipboardReadRequest,
    ClipboardWriteError, DndError, DndEvent, DndLocation, DndOperation, DndOperations,
    DragEntryForm, DragImageFormat, DropForm, DropRequest, FileCompression, FileTransferError,
    FileTransferEvent, FileTransferListing, FileTransmission, FileType, Notification,
    NotificationSupport, Theme, When,
};
use coneko_term::{Event, Terminal};
use wasm_bindgen::prelude::*;

/// このライブラリの版を返す。
#[wasm_bindgen]
pub fn version() -> String {
    coneko_term::version().to_string()
}

/// セル 1 つが使う `u32` の数。1 つのセルは次の並び:
///
/// 0. 字。`0` は空、`CLUSTER_FLAG` つきは `cluster(番号)`、それ以外は Unicode のコードポイント。
/// 1. 字が使うセルの数（全角の右半分は 0）。
/// 2. 前景色。3. 背景色。色は上位 8 ビットが種類（0 既定、1 パレット、2 RGB）、下位 24 ビットが値。
/// 4. ビット 0 から 6 が属性（太字、薄字、斜体、点滅、反転、不可視、取り消し線）、
///    ビット 8 から 10 が下線の形（0 なし、1 1 本、2 2 本、3 波、4 点、5 破）。
/// 5. 下線の色。
/// 6. 複数のセルにまたがる字（OSC 66）の一部なら、下位 16 ビットが `sized_info`・`sized_text` の番号 + 1、
///    ビット 16 から 23 が四角の左上から右へ何セル目か、ビット 24 から 31 が下へ何セル目か。それ以外は 0。
///    この字のセルは、字（0）を空にし、幅を 1 にして書き出す（四角は `sized_info` が持つ）。
pub const CELL_STRIDE: usize = 7;

/// 画像の配置 1 つが使う数（`WasmTerminal::placements` の並び）の数。並びは次のとおり:
///
/// 0. 左上のセルの行（見えている画面の一番上を 0 とする。スクロールバックの中は負）。
/// 1. 画像の ID。2. 配置の番号（0 は無い）。3. 左上のセルの列。
/// 4 から 7. 元の画像の使う部分の左端・上端・幅・高さ（画素。幅と高さが 0 なら端まで）。
/// 8, 9. 左上のセルの中でのずれ（横・縦、画素）。10, 11. 使うセルの列数・行数。
/// 12. 重ねる順（`z`）。13. 画素のまま置くなら 1、セルいっぱいに引き伸ばすなら 0。
/// 14, 15. 画素で決めた描く大きさ（幅・高さ）。0 なら 13 と `cols` x `rows` で決める
/// （Unicode placeholder の配置が使う）。
pub const PLACEMENT_STRIDE: usize = 16;

/// クラスター（結合文字を含む書記素）のセルの字に立てるビット。残りのビットが `cluster(番号)` の番号。
pub const CLUSTER_FLAG: u32 = 0x8000_0000;

fn color_word(c: Color) -> u32 {
    match c {
        Color::Default => 0,
        Color::Palette(i) => (1 << 24) | i as u32,
        Color::Rgb(r, g, b) => (2 << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32,
        _ => 0,
    }
}

fn underline_code(s: UnderlineStyle) -> u32 {
    match s {
        UnderlineStyle::None => 0,
        UnderlineStyle::Single => 1,
        UnderlineStyle::Double => 2,
        UnderlineStyle::Curly => 3,
        UnderlineStyle::Dotted => 4,
        UnderlineStyle::Dashed => 5,
        _ => 1,
    }
}

fn location_code(l: ClipboardLocation) -> u8 {
    match l {
        ClipboardLocation::Clipboard => 0,
        ClipboardLocation::Primary => 1,
    }
}

/// OSC 72 のイベント（`WasmEvent` の種類 10）を、数・文字列・バイト列にして持つ。
/// `dnd_kind` ごとの並びは次のとおり（`ints` / `strings` / `blobs`）。
///
/// - 0 `DropEnabled`: / MIME の種類 /
/// - 1 `DropDisabled`
/// - 2 `DropResponse`: `[操作 (0 なし、1 copy、2 move), MIME の一覧があるか]` / MIME の種類 /
/// - 3 `DropRequest`: `[種類 (0 Data、1 Entry、2 DirEntry、3 DirClose), a, b]`。Data は `[0, index, 0]`、
///   Entry は `[1, index, sub_index]`、DirEntry は `[2, handle, index]`、DirClose は `[3, handle, 0]`
/// - 4 `DropFinished`: `[操作]`
/// - 5 `DragEnabled`: `[機械の ID があるか]` / `[機械の ID]` /
/// - 6 `DragDisabled`
/// - 7 `DragStart`: `[許す操作, 始める画像の番号 (無ければ -1), MIME の数 n, 画像の数 m,
///   先に送ったデータがあるか × n, (形式 0 Rgb・1 Rgba・2 Png・3 Text, 幅, 高さ, 不透明度) × m,
///   機械の ID があるか]` / `MIME の種類 × n, [機械の ID]` / `データ × n, 画像のデータ × m`
/// - 8 `DragImage`: `[番号 (外すなら -1)]`
/// - 9 `DragData`: `[番号]` / / `[データ]`
/// - 10 `DragDataError`: `[番号]` / `[記号名, 説明]` /
/// - 11 `DragEntry`: `[index, sub_index, parent, 形 (0 File、1 Symlink、2 Directory), handle]` / / `[データ]`
/// - 12 `DragCanceled`
/// - 13 `DragAborted`: / `[記号名, 説明]` /
#[derive(Default)]
struct DndPayload {
    kind: u8,
    ints: Vec<i32>,
    strings: Vec<String>,
    blobs: Vec<Vec<u8>>,
}

fn op_code(op: Option<DndOperation>) -> i32 {
    match op {
        None => 0,
        Some(DndOperation::Copy) => 1,
        Some(DndOperation::Move) => 2,
    }
}

fn dnd_payload(ev: DndEvent) -> DndPayload {
    let mut p = DndPayload::default();
    match ev {
        DndEvent::DropEnabled { mimes } => {
            p.kind = 0;
            p.strings = mimes;
        }
        DndEvent::DropDisabled => p.kind = 1,
        DndEvent::DropResponse { operation, mimes } => {
            p.kind = 2;
            p.ints = vec![op_code(operation), mimes.is_some() as i32];
            p.strings = mimes.unwrap_or_default();
        }
        DndEvent::DropRequest(r) => {
            p.kind = 3;
            p.ints = match r {
                DropRequest::Data { index } => vec![0, index as i32, 0],
                DropRequest::Entry { index, sub_index } => vec![1, index as i32, sub_index as i32],
                DropRequest::DirEntry { handle, index } => vec![2, handle as i32, index as i32],
                DropRequest::DirClose { handle } => vec![3, handle as i32, 0],
            };
        }
        DndEvent::DropFinished { operation } => {
            p.kind = 4;
            p.ints = vec![op_code(operation)];
        }
        DndEvent::DragEnabled { machine_id } => {
            p.kind = 5;
            p.ints = vec![machine_id.is_some() as i32];
            p.strings = machine_id.into_iter().collect();
        }
        DndEvent::DragDisabled => p.kind = 6,
        DndEvent::DragStart(o) => {
            let o = *o;
            p.kind = 7;
            let ops = o.operations.copy as i32 | (o.operations.r#move as i32) << 1;
            p.ints = vec![
                ops,
                o.image.map_or(-1, |i| i as i32),
                o.mimes.len() as i32,
                o.images.len() as i32,
            ];
            p.ints.extend(o.data.iter().map(|d| d.is_some() as i32));
            for img in &o.images {
                p.ints.push(match img.format {
                    DragImageFormat::Rgb => 0,
                    DragImageFormat::Rgba => 1,
                    DragImageFormat::Png => 2,
                    _ => 3,
                });
                p.ints.push(img.width as i32);
                p.ints.push(img.height as i32);
                p.ints.push(img.opacity as i32);
            }
            p.ints.push(o.machine_id.is_some() as i32);
            p.strings = o.mimes;
            p.strings.extend(o.machine_id);
            p.blobs = o.data.into_iter().map(Option::unwrap_or_default).collect();
            p.blobs.extend(o.images.into_iter().map(|i| i.data));
        }
        DndEvent::DragImage { index } => {
            p.kind = 8;
            p.ints = vec![index.map_or(-1, |i| i as i32)];
        }
        DndEvent::DragData { index, data } => {
            p.kind = 9;
            p.ints = vec![index as i32];
            p.blobs = vec![data];
        }
        DndEvent::DragDataError {
            index,
            error,
            description,
        } => {
            p.kind = 10;
            p.ints = vec![index as i32];
            p.strings = vec![error, description];
        }
        DndEvent::DragEntry { key, form, data } => {
            p.kind = 11;
            let (form, handle) = match form {
                DragEntryForm::File => (0, 0),
                DragEntryForm::Symlink => (1, 0),
                DragEntryForm::Directory(h) => (2, h as i32),
            };
            p.ints = vec![
                key.index as i32,
                key.sub_index as i32,
                key.parent as i32,
                form,
                handle,
            ];
            p.blobs = vec![data];
        }
        DndEvent::DragCanceled => p.kind = 12,
        DndEvent::DragAborted { error, description } => {
            p.kind = 13;
            p.strings = vec![error, description];
        }
        _ => p.kind = 255,
    }
    p
}

fn dnd_error(name: &str) -> DndError {
    match name {
        "ENOENT" => DndError::Enoent,
        "EINVAL" => DndError::Einval,
        "EPERM" => DndError::Eperm,
        "EMFILE" => DndError::Emfile,
        "ENOMEM" => DndError::Enomem,
        "EFBIG" => DndError::Efbig,
        "EUNKNOWN" => DndError::Eunknown,
        _ => DndError::Eio,
    }
}

fn dnd_location(x: i32, y: i32, pixel_x: i32, pixel_y: i32) -> DndLocation {
    DndLocation {
        x,
        y,
        pixel_x,
        pixel_y,
    }
}

fn dnd_ops(bits: u8) -> DndOperations {
    DndOperations::from_bits(bits).unwrap_or_default()
}

fn dnd_request(kind: u8, a: u32, b: u32) -> Option<DropRequest> {
    Some(match kind {
        0 => DropRequest::Data { index: a },
        1 => DropRequest::Entry {
            index: a,
            sub_index: b,
        },
        2 => DropRequest::DirEntry {
            handle: a,
            index: b,
        },
        3 => DropRequest::DirClose { handle: a },
        _ => return None,
    })
}

/// プログラムの状態の記録を、`\0` で区切った項目にする。並びは、消えたか（`1`・`0`）、id、状態、
/// `blocked` の理由、進み具合、app、title、msg。無い項目は空。報告の本文に制御文字は入らないので、
/// `\0` で区切ってよい。
fn program_status_fields(id: &str, s: Option<&coneko_term::terminal::ProgramStatus>) -> String {
    let Some(s) = s else {
        return format!("1\0{id}\0\0\0\0\0\0");
    };
    let progress = s.progress.map(|p| p.to_string()).unwrap_or_default();
    let kind = s.kind.map(|k| k.name()).unwrap_or_default();
    format!(
        "0\0{id}\0{}\0{kind}\0{progress}\0{}\0{}\0{}",
        s.state.name(),
        s.app.as_deref().unwrap_or_default(),
        s.title.as_deref().unwrap_or_default(),
        s.msg.as_deref().unwrap_or_default(),
    )
}

/// ターミナルが外へ知らせること 1 つ。`kind` は 0 が BEL、1 が題名、2 がアイコン名、3 が OSC、
/// 4 が通知（OSC 99）、5 が通知を閉じる求め、6 が通知の対応の問い合わせ（`p=?`）、
/// 7 が生きている通知の問い合わせ（`p=alive`）、8 がクリップボードへの書き込み（OSC 5522）、
/// 9 がクリップボードの読み取りの頼み、12 がプログラムの状態の記録の変化（OSC 7501）。5 から 7 は
/// `data` が `i=`（UTF-8、無ければ空）。8 と 9 は
/// `data` が `id`（UTF-8、無ければ空）。
#[wasm_bindgen]
pub struct WasmEvent {
    kind: u8,
    ident: u32,
    data: Vec<u8>,
    notification: Option<Notification>,
    clipboard_location: u8,
    clipboard_list: bool,
    clipboard_mimes: Vec<String>,
    clipboard_data: Vec<Vec<u8>>,
    dnd: DndPayload,
    ft: FtPayload,
}

/// OSC 5113 のイベントの中身。`action` は 0 が send、1 が receive、2 が file、3 が data、4 が end_data、
/// 5 が finish、6 が cancel。
#[derive(Default)]
struct FtPayload {
    action: u8,
    id: String,
    file_id: String,
    name: String,
    bypass: Option<String>,
    quiet: u8,
    file_type: u8,
    compression: u8,
    transmission: u8,
    mtime: Option<i64>,
    permissions: Option<u32>,
    size: Option<u64>,
    parent: Option<String>,
    data: Vec<u8>,
}

fn ft_payload(e: FileTransferEvent) -> Option<FtPayload> {
    let mut p = FtPayload::default();
    let is_send = matches!(e, FileTransferEvent::Send(_));
    match e {
        FileTransferEvent::Send(s) | FileTransferEvent::Receive(s) => {
            p.action = u8::from(!is_send);
            p.id = s.id;
            p.bypass = s.bypass;
            p.quiet = s.quiet;
            p.size = s.paths;
        }
        FileTransferEvent::File(f) => {
            p.action = 2;
            p.id = f.id;
            p.file_id = f.file_id;
            p.name = f.name;
            p.file_type = match f.file_type {
                FileType::Regular => 0,
                FileType::Directory => 1,
                FileType::Symlink => 2,
                FileType::Link => 3,
                _ => return None,
            };
            p.compression = match f.compression {
                FileCompression::None => 0,
                FileCompression::Zlib => 1,
                _ => return None,
            };
            p.transmission = match f.transmission {
                FileTransmission::Simple => 0,
                FileTransmission::Rsync => 1,
                _ => return None,
            };
            p.mtime = f.mtime;
            p.permissions = f.permissions;
            p.size = f.size;
            p.parent = f.parent;
        }
        FileTransferEvent::Data(d) => {
            p.action = if d.end { 4 } else { 3 };
            p.id = d.id;
            p.file_id = d.file_id;
            p.data = d.data;
        }
        FileTransferEvent::Finish { id } => {
            p.action = 5;
            p.id = id;
        }
        FileTransferEvent::Cancel { id } => {
            p.action = 6;
            p.id = id;
        }
        _ => return None,
    }
    Some(p)
}

impl WasmEvent {
    fn new(kind: u8, ident: u32, data: Vec<u8>) -> WasmEvent {
        WasmEvent {
            kind,
            ident,
            data,
            notification: None,
            clipboard_location: 0,
            clipboard_list: false,
            clipboard_mimes: Vec::new(),
            clipboard_data: Vec::new(),
            dnd: DndPayload::default(),
            ft: FtPayload::default(),
        }
    }

    fn with_notification<T: Default>(&self, f: impl FnOnce(&Notification) -> T) -> T {
        self.notification.as_ref().map(f).unwrap_or_default()
    }
}

#[wasm_bindgen]
impl WasmEvent {
    /// 種類（0 が BEL、1 が題名、2 がアイコン名、3 が登録した OSC、4 から 7 が通知）。
    #[wasm_bindgen(getter)]
    pub fn kind(&self) -> u8 {
        self.kind
    }
    /// OSC の番号（種類が 3 のとき）。
    #[wasm_bindgen(getter)]
    pub fn ident(&self) -> u32 {
        self.ident
    }
    /// 中身（題名・アイコン名・OSC のバイト列。題名とアイコン名は UTF-8）。
    #[wasm_bindgen(getter)]
    pub fn data(&self) -> Vec<u8> {
        self.data.clone()
    }
    /// 通知の `i=`（無ければ空）。種類が 4 のとき。以降の `notification_*` も同じ。
    #[wasm_bindgen(getter)]
    pub fn notification_id(&self) -> String {
        self.with_notification(|n| n.id.clone())
    }
    /// 通知の題名。
    #[wasm_bindgen(getter)]
    pub fn notification_title(&self) -> String {
        self.with_notification(|n| n.title.clone())
    }
    /// 通知の本文。
    #[wasm_bindgen(getter)]
    pub fn notification_body(&self) -> String {
        self.with_notification(|n| n.body.clone())
    }
    /// 緊急度（`u`）。指定が無ければ -1。
    #[wasm_bindgen(getter)]
    pub fn notification_urgency(&self) -> i32 {
        self.with_notification(|n| n.urgency.map_or(-1, i32::from))
    }
    /// 押されたときの動き（`a`）。ビット 0 が focus、ビット 1 が report。
    #[wasm_bindgen(getter)]
    pub fn notification_actions(&self) -> u8 {
        self.with_notification(|n| n.actions.focus as u8 | (n.actions.report as u8) << 1)
    }
    /// いつ出すか（`o`）。0 always、1 unfocused、2 invisible。
    #[wasm_bindgen(getter)]
    pub fn notification_when(&self) -> u8 {
        self.with_notification(|n| match n.when {
            When::Unfocused => 1,
            When::Invisible => 2,
            _ => 0,
        })
    }
    /// 閉じられたときに知らせるか（`c=1`）。
    #[wasm_bindgen(getter)]
    pub fn notification_report_close(&self) -> bool {
        self.with_notification(|n| n.report_close)
    }
    /// 消えるまでの時間（`w`、ミリ秒）。指定が無ければ -2。
    #[wasm_bindgen(getter)]
    pub fn notification_timeout(&self) -> i32 {
        self.with_notification(|n| n.timeout.unwrap_or(-2))
    }
    /// アプリケーションの名前（`f`）。無ければ `undefined`。
    #[wasm_bindgen(getter)]
    pub fn notification_app_name(&self) -> Option<String> {
        self.notification.as_ref().and_then(|n| n.app_name.clone())
    }
    /// 音（`s`）。無ければ `undefined`。
    #[wasm_bindgen(getter)]
    pub fn notification_sound(&self) -> Option<String> {
        self.notification.as_ref().and_then(|n| n.sound.clone())
    }
    /// アイコンの画像の ID（`g`）。無ければ `undefined`。
    #[wasm_bindgen(getter)]
    pub fn notification_icon_cache_id(&self) -> Option<String> {
        self.notification
            .as_ref()
            .and_then(|n| n.icon_cache_id.clone())
    }
    /// 通知の種類（`t`）。
    #[wasm_bindgen(getter)]
    pub fn notification_types(&self) -> Vec<String> {
        self.with_notification(|n| n.types.clone())
    }
    /// アイコンの名前（`n`）。
    #[wasm_bindgen(getter)]
    pub fn notification_icon_names(&self) -> Vec<String> {
        self.with_notification(|n| n.icon_names.clone())
    }
    /// ボタンの文字（`p=buttons`）。
    #[wasm_bindgen(getter)]
    pub fn notification_buttons(&self) -> Vec<String> {
        self.with_notification(|n| n.buttons.clone())
    }
    /// クリップボードの場所（種類が 8 か 9 のとき）。0 が主のクリップボード、1 が primary selection。
    #[wasm_bindgen(getter)]
    pub fn clipboard_location(&self) -> u8 {
        self.clipboard_location
    }
    /// 読み取りの頼みが種類の一覧（`.`）か（種類が 9 のとき）。
    #[wasm_bindgen(getter)]
    pub fn clipboard_list(&self) -> bool {
        self.clipboard_list
    }
    /// 書き込みの MIME の種類（種類が 8 のとき）か、読み取りで頼まれた MIME の種類（種類が 9 のとき）。
    #[wasm_bindgen(getter)]
    pub fn clipboard_mimes(&self) -> Vec<String> {
        self.clipboard_mimes.clone()
    }
    /// 書き込みの `index` 番目の MIME の種類のデータ（種類が 8 のとき）。
    pub fn clipboard_item_data(&self, index: usize) -> Vec<u8> {
        self.clipboard_data.get(index).cloned().unwrap_or_default()
    }
    /// OSC 5113 の命令の種類（`WasmEvent` の種類が 11 のとき）。0 が send、1 が receive、2 が file、
    /// 3 が data、4 が end_data、5 が finish、6 が cancel。
    #[wasm_bindgen(getter)]
    pub fn ft_action(&self) -> u8 {
        self.ft.action
    }
    /// セッションの `id`。
    #[wasm_bindgen(getter)]
    pub fn ft_id(&self) -> String {
        self.ft.id.clone()
    }
    /// `fid`（file・data・end_data）。
    #[wasm_bindgen(getter)]
    pub fn ft_file_id(&self) -> String {
        self.ft.file_id.clone()
    }
    /// 戻したパス（file）。
    #[wasm_bindgen(getter)]
    pub fn ft_name(&self) -> String {
        self.ft.name.clone()
    }
    /// 戻した `pw`（send・receive）。
    #[wasm_bindgen(getter)]
    pub fn ft_bypass(&self) -> Option<String> {
        self.ft.bypass.clone()
    }
    /// `q`（send・receive）。
    #[wasm_bindgen(getter)]
    pub fn ft_quiet(&self) -> u8 {
        self.ft.quiet
    }
    /// 0 が regular、1 が directory、2 が symlink、3 が link（file）。
    #[wasm_bindgen(getter)]
    pub fn ft_file_type(&self) -> u8 {
        self.ft.file_type
    }
    /// 0 が圧縮なし、1 が zlib（file）。
    #[wasm_bindgen(getter)]
    pub fn ft_compression(&self) -> u8 {
        self.ft.compression
    }
    /// 0 が simple、1 が rsync（file）。
    #[wasm_bindgen(getter)]
    pub fn ft_transmission(&self) -> u8 {
        self.ft.transmission
    }
    /// `mod`（ナノ秒。file）。
    #[wasm_bindgen(getter)]
    pub fn ft_mtime(&self) -> Option<i64> {
        self.ft.mtime
    }
    /// `prm`（file）。
    #[wasm_bindgen(getter)]
    pub fn ft_permissions(&self) -> Option<u32> {
        self.ft.permissions
    }
    /// `sz`（file。send・receive では頼むパスの数）。
    #[wasm_bindgen(getter)]
    pub fn ft_size(&self) -> Option<f64> {
        self.ft.size.map(|s| s as f64)
    }
    /// `pr`（file）。
    #[wasm_bindgen(getter)]
    pub fn ft_parent(&self) -> Option<String> {
        self.ft.parent.clone()
    }
    /// データ（data・end_data）。
    pub fn ft_data(&self) -> Vec<u8> {
        self.ft.data.clone()
    }
    /// OSC 72 のイベントの種類（`WasmEvent` の種類が 10 のとき）。並びは `DndPayload` の説明のとおり。
    #[wasm_bindgen(getter)]
    pub fn dnd_kind(&self) -> u8 {
        self.dnd.kind
    }
    /// OSC 72 のイベントの数の並び。
    #[wasm_bindgen(getter)]
    pub fn dnd_ints(&self) -> Vec<i32> {
        self.dnd.ints.clone()
    }
    /// OSC 72 のイベントの文字列の並び。
    #[wasm_bindgen(getter)]
    pub fn dnd_strings(&self) -> Vec<String> {
        self.dnd.strings.clone()
    }
    /// OSC 72 のイベントの `index` 番目のバイト列。
    pub fn dnd_blob(&self, index: usize) -> Vec<u8> {
        self.dnd.blobs.get(index).cloned().unwrap_or_default()
    }
}

/// ターミナル。
#[wasm_bindgen]
pub struct WasmTerminal {
    term: Terminal,
    cells: Vec<u32>,
    clusters: Vec<String>,
    sized: Vec<std::sync::Arc<coneko_term::screen::SizedBlock>>,
    events: std::collections::VecDeque<Event>,
    last_wrapped: bool,
    last_input: bool,
    clipboard_reply_items: Vec<ClipboardItem>,
}

#[wasm_bindgen]
impl WasmTerminal {
    /// `rows` 行 `cols` 列で、スクロールバックが `scrollback` 行のターミナル。
    #[wasm_bindgen(constructor)]
    pub fn new(rows: usize, cols: usize, scrollback: usize) -> WasmTerminal {
        let screen = coneko_term::screen::Screen::with_scrollback(rows, cols, scrollback);
        WasmTerminal {
            term: Terminal::with_screen(screen),
            cells: Vec::new(),
            clusters: Vec::new(),
            sized: Vec::new(),
            events: Default::default(),
            last_wrapped: false,
            last_input: false,
            clipboard_reply_items: Vec::new(),
        }
    }

    /// バイト列を書く。
    pub fn write(&mut self, bytes: &[u8]) {
        self.term.write(bytes);
        self.events.extend(self.term.take_events());
    }

    /// 初期状態に戻す。
    pub fn reset(&mut self) {
        self.term.reset();
        self.events.extend(self.term.take_events());
    }

    /// 大きさを変える。
    pub fn resize(&mut self, rows: usize, cols: usize) {
        self.term.resize(rows.max(1), cols.max(1));
    }

    /// 行の数。
    pub fn rows(&self) -> usize {
        self.term.screen().rows()
    }

    /// 列の数。
    pub fn cols(&self) -> usize {
        self.term.screen().cols()
    }

    /// スクロールバックの行の数。
    pub fn scrollback_len(&self) -> usize {
        self.term.screen().scrollback_len()
    }

    /// 次のイベントを取り出す。無ければ `undefined`。
    pub fn next_event(&mut self) -> Option<WasmEvent> {
        self.events.pop_front().map(|e| match e {
            Event::Bell => WasmEvent::new(0, 0, Vec::new()),
            Event::Title(s) => WasmEvent::new(1, 0, s.into_bytes()),
            Event::IconName(s) => WasmEvent::new(2, 0, s.into_bytes()),
            Event::Osc { ident, data } => WasmEvent::new(3, ident, data),
            Event::Notification(n) => {
                let mut ev = WasmEvent::new(4, 0, Vec::new());
                ev.notification = Some(*n);
                ev
            }
            Event::NotificationClose { id } => WasmEvent::new(5, 0, id.into_bytes()),
            Event::NotificationQuery { id } => WasmEvent::new(6, 0, id.into_bytes()),
            Event::NotificationAlive { id } => WasmEvent::new(7, 0, id.into_bytes()),
            Event::ClipboardWrite(w) => {
                let w = *w;
                let mut ev = WasmEvent::new(8, 0, w.id.into_bytes());
                ev.clipboard_location = location_code(w.location);
                for item in w.items {
                    ev.clipboard_mimes.push(item.mime);
                    ev.clipboard_data.push(item.data);
                }
                ev
            }
            Event::ClipboardRead {
                id,
                location,
                request,
            } => {
                let mut ev = WasmEvent::new(9, 0, id.into_bytes());
                ev.clipboard_location = location_code(location);
                match request {
                    ClipboardReadRequest::Types => ev.clipboard_list = true,
                    ClipboardReadRequest::Mimes(m) => ev.clipboard_mimes = m,
                }
                ev
            }
            Event::FileTransfer(e) => match ft_payload(*e) {
                Some(ft) => {
                    let mut ev = WasmEvent::new(11, 0, Vec::new());
                    ev.ft = ft;
                    ev
                }
                None => WasmEvent::new(255, 0, Vec::new()),
            },
            Event::ProgramStatus(c) => {
                let c = *c;
                WasmEvent::new(
                    12,
                    0,
                    program_status_fields(&c.id, c.status.as_ref()).into_bytes(),
                )
            }
            Event::Dnd(d) => {
                let dnd = dnd_payload(*d);
                let mut ev = WasmEvent::new(if dnd.kind == 255 { 255 } else { 10 }, 0, Vec::new());
                ev.dnd = dnd;
                ev
            }
            _ => WasmEvent::new(255, 0, Vec::new()),
        })
    }

    /// 通知が押されたことを返す。`button` は 0 なら通知そのもの、1 以上ならボタンの番号。
    pub fn notification_activated(&mut self, id: &str, button: u32) {
        self.term
            .notification_activated(id, (button > 0).then_some(button));
    }

    /// 通知が閉じられたことを返す。
    pub fn notification_closed(&mut self, id: &str) {
        self.term.notification_closed(id);
    }

    /// `p=?` の問い合わせに答える。
    #[allow(clippy::too_many_arguments)]
    pub fn notification_query_reply(
        &mut self,
        id: &str,
        actions: Vec<String>,
        when: Vec<String>,
        payloads: Vec<String>,
        urgencies: Vec<u8>,
        close_report: bool,
        timeout: bool,
        sounds: Vec<String>,
        types: Vec<String>,
    ) {
        self.term.notification_query_reply(
            id,
            &NotificationSupport {
                actions,
                when,
                payloads,
                urgencies,
                close_report,
                timeout,
                sounds,
                types,
            },
        );
    }

    /// `p=alive` の問い合わせに答える。
    pub fn notification_alive_reply(&mut self, id: &str, ids: Vec<String>) {
        self.term.notification_alive_reply(id, &ids);
    }

    /// クリップボードへの書き込み（種類が 8 のイベント）に答える。`status` は `DONE`（成功）か、
    /// `EIO`・`EINVAL`・`ENOSYS`・`EPERM`・`EBUSY`・`EFBIG`。知らない値は `EIO`。
    pub fn clipboard_write_reply(&mut self, id: &str, status: &str) {
        let result = match status {
            "DONE" => Ok(()),
            "EINVAL" => Err(ClipboardWriteError::Einval),
            "ENOSYS" => Err(ClipboardWriteError::Enosys),
            "EPERM" => Err(ClipboardWriteError::Eperm),
            "EBUSY" => Err(ClipboardWriteError::Ebusy),
            "EFBIG" => Err(ClipboardWriteError::Efbig),
            _ => Err(ClipboardWriteError::Eio),
        };
        self.term.clipboard_write_reply(id, result);
    }

    /// 読み取りの頼み（種類が 9 のイベント）の種類の一覧（`.`）に答える。
    pub fn clipboard_read_reply_types(&mut self, id: &str, mimes: Vec<String>) {
        self.term.clipboard_read_reply_types(id, &mimes);
    }

    /// 読み取りの答えに入れるデータを 1 つ足す。`clipboard_read_reply_data` が送って空にする。
    pub fn clipboard_reply_item(&mut self, mime: String, data: Vec<u8>) {
        self.clipboard_reply_items
            .push(ClipboardItem { mime, data });
    }

    /// 読み取りの頼みのデータに、`clipboard_reply_item` で足したものを答える。足していなければ、データの無い `OK` と `DONE` だけを返す。
    pub fn clipboard_read_reply_data(&mut self, id: &str) {
        let items = std::mem::take(&mut self.clipboard_reply_items);
        self.term.clipboard_read_reply_data(id, &items);
    }

    /// 読み取りの頼みを断る。`error` は `ENOSYS`・`EPERM`・`EBUSY`。知らない値は `EPERM`。
    pub fn clipboard_read_reply_error(&mut self, id: &str, error: &str) {
        self.clipboard_reply_items.clear();
        let e = match error {
            "ENOSYS" => ClipboardReadError::Enosys,
            "EBUSY" => ClipboardReadError::Ebusy,
            _ => ClipboardReadError::Eperm,
        };
        self.term.clipboard_read_reply_error(id, e);
    }

    // ---- kitty の file transfer protocol（OSC 5113）----

    /// セッションの始まり（`send`・`receive`）や、ファイル・ディレクトリの受け付けを許す（`status=OK`）。
    /// `file_id` は空なら付けない。`size` は書き終えたバイト数、`name` はホームディレクトリ（受け取りの一覧の終わり）。
    pub fn file_transfer_ok(
        &mut self,
        id: &str,
        file_id: &str,
        size: Option<f64>,
        name: Option<String>,
    ) {
        self.term.file_transfer_ok(
            id,
            (!file_id.is_empty()).then_some(file_id),
            size.map(|s| s as u64),
            name.as_deref(),
        );
    }

    /// ファイルを受け付けた（`status=STARTED`）。
    pub fn file_transfer_started(&mut self, id: &str, file_id: &str, rsync: bool) {
        self.term.file_transfer_started(id, file_id, rsync);
    }

    /// データを書いた（`status=PROGRESS`）。
    pub fn file_transfer_progress(&mut self, id: &str, file_id: &str, written: f64) {
        self.term
            .file_transfer_progress(id, file_id, written as u64);
    }

    /// エラーで答える。`error` は `EPERM`・`ENOENT`・`EEXIST`・`EISDIR`・`ENOTDIR`・`EACCES`・`EINVAL`・`EIO`・
    /// `EFBIG`・`ENOSPC`。知らない値は `EIO`。`file_id` が空ならセッション全体のエラー。
    pub fn file_transfer_error(&mut self, id: &str, file_id: &str, error: &str, message: &str) {
        let e = match error {
            "EPERM" => FileTransferError::Eperm,
            "ENOENT" => FileTransferError::Enoent,
            "EEXIST" => FileTransferError::Eexist,
            "EISDIR" => FileTransferError::Eisdir,
            "ENOTDIR" => FileTransferError::Enotdir,
            "EACCES" => FileTransferError::Eacces,
            "EINVAL" => FileTransferError::Einval,
            "EFBIG" => FileTransferError::Efbig,
            "ENOSPC" => FileTransferError::Enospc,
            _ => FileTransferError::Eio,
        };
        self.term
            .file_transfer_error(id, (!file_id.is_empty()).then_some(file_id), e, message);
    }

    /// セッションの始まりを断る。
    pub fn file_transfer_refuse(&mut self, id: &str) {
        self.term.file_transfer_refuse(id);
    }

    /// 取り消しに答える（`status=CANCELED`）。
    pub fn file_transfer_canceled(&mut self, id: &str) {
        self.term.file_transfer_canceled(id);
    }

    /// 受け取りの一覧に項目を足す。`file_type` は 0 が regular、1 が directory、2 が symlink、3 が link。
    #[allow(clippy::too_many_arguments)]
    pub fn file_transfer_listing(
        &mut self,
        id: &str,
        request_id: String,
        entry_id: String,
        name: String,
        file_type: u8,
        mtime: Option<i64>,
        permissions: Option<u32>,
        size: Option<f64>,
        parent: Option<String>,
        data: Option<Vec<u8>>,
    ) {
        self.term.file_transfer_listing(
            id,
            &FileTransferListing {
                request_id,
                entry_id,
                name,
                file_type: match file_type {
                    1 => FileType::Directory,
                    2 => FileType::Symlink,
                    3 => FileType::Link,
                    _ => FileType::Regular,
                },
                mtime,
                permissions,
                size: size.map(|s| s as u64),
                parent,
                data,
            },
        );
    }

    /// 受け取るファイルのデータを送る。`last` が真なら最後を `end_data` にする。
    pub fn file_transfer_data(&mut self, id: &str, file_id: &str, bytes: &[u8], last: bool) {
        self.term.file_transfer_data(id, file_id, bytes, last);
    }

    // ---- kitty の drag and drop protocol（OSC 72）----

    /// プログラムがドロップを受け付けると言っているか。
    pub fn drop_accepting(&self) -> bool {
        self.term.drop_accepting()
    }

    /// プログラムがドロップを受け付けると言った MIME の種類。
    pub fn drop_accepted_mimes(&self) -> Vec<String> {
        self.term.drop_accepted_mimes().to_vec()
    }

    /// プログラムが渡した機械の ID（ドロップ側）。
    pub fn drop_machine_id(&self) -> Option<String> {
        self.term.drop_machine_id().map(str::to_string)
    }

    /// ドラッグが窓の上を動いた。`allowed` は 1 が copy、2 が move、3 が両方。
    pub fn drop_move(
        &mut self,
        x: i32,
        y: i32,
        pixel_x: i32,
        pixel_y: i32,
        mimes: Vec<String>,
        allowed: u8,
    ) -> bool {
        self.term.drop_move(
            dnd_location(x, y, pixel_x, pixel_y),
            &mimes,
            dnd_ops(allowed),
        )
    }

    /// ドラッグが窓から出た。
    pub fn drop_leave(&mut self) -> bool {
        self.term.drop_leave()
    }

    /// ドロップされた。
    pub fn drop_drop(
        &mut self,
        x: i32,
        y: i32,
        pixel_x: i32,
        pixel_y: i32,
        mimes: Vec<String>,
        allowed: u8,
        from_this_window: bool,
    ) -> bool {
        self.term.drop_drop(
            dnd_location(x, y, pixel_x, pixel_y),
            &mimes,
            dnd_ops(allowed),
            from_this_window,
        )
    }

    /// ドロップのデータの頼み（`dnd_ints` の並びの `request_kind`・`a`・`b`）にデータで答える。
    /// `form` は 0 が MIME の種類のデータ（`flag` が遠くの機械か）、1 がファイル、2 がシンボリックリンク、
    /// 3 がディレクトリ（`handle` がその handle）。
    pub fn drop_data_reply(
        &mut self,
        request_kind: u8,
        a: u32,
        b: u32,
        form: u8,
        handle: u32,
        remote: bool,
        data: Vec<u8>,
    ) {
        let Some(req) = dnd_request(request_kind, a, b) else {
            return;
        };
        let form = match form {
            0 => DropForm::Mime { remote },
            1 => DropForm::File,
            2 => DropForm::Symlink,
            _ => DropForm::Directory(handle),
        };
        self.term.drop_data_reply(&req, form, &data);
    }

    /// ドロップのデータの頼みにエラーで答える。`error` は POSIX の記号名（`ENOENT` など）。
    pub fn drop_error_reply(
        &mut self,
        request_kind: u8,
        a: u32,
        b: u32,
        error: &str,
        description: &str,
    ) {
        let Some(req) = dnd_request(request_kind, a, b) else {
            return;
        };
        self.term
            .drop_error_reply(&req, dnd_error(error), description);
    }

    /// プログラムがドラッグを始められると言っているか。
    pub fn drag_offers_enabled(&self) -> bool {
        self.term.drag_offers_enabled()
    }

    /// プログラムが渡した機械の ID（ドラッグ側）。
    pub fn drag_machine_id(&self) -> Option<String> {
        self.term.drag_machine_id().map(str::to_string)
    }

    /// ドラッグの身振りを見つけた。
    pub fn drag_gesture(&mut self, x: i32, y: i32, pixel_x: i32, pixel_y: i32) -> bool {
        self.term.drag_gesture(dnd_location(x, y, pixel_x, pixel_y))
    }

    /// OS のドラッグが始まった。
    pub fn drag_started(&mut self) -> bool {
        self.term.drag_started()
    }

    /// ドラッグを始められなかった・止める。`error` は POSIX の記号名。
    pub fn drag_error(&mut self, error: &str, description: &str) -> bool {
        self.term.drag_error(dnd_error(error), description)
    }

    /// ドラッグが受け入れられた。
    pub fn drag_accepted(&mut self, index: u32) -> bool {
        self.term.drag_accepted(index)
    }

    /// 相手が行いそうな操作が変わった。`operation` は 0 が受け付けない、1 が copy、2 が move。
    pub fn drag_action(&mut self, operation: u8) -> bool {
        let op = match operation {
            1 => Some(DndOperation::Copy),
            2 => Some(DndOperation::Move),
            _ => None,
        };
        self.term.drag_action(op)
    }

    /// ドラッグが相手に落とされた。
    pub fn drag_dropped(&mut self) -> bool {
        self.term.drag_dropped()
    }

    /// ドラッグが終わった。
    pub fn drag_finished(&mut self, canceled: bool) -> bool {
        self.term.drag_finished(canceled)
    }

    /// MIME の種類のデータをプログラムに頼む。
    pub fn drag_data_request(&mut self, index: u32) -> bool {
        self.term.drag_data_request(index)
    }

    /// 遠くの機械へのドラッグで、`text/uri-list` の項目をプログラムに頼む。
    pub fn drag_entry_request(&mut self, index: u32) -> bool {
        self.term.drag_entry_request(index)
    }

    /// PTY へ送るバイト列を取り出す。
    pub fn take_output(&mut self) -> Vec<u8> {
        self.term.take_output()
    }

    /// PTY へ送るバイト列を、返事と同じ出口に入れる（キーボードやマウス用）。
    pub fn queue_output(&mut self, bytes: &[u8]) {
        self.term.queue_output(bytes);
    }

    /// 色の問い合わせに返すテーマを変える。`ansi` は 16 色ぶんの RGB（48 バイト）。
    pub fn set_theme(&mut self, foreground: &[u8], background: &[u8], cursor: &[u8], ansi: &[u8]) {
        let rgb = |b: &[u8]| [b[0], b[1], b[2]];
        let mut theme = Theme {
            foreground: rgb(foreground),
            background: rgb(background),
            cursor: rgb(cursor),
            ..Theme::default()
        };
        for i in 0..16 {
            theme.ansi[i] = rgb(&ansi[i * 3..]);
        }
        self.term.set_theme(theme);
    }

    /// 前に呼んでから色が変わったか（プログラムが OSC 4・10・11・12・21 で変えた、テーマを変えた、
    /// 積み重ねから戻した）。呼ぶと印を消す。
    pub fn colors_changed(&mut self) -> bool {
        self.term.take_colors_changed()
    }

    /// いまの色。並びは、文字・背景・カーソルの RGB（9 バイト）、続けて `cursor_text`・
    /// `selection_background`・`selection_foreground` を 1 つ 4 バイト（設定されていれば 1、R、G、B。
    /// 動的なら 0 が 4 つ）、続けて 256 色の RGB（768 バイト）。
    pub fn colors(&self) -> Vec<u8> {
        let c = self.term.colors();
        let mut out = Vec::with_capacity(9 + 12 + 768);
        for rgb in [c.foreground, c.background, c.cursor] {
            out.extend_from_slice(&rgb);
        }
        for opt in [
            c.cursor_text,
            c.selection_background,
            c.selection_foreground,
        ] {
            match opt {
                Some(rgb) => out.extend_from_slice(&[1, rgb[0], rgb[1], rgb[2]]),
                None => out.extend_from_slice(&[0; 4]),
            }
        }
        for rgb in c.palette {
            out.extend_from_slice(&rgb);
        }
        out
    }

    /// ウィンドウのフォーカスの出入りを知らせる。
    pub fn set_focus(&mut self, focused: bool) {
        self.term.set_focus(focused);
        self.events.extend(self.term.take_events());
    }

    /// OSC の番号 `ident` を呼ぶ側で拾う。
    pub fn register_osc(&mut self, ident: u32) {
        self.term.register_osc(ident);
    }

    /// OSC の番号の登録を外す。
    pub fn unregister_osc(&mut self, ident: u32) {
        self.term.unregister_osc(ident);
    }

    /// カーソルを点滅させるか（`DECSCUSR` で変わる）。
    pub fn set_cursor_blink(&mut self, blink: bool) {
        self.term.screen_mut().cursor_blinking = blink;
    }

    /// カーソルの状態。`[x, y, 見えるか, 点滅するか, 形（0 ブロック 1 下線 2 縦棒）]`。
    pub fn cursor(&self) -> Vec<u32> {
        use coneko_term::screen::CursorShape;
        let c = self.term.screen().cursor();
        vec![
            c.x as u32,
            c.y as u32,
            c.visible as u32,
            c.blinking as u32,
            match c.shape {
                CursorShape::Underline => 1,
                CursorShape::Bar => 2,
                _ => 0,
            },
        ]
    }

    /// OSC 22 で決まった、いまのポインターの形（CSS の `cursor` の値）。決まっていなければ空。
    pub fn pointer_shape(&self) -> Option<String> {
        self.term.pointer_shape().map(str::to_owned)
    }

    /// 追加のカーソル（kitty の multiple cursors protocol）。`[x, y, 形]` の並び（上の行から、
    /// 左の列の順）。形は 0 ブロック 1 下線 2 縦棒 3 主のカーソルに従う。
    pub fn extra_cursors(&self) -> Vec<u32> {
        use coneko_term::screen::ExtraCursorShape;
        let mut v = Vec::new();
        for (x, y, shape) in self.term.screen().extra_cursors.iter() {
            v.push(x as u32);
            v.push(y as u32);
            v.push(match shape {
                ExtraCursorShape::Block => 0,
                ExtraCursorShape::Underline => 1,
                ExtraCursorShape::Bar => 2,
                _ => 3,
            });
        }
        v
    }

    /// 追加のカーソルの色の組 `[字の色, カーソルの色]`。色の語は、セルの色と同じ書き方
    /// （上位 8 ビットが種類: 0 主のカーソルと同じ、1 パレット、2 RGB、3 反転）。
    pub fn extra_cursor_colors(&self) -> Vec<u32> {
        use coneko_term::screen::ExtraCursorColor;
        let word = |c: ExtraCursorColor| match c {
            ExtraCursorColor::Special => 3 << 24,
            ExtraCursorColor::Rgb(r, g, b) => {
                (2 << 24) | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
            }
            ExtraCursorColor::Indexed(i) => (1 << 24) | u32::from(i),
            _ => 0,
        };
        let c = &self.term.screen().extra_cursors;
        vec![word(c.text_color), word(c.cursor_color)]
    }

    /// マウスが窓から出たことを報告する（kitty の拡張）。報告したら `true`。
    pub fn report_mouse_leave(&mut self) -> bool {
        self.term.report_mouse_leave()
    }

    /// キーを押したことを知らせる（終わった・失敗した状態の記録を消す）。
    pub fn program_status_key_pressed(&mut self) {
        self.term.program_status_key_pressed();
        self.events.extend(self.term.take_events());
    }

    /// プロセスが終わったことを知らせる（動いている・待っている状態の記録を消す）。
    pub fn program_status_drop_running(&mut self) {
        self.term.program_status_drop_running();
        self.events.extend(self.term.take_events());
    }

    /// いま持っているプログラムの状態の記録。記録を `\u{1}` で、項目を `\0` で区切る
    /// （項目は `program_status_fields` の並び）。
    pub fn program_status_records(&self) -> String {
        self.term
            .program_status()
            .iter()
            .map(|(id, s)| program_status_fields(id, Some(s)))
            .collect::<Vec<_>>()
            .join("\u{1}")
    }

    /// マウスの報告が有効か。
    pub fn mouse_reporting(&self) -> bool {
        self.term.mouse_reporting()
    }

    /// マウスのイベントを報告する。報告したら `true`（バイト列は `take_output` に出る）。
    ///
    /// `button`: 0 左、1 真ん中、2 右、3 押していない、4 戻る、5 進む、6 ホイール上、
    /// 7 ホイール下、8 ホイール左、9 ホイール右。`action`: 0 押した、1 離した、2 動かした。
    /// `mods`: ビット 0 が Shift、1 が Alt、2 が Ctrl。`col` と `row` は 0 始まり。
    #[allow(clippy::too_many_arguments)]
    pub fn report_mouse(
        &mut self,
        button: u8,
        action: u8,
        col: usize,
        row: usize,
        x: u32,
        y: u32,
        mods: u8,
    ) -> bool {
        let button = match button {
            0 => MouseButton::Left,
            1 => MouseButton::Middle,
            2 => MouseButton::Right,
            4 => MouseButton::Back,
            5 => MouseButton::Forward,
            6 => MouseButton::WheelUp,
            7 => MouseButton::WheelDown,
            8 => MouseButton::WheelLeft,
            9 => MouseButton::WheelRight,
            _ => MouseButton::NoButton,
        };
        let action = match action {
            0 => MouseAction::Press,
            1 => MouseAction::Release,
            _ => MouseAction::Move,
        };
        let mut e = MouseEvent::new(button, action, col, row);
        e.x = x;
        e.y = y;
        e.shift = mods & 1 != 0;
        e.alt = mods & 2 != 0;
        e.ctrl = mods & 4 != 0;
        self.term.report_mouse(&e)
    }

    /// スクロールバックの上から捨てた行の数の合計（代替画面では 0）。選択の位置を覚えるのに使う。
    pub fn scrollback_trimmed(&self) -> f64 {
        self.term.screen().scrollback_trimmed() as f64
    }

    /// 行の並び（スクロールバック + 画面）の `row` 行 `col` 列の単語の範囲 `[開始の行, 開始の列, 終わりの行, 終わりの列]`（終わりは含まない）。
    pub fn word_range(&self, row: usize, col: usize, separators: &str) -> Vec<u32> {
        let (a, b) = selection::word_at(self.term.screen(), row, col, separators);
        vec![a.row as u32, a.col as u32, b.row as u32, b.col as u32]
    }

    /// `row` 行目を含む、折り返しでつながった行全体の範囲。並びは `word_range` と同じ。
    pub fn line_range(&self, row: usize) -> Vec<u32> {
        let (a, b) = selection::line_range(self.term.screen(), row);
        vec![a.row as u32, a.col as u32, b.row as u32, b.col as u32]
    }

    /// 範囲 `[開始, 終わり)` の字。折り返した行は改行を入れずにつなぐ。
    pub fn selection_text(&self, row0: usize, col0: usize, row1: usize, col1: usize) -> String {
        selection::text(
            self.term.screen(),
            Pos::new(row0, col0),
            Pos::new(row1, col1),
        )
    }

    /// 代替画面か。
    pub fn is_alt_screen(&self) -> bool {
        self.term.screen().is_alt_screen()
    }

    /// bracketed paste が有効か（`CSI ? 2004 h`）。
    pub fn bracketed_paste(&self) -> bool {
        self.term.modes().bracketed_paste
    }

    /// アプリケーションカーソルキーか（`CSI ? 1 h`）。
    pub fn application_cursor_keys(&self) -> bool {
        self.term.modes().application_cursor_keys
    }

    /// いまの画面の kitty keyboard protocol のフラグ（`CSI ? u` で答える値）。
    /// 1 が立っていれば disambiguate escape codes。
    pub fn keyboard_flags(&self) -> u32 {
        self.term.keyboard_flags()
    }

    /// 変わった行（見えている画面の行番号）を返し、「変わった」の印を消す。
    pub fn take_dirty_rows(&mut self) -> Vec<u32> {
        self.term
            .screen_mut()
            .take_dirty_rows()
            .into_iter()
            .map(|y| y as u32)
            .collect()
    }

    /// 見えている画面の `y` 行目を、セルのバッファーに書き出す。セルの数を返す。
    pub fn read_row(&mut self, y: usize) -> usize {
        if y >= self.term.screen().rows() {
            return self.fill(None);
        }
        let line = self.term.screen().row(y).clone();
        self.fill(Some(&line))
    }

    /// スクロールバックの `i` 行目（0 が一番古い）を、セルのバッファーに書き出す。
    pub fn read_scrollback_row(&mut self, i: usize) -> usize {
        if i >= self.term.screen().scrollback_len() {
            return self.fill(None);
        }
        let line = self.term.screen().scrollback_row(i).clone();
        self.fill(Some(&line))
    }

    /// 直前の `read_*` で書き出した行が、次の行へ折り返されているか。
    pub fn row_wrapped(&self) -> bool {
        self.last_wrapped
    }

    /// 直前の `read_*` で書き出した行が、シェルが入力を受けている行（プロンプトと打っている途中のコマンド）か。
    pub fn row_input(&self) -> bool {
        self.last_input
    }

    /// セルのバッファーの位置（WASM のメモリの中の byte offset）。
    pub fn cells_ptr(&self) -> *const u32 {
        self.cells.as_ptr()
    }

    /// クラスター（結合文字を含む書記素）の字。番号は、直前の `read_*` で書き出したセルのもの。
    pub fn cluster(&self, index: usize) -> String {
        self.clusters.get(index).cloned().unwrap_or_default()
    }
    /// 複数のセルにまたがる字（OSC 66）の字。番号は、セルの並びの 6 番目の下位 16 ビットから 1 を引いたもの。
    pub fn sized_text(&self, index: usize) -> String {
        self.sized
            .get(index)
            .map(|b| b.text.to_string())
            .unwrap_or_default()
    }

    /// 複数のセルにまたがる字の四角。幅（セル）・高さ（セル、大きさ `s`）・分数の分子・分母・縦の寄せ方・
    /// 横の寄せ方（0 上または左、1 下または右、2 中央）の 6 つ。
    pub fn sized_info(&self, index: usize) -> Vec<u32> {
        use coneko_term::screen::Align;
        let code = |a: Align| match a {
            Align::End => 1,
            Align::Center => 2,
            _ => 0,
        };
        self.sized.get(index).map_or_else(Vec::new, |b| {
            vec![
                u32::from(b.cols),
                u32::from(b.scale),
                u32::from(b.numerator),
                u32::from(b.denominator),
                code(b.vertical),
                code(b.horizontal),
            ]
        })
    }

    /// セルの大きさ（画素）を教える。`c`・`r` が無い画像の配置の大きさを決めるのに使う。
    pub fn set_cell_size(&mut self, width: u32, height: u32) {
        self.term.set_cell_size(width, height);
    }

    /// 画像の配置を上の行から順に、`PLACEMENT_STRIDE` 個ずつの数で返す。
    /// 並びは `PLACEMENT_STRIDE` の説明を参照。
    pub fn placements(&self) -> Vec<f64> {
        let mut out = Vec::new();
        for p in self.term.placements() {
            let q = &p.placement;
            out.extend_from_slice(&[
                p.row as f64,
                q.image_id as f64,
                q.placement_id as f64,
                q.col as f64,
                q.src_x as f64,
                q.src_y as f64,
                q.src_w as f64,
                q.src_h as f64,
                q.offset_x as f64,
                q.offset_y as f64,
                q.cols as f64,
                q.rows as f64,
                q.z as f64,
                q.native_size as u8 as f64,
                q.dest_size.map_or(0, |d| d.0) as f64,
                q.dest_size.map_or(0, |d| d.1) as f64,
            ]);
        }
        out
    }

    /// 持っている画像の `[ID, 通し番号]` の並び。同じ ID で通し番号が変われば別の画像。
    pub fn image_serials(&self) -> Vec<f64> {
        self.term
            .images()
            .flat_map(|i| [i.id as f64, i.serial as f64])
            .collect()
    }

    /// 画像の情報 `[通し番号, 形式（0 RGB、1 RGBA、2 PNG）, 圧縮されているか, 幅, 高さ]`。
    /// 幅と高さが 0 なら分からない。画像が無ければ空。
    pub fn image_info(&self, id: u32) -> Vec<f64> {
        use coneko_term::graphics::ImageFormat;
        match self.term.image(id) {
            None => Vec::new(),
            Some(i) => vec![
                i.serial as f64,
                match i.format {
                    ImageFormat::Rgb => 0.0,
                    ImageFormat::Rgba => 1.0,
                    _ => 2.0,
                },
                i.compressed as u8 as f64,
                i.width as f64,
                i.height as f64,
            ],
        }
    }

    /// 画像のバイト列（base64 を戻したもの。圧縮も PNG もそのまま）。画像が無ければ空。
    pub fn image_data(&self, id: u32) -> Vec<u8> {
        self.term
            .image(id)
            .map(|i| i.data.clone())
            .unwrap_or_default()
    }

    /// 画像のアニメーションの状態 `[コマの数, いまのコマ（1 始まり）, 再生の状態（1 止まっている、
    /// 2 回す・最後で待つ、3 回す）, いまのコマの表示時間（ミリ秒。負は見せずに飛ばす）, 操作の数]`。
    /// 画像が無ければ空。操作の数が 0 なら、コマを足していない（ふつうの画像）。
    pub fn animation_info(&self, id: u32) -> Vec<f64> {
        use coneko_term::graphics::PlayState;
        match self.term.image(id) {
            None => Vec::new(),
            Some(i) => {
                let a = &i.animation;
                vec![
                    a.frame_count() as f64,
                    f64::from(a.current),
                    match a.state {
                        PlayState::Stopped => 1.0,
                        PlayState::Loading => 2.0,
                        PlayState::Running => 3.0,
                    },
                    f64::from(a.current_gap()),
                    a.ops.len() as f64,
                ]
            }
        }
    }

    /// いまのコマの表示時間が過ぎたと知らせる。次のコマへ進んだら `true`。時計は描く側が持つ。
    pub fn animation_step(&mut self, id: u32) -> bool {
        self.term.animation_step(id)
    }

    /// コマを作る `index` 番目（0 始まり）の操作。無ければ空。並びは次のとおり:
    /// - データを置く: `[0, 作るコマ, 下地の種類（0 そのコマ自身、1 別のコマ、2 1 色）, 下地の値
    ///   （コマの番号、または 0xRRGGBBAA）, x, y, 幅, 高さ, 置き換えか, 形式（0 RGB、1 RGBA、2 PNG）,
    ///   圧縮されているか]`
    /// - 四角を重ねる: `[1, 元のコマ, 先のコマ, 元の x, 元の y, 先の x, 先の y, 幅, 高さ, 置き換えか]`
    pub fn frame_op(&self, id: u32, index: usize) -> Vec<f64> {
        use coneko_term::graphics::{FrameBase, FrameOp, ImageFormat};
        let Some(op) = self.term.image(id).and_then(|i| i.animation.ops.get(index)) else {
            return Vec::new();
        };
        match op {
            FrameOp::Load {
                target,
                base,
                x,
                y,
                width,
                height,
                overwrite,
                format,
                compressed,
                ..
            } => {
                let (kind, value) = match base {
                    FrameBase::Edit => (0, 0),
                    FrameBase::Frame(n) => (1, *n),
                    FrameBase::Color(c) => (2, *c),
                };
                vec![
                    0.0,
                    f64::from(*target),
                    f64::from(kind),
                    f64::from(value),
                    f64::from(*x),
                    f64::from(*y),
                    f64::from(*width),
                    f64::from(*height),
                    f64::from(u8::from(*overwrite)),
                    match format {
                        ImageFormat::Rgb => 0.0,
                        ImageFormat::Rgba => 1.0,
                        _ => 2.0,
                    },
                    f64::from(u8::from(*compressed)),
                ]
            }
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
            } => vec![
                1.0,
                f64::from(*src),
                f64::from(*dst),
                f64::from(*src_x),
                f64::from(*src_y),
                f64::from(*dst_x),
                f64::from(*dst_y),
                f64::from(*width),
                f64::from(*height),
                f64::from(u8::from(*overwrite)),
            ],
        }
    }

    /// `frame_op` の「データを置く」操作のバイト列（圧縮も PNG もそのまま）。無ければ空。
    pub fn frame_op_data(&self, id: u32, index: usize) -> Vec<u8> {
        use coneko_term::graphics::FrameOp;
        match self.term.image(id).and_then(|i| i.animation.ops.get(index)) {
            Some(FrameOp::Load { data, .. }) => data.clone(),
            _ => Vec::new(),
        }
    }
}

impl WasmTerminal {
    fn fill(&mut self, line: Option<&Line>) -> usize {
        self.cells.clear();
        self.clusters.clear();
        self.sized.clear();
        self.last_wrapped = line.is_some_and(|l| l.is_wrapped());
        self.last_input = line.is_some_and(|l| l.is_input());
        let Some(line) = line else { return 0 };
        for cell in line.cells() {
            self.push_cell(cell);
        }
        line.cells().len()
    }

    fn push_cell(&mut self, cell: &Cell) {
        let cp = match &cell.content {
            Content::Empty | Content::WideTail => 0,
            Content::Char(c) => *c as u32,
            Content::Sized { .. } => 0,
            Content::Cluster(s) => {
                self.clusters.push(s.to_string());
                CLUSTER_FLAG | (self.clusters.len() as u32 - 1)
            }
            _ => 0,
        };
        let (width, sized) = match &cell.content {
            Content::Sized { block, dx, dy } => {
                if !self
                    .sized
                    .last()
                    .is_some_and(|b| std::sync::Arc::ptr_eq(b, block))
                {
                    self.sized.push(block.clone());
                }
                let n = (self.sized.len() as u32) & 0xFFFF;
                (1, n | (u32::from(*dx) << 16) | (u32::from(*dy) << 24))
            }
            _ => (cell.width as u32, 0),
        };
        let f = cell.attrs.flags;
        use coneko_term::screen::cell::AttrFlags as F;
        let bits = [
            F::BOLD,
            F::DIM,
            F::ITALIC,
            F::BLINK,
            F::INVERSE,
            F::INVISIBLE,
            F::STRIKETHROUGH,
        ]
        .iter()
        .enumerate()
        .fold(0u32, |acc, (i, b)| acc | ((f.contains(*b) as u32) << i));
        self.cells.extend_from_slice(&[
            cp,
            width,
            color_word(cell.attrs.fg),
            color_word(cell.attrs.bg),
            bits | (underline_code(cell.attrs.underline.style) << 8),
            color_word(cell.attrs.underline.color),
            sized,
        ]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_core() {
        assert_eq!(super::version(), coneko_term::version());
    }

    #[test]
    fn rows_are_exported_to_buffer() {
        let mut t = WasmTerminal::new(2, 4, 10);
        t.write("a\u{1b}[1;31mあ".as_bytes());
        assert_eq!(t.take_dirty_rows(), vec![0, 1]);
        assert_eq!(t.read_row(0), 4);
        assert_eq!(t.cells[0], 'a' as u32);
        assert_eq!(t.cells[CELL_STRIDE], 'あ' as u32);
        assert_eq!(t.cells[CELL_STRIDE + 1], 2);
        assert_eq!(t.cells[CELL_STRIDE + 2], (1 << 24) | 1);
        assert_eq!(t.cells[CELL_STRIDE + 4] & 1, 1);
        assert_eq!(t.cells[2 * CELL_STRIDE + 1], 0);
        assert!(t.take_dirty_rows().is_empty());
    }
}

#[cfg(test)]
mod sized_tests {
    use super::*;

    #[test]
    fn sized_cells_are_exported() {
        let mut t = WasmTerminal::new(3, 6, 10);
        t.write(b"\x1b]66;s=2;Hi\x07");
        assert_eq!(t.read_row(1), 6);
        // 2 行目の 2 文字目の左から 2 番目（dx = 1, dy = 1）
        let o = 3 * CELL_STRIDE;
        assert_eq!(t.cells[o], 0);
        assert_eq!(t.cells[o + 1], 1);
        assert_eq!(t.cells[o + 6], 2 | (1 << 16) | (1 << 24));
        assert_eq!(t.sized_text(1), "i");
        assert_eq!(t.sized_info(1), vec![2, 2, 0, 0, 0, 0]);
        assert_eq!(t.cells[4 * CELL_STRIDE + 6], 0);
        assert!(t.sized_info(5).is_empty());
    }
}

#[cfg(test)]
mod image_tests {
    use super::*;

    #[test]
    fn placements_and_images_are_exported() {
        let mut t = WasmTerminal::new(5, 20, 10);
        t.set_cell_size(10, 20);
        // 1x2 の RGB を、2 列 x 1 行で置く
        t.write(b"\x1b_Ga=T,f=24,s=1,v=2,i=7,c=2,r=1,z=-1;AAAAAAAA\x1b\\");
        let p = t.placements();
        assert_eq!(p.len(), PLACEMENT_STRIDE);
        // 行 0、ID 7、配置の番号 0、列 0、元の部分 (0, 0, 1, 2)、ずれ (0, 0)、2 列 1 行、z -1、引き伸ばし
        assert_eq!(
            p,
            vec![0.0, 7.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 0.0, 0.0, 2.0, 1.0, -1.0, 0.0, 0.0, 0.0]
        );
        let serials = t.image_serials();
        assert_eq!(serials.len(), 2);
        assert_eq!(serials[0], 7.0);
        assert_eq!(t.image_info(7), vec![serials[1], 0.0, 0.0, 1.0, 2.0]);
        assert_eq!(t.image_data(7), vec![0; 6]);
        assert!(t.image_info(8).is_empty());
        assert!(t.image_data(8).is_empty());
    }

    #[test]
    fn animation_is_exported() {
        let mut t = WasmTerminal::new(5, 20, 10);
        t.write(b"\x1b_Ga=t,f=32,s=1,v=1,i=1;AAAAAA==\x1b\\");
        assert_eq!(t.animation_info(1), vec![1.0, 1.0, 1.0, 0.0, 0.0]);
        assert!(t.animation_info(2).is_empty());
        t.write(b"\x1b_Ga=f,f=32,s=1,v=1,i=1,z=25;/wAA/w==\x1b\\");
        t.write(b"\x1b_Ga=c,i=1,r=1,c=2\x1b\\");
        t.write(b"\x1b_Ga=a,i=1,s=3\x1b\\");
        assert_eq!(t.animation_info(1), vec![2.0, 1.0, 3.0, 0.0, 2.0]);
        assert_eq!(
            t.frame_op(1, 0),
            vec![0.0, 2.0, 2.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0]
        );
        assert_eq!(t.frame_op_data(1, 0), vec![0xff, 0, 0, 0xff]);
        assert_eq!(
            t.frame_op(1, 1),
            vec![1.0, 1.0, 2.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0]
        );
        assert!(t.frame_op_data(1, 1).is_empty());
        assert!(t.frame_op(1, 2).is_empty());
        assert!(t.animation_step(1));
        assert_eq!(t.animation_info(1), vec![2.0, 2.0, 3.0, 25.0, 2.0]);
    }
}
