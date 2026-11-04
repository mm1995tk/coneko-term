import { WasmTerminal, type WasmEvent } from "./wasm/coneko_term_wasm.js";
import { getMemory } from "./memory.js";
import {
  SelectionModel,
  type SelectionHost,
  type SelectionRange,
} from "./selection.js";
import {
  CompositionHelper,
  KeyHandler,
  detectPlatform,
  type CompositionAnchor,
  type CompositionViewLike,
} from "./input.js";
import { bracketTextForPaste, prepareTextForTerminal } from "./keyboard.js";
import {
  CanvasRenderer,
  Palette,
  Scrollbar,
  SCROLLBAR_WIDTH,
  proposeDimensions,
  type CellSize,
} from "./renderer.js";

/** セル 1 つが使う `u32` の数。`RowView.cells` の並びは README を参照。 */
export const CELL_STRIDE = 7;

/** セルの字（並びの 0 番目）にこのビットが立っていたら、残りのビットが `RowView.cluster` の番号。 */
export const CLUSTER_FLAG = 0x8000_0000;

/** `Terminal.handleMouse` に渡すポインターのイベント。 */
export interface MouseInput {
  /** 押した、離した、動かした。 */
  type: "down" | "up" | "move";
  /** DOM の `MouseEvent.button` と同じ（0 左、1 真ん中、2 右、3 戻る、4 進む）。`down` だけが使う。 */
  button: number;
  /** 見えている画面の列（0 始まり）。範囲の外は端に寄せる。 */
  col: number;
  /** 見えている画面の行（0 始まり）。範囲の外は端に寄せる。 */
  row: number;
  /** 画面の左端からの CSS ピクセル（`CSI ? 1016 h` のときだけ使う）。 */
  x?: number;
  /** 画面の上端からの CSS ピクセル（`CSI ? 1016 h` のときだけ使う）。 */
  y?: number;
  /** Shift を押している。 */
  shiftKey?: boolean;
  /** Alt（Option）を押している。 */
  altKey?: boolean;
  /** Ctrl を押している。 */
  ctrlKey?: boolean;
  /** 連続して押した回数（DOM の `MouseEvent.detail`）。1 は範囲、2 は単語、3 は行。`down` だけが使う。 */
  clickCount?: number;
}

/** `Terminal.handleWheel` に渡すホイールのイベント。 */
export interface WheelInput {
  /** 縦の量（DOM の `WheelEvent.deltaY`）。負が上。 */
  deltaY: number;
  /** 0 がピクセル、1 が行、2 がページ（DOM の `WheelEvent.deltaMode`）。 */
  deltaMode?: number;
  /** 見えている画面の列（0 始まり）。報告が有効なときだけ使う（無ければ 0）。 */
  col?: number;
  /** 見えている画面の行（0 始まり）。報告が有効なときだけ使う（無ければ 0）。 */
  row?: number;
  /** 画面の左端からの CSS ピクセル（`CSI ? 1016 h` の報告だけが使う）。 */
  x?: number;
  /** 画面の上端からの CSS ピクセル（`CSI ? 1016 h` の報告だけが使う）。 */
  y?: number;
  /** Shift を押している。押していると縦に動かさない。 */
  shiftKey?: boolean;
  /** Alt（Option）を押している。 */
  altKey?: boolean;
  /** Ctrl を押している。 */
  ctrlKey?: boolean;
}

/** 後始末ができるもの。 */
export interface Disposable {
  dispose(): void;
}

/** 色。`#rgb` か `#rrggbb`。 */
export interface Theme {
  /** 文字の色（OSC 10 の問い合わせに返す）。 */
  foreground?: string;
  /** 背景の色（OSC 11）。 */
  background?: string;
  /** カーソルの色（OSC 12）。 */
  cursor?: string;
  /** 選択範囲の背景。`#rgb`、`#rgba`、`#rrggbb`、`#rrggbbaa`。字はこの色の下に見えるので、半透明にする。既定は `#ffffff4d`（xterm.js と同じ）。 */
  selectionBackground?: string;
  black?: string;
  red?: string;
  green?: string;
  yellow?: string;
  blue?: string;
  magenta?: string;
  cyan?: string;
  white?: string;
  brightBlack?: string;
  brightRed?: string;
  brightGreen?: string;
  brightYellow?: string;
  brightBlue?: string;
  brightMagenta?: string;
  brightCyan?: string;
  brightWhite?: string;
}

/** `Terminal` を作るときの設定。 */
export interface TerminalOptions {
  /** 列の数。既定は 80。 */
  cols?: number;
  /** 行の数。既定は 24。 */
  rows?: number;
  /** 描画に使うフォント（CSS の `font-family`）。既定は `monospace`。 */
  fontFamily?: string;
  /** 字の大きさ（CSS ピクセル）。既定は 15（xterm.js と同じ）。 */
  fontSize?: number;
  /** 行の高さ。字の高さに掛ける倍率（xterm.js と同じ）。既定は 1。 */
  lineHeight?: number;
  /** カーソルを点滅させるか。`CSI Ps SP q` で変わる。既定は false。 */
  cursorBlink?: boolean;
  /** スクロールバックの行数。既定は 1000。 */
  scrollback?: number;
  /** テーマ。 */
  theme?: Theme;
  /** ダブルクリックで単語を選ぶときの区切りの字。既定は xterm.js と同じ（空白と ``()[]{}',"`` ）。 */
  wordSeparator?: string;
  /**
   * macOS で Option を Meta（ESC の前置き）として送るか。既定は false（xterm.js と同じ。
   * Option は字を打つキーとして扱う）。
   */
  macOptionIsMeta?: boolean;
  /** キーを打ったとき、さかのぼって見ていたら一番下へ戻すか。既定は true（xterm.js と同じ）。 */
  scrollOnUserInput?: boolean;
}

/**
 * 組み立て終わった通知（kitty の desktop notifications、OSC 99）。
 * 仕様のキーを読んだ形で持つ。OS の通知につなぐのは呼ぶ側。
 */
export interface TerminalNotification {
  /** `i`。無ければ空文字列。 */
  id: string;
  /** 題名（`p=title`）。 */
  title: string;
  /** 本文（`p=body`）。 */
  body: string;
  /** 緊急度（`u`）。指定が無ければ `undefined`。 */
  urgency?: 0 | 1 | 2;
  /** 押されたときの動き（`a`）。既定は `{ focus: true, report: false }`。 */
  actions: { focus: boolean; report: boolean };
  /** いつ出すか（`o`）。 */
  when: "always" | "unfocused" | "invisible";
  /** 閉じられたときに知らせるか（`c=1`）。 */
  reportClose: boolean;
  /** アプリケーションの名前（`f`）。 */
  appName?: string;
  /** 通知の種類（`t`）。 */
  types: string[];
  /** 消えるまでの時間（`w`、ミリ秒。-1 は OS の既定、0 は消えない）。指定が無ければ `undefined`。 */
  timeout?: number;
  /** 音（`s`）。 */
  sound?: string;
  /** アイコンの名前（`n`）。 */
  iconNames: string[];
  /** アイコンの画像を指す ID（`g`）。画像そのものは読まない。 */
  iconCacheId?: string;
  /** ボタンの文字（`p=buttons`）。 */
  buttons: string[];
}

/** `p=?` への返事に書く、対応していることの一覧。省いた項目は既定（下記）。 */
export interface NotificationSupport {
  /** 対応する `a` の値。既定は `["focus", "report"]`。 */
  actions?: string[];
  /** 対応する `o` の値。既定は `["always", "unfocused", "invisible"]`。 */
  when?: string[];
  /** 対応する `p` の値。既定は `["title", "body", "?", "close", "alive"]`。 */
  payloads?: string[];
  /** 対応する `u` の値。既定は `[0, 1, 2]`。 */
  urgencies?: number[];
  /** `c=1` に対応するか。既定は `true`。 */
  closeReport?: boolean;
  /** `w` に対応するか。既定は `false`。 */
  timeout?: boolean;
  /** 対応する `s` の値。既定は無し。 */
  sounds?: string[];
  /** 対応する `t` の値。既定は無し。 */
  types?: string[];
}

/** どちらのクリップボードか。 */
export type ClipboardLocation = "clipboard" | "primary";

// wasm 側の場所の番号（0 が主のクリップボード、1 が primary selection）。
function clipboardLocationOf(code: number): ClipboardLocation {
  return code === 1 ? "primary" : "clipboard";
}

/** クリップボードの、ある MIME の種類のデータ。 */
export interface ClipboardItem {
  /** MIME の種類（`text/plain` など）。 */
  mime: string;
  /** データ（base64 を戻したあとのバイト列）。 */
  data: Uint8Array;
}

/** クリップボードへの書き込み（kitty の clipboard protocol、OSC 5522）。 */
export interface ClipboardWriteRequest {
  /** `id`。無ければ空文字列。`replyClipboardWrite` に渡す。 */
  id: string;
  /** 書き込み先。 */
  location: ClipboardLocation;
  /** MIME の種類ごとのデータ。別名（`walias`）は、別名ごとの項目に広げてある。 */
  items: ClipboardItem[];
}

/** クリップボードの読み取りの頼み（OSC 5522 の `type=read`）。 */
export interface ClipboardReadRequest {
  /** `id`。無ければ空文字列。答えに渡す。 */
  id: string;
  /** 読む先。 */
  location: ClipboardLocation;
  /** 真なら、あるデータの種類の一覧がほしい（`mimes` は空）。許可を求めなくてよい（仕様）。 */
  list: boolean;
  /** 頼まれた MIME の種類（先に書いたものほど優先）。 */
  mimes: string[];
}

/** 書き込みの答え。`DONE` は成功。 */
export type ClipboardWriteStatus =
  | "DONE"
  | "EIO"
  | "EINVAL"
  | "ENOSYS"
  | "EPERM"
  | "EBUSY"
  | "EFBIG";

/** 読み取りを断る・失敗を知らせる答え。 */
export type ClipboardReadError = "ENOSYS" | "EPERM" | "EBUSY";

/** ドロップやドラッグの操作。 */
export type DndOperation = "copy" | "move";

/** 許す操作の組。 */
export interface DndOperations {
  copy?: boolean;
  move?: boolean;
}

function operationBits(ops: DndOperations): number {
  return (ops.copy ? 1 : 0) | (ops.move ? 2 : 0);
}

function operationOf(code: number): DndOperation | null {
  return code === 1 ? "copy" : code === 2 ? "move" : null;
}

/** ドラッグの位置（kitty の drag and drop protocol）。セルは左上が (0, 0)、画素は左上からのずれ。 */
export interface DndLocation {
  x: number;
  y: number;
  pixelX: number;
  pixelY: number;
}

/** ターミナルが答えるエラーの、POSIX の記号名。知らない値は `EIO`。 */
export type DndErrorName =
  | "ENOENT"
  | "EINVAL"
  | "EUNKNOWN"
  | "EPERM"
  | "EIO"
  | "EMFILE"
  | "ENOMEM"
  | "EFBIG";

/** ドロップされたデータの頼み（`DropRequestEvent`）。鍵は仕様の `x`・`y`・`Y`。 */
export type DropRequest =
  /** `t=r:x=idx`: 落とされた一覧の `index` 番目（1 始まり）の MIME の種類のデータ。 */
  | { type: "data"; index: number }
  /** `t=r:x=idx:y=subidx`: `text/uri-list`（`index` 番目の種類）の `subIndex` 番目の項目。遠くの機械からのドロップ。 */
  | { type: "entry"; index: number; subIndex: number }
  /** `t=r:Y=handle:x=num`: ディレクトリ（`handle`）の `index` 番目（1 始まり）の項目。 */
  | { type: "dirEntry"; handle: number; index: number }
  /** `t=r:Y=handle`: ディレクトリを読み終えた。`handle` を捨ててよい。答えは要らない。 */
  | { type: "dirClose"; handle: number };

/** ドロップのデータの答えの形（`X` 鍵）。 */
export type DropDataForm =
  /** MIME の種類のデータ。`remote` は、送り元がプログラムとは別の機械か（`text/uri-list` の答えにだけ付ける）。 */
  | { kind: "mime"; remote?: boolean }
  /** 普通のファイルの中身。 */
  | { kind: "file" }
  /** シンボリックリンク。データはリンク先。 */
  | { kind: "symlink" }
  /** ディレクトリ。データは、中の項目を NUL で区切った一覧。`handle` は 0 と 1 以外。 */
  | { kind: "directory"; handle: number };

/** ドラッグの画像（サムネイル）。 */
export interface DragImage {
  /** 形式。`text` は UTF-8 のテキストで、ターミナルが描く。 */
  format: "rgb" | "rgba" | "png" | "text";
  /** 幅（画素）。`text` では、文字の大きさの割合の分子（`基本の大きさ * width / height`）。 */
  width: number;
  /** 高さ（画素）。`text` では、同じ割合の分母。 */
  height: number;
  /** 背景の不透明度（0 から 1024）。`text` のときだけ意味がある。 */
  opacity: number;
  /** 画像のデータ（base64 を戻したあと）。 */
  data: Uint8Array;
}

/**
 * kitty の drag and drop protocol（OSC 72）のイベント。`onDnd` で受ける。
 *
 * ドロップを受ける側:
 * - `dropEnabled` / `dropDisabled`: プログラムがドロップを受け付ける・やめた。ドロップの途中でやめたら、先に `dropFinished`（`operation` が null）が来る。
 * - `dropResponse`: ドラッグが窓の上にある間のプログラムの答え。OS に、受けるか・どの操作かを伝える。
 * - `dropRequest`: ドロップされたデータの頼み。`replyDropData` か `replyDropError` で答える。
 * - `dropFinished`: プログラムがドロップを読み終えた。OS にドロップの完了を伝える。`operation` が null なら取り消し。
 *
 * ドラッグを始める側:
 * - `dragEnabled` / `dragDisabled`: プログラムがドラッグを始められると言った・やめた。
 * - `dragStart`: ドラッグを始める求め。OS のドラッグを始め、`dragStarted` か `dragError` で答える。
 * - `dragImage`: ドラッグの画像を変える。`index` が null なら外す（範囲外の番号も null にそろえてある）。
 * - `dragData` / `dragDataError`: `requestDragData` で頼んだ MIME の種類（0 始まり）のデータ・エラー。分けて届いたデータは、終わり（空のパケット）でそろえて 1 回だけ来る。
 * - `dragEntry`: 遠くの機械へのドラッグで、`requestDragEntry` で頼んだ項目のデータ。
 * - `dragCanceled` / `dragAborted`: プログラムがドラッグを取り消した・エラーで止めた。
 */
export type DndEvent =
  | { type: "dropEnabled"; mimes: string[] }
  | { type: "dropDisabled" }
  | {
      type: "dropResponse";
      operation: DndOperation | null;
      /** 受けたい MIME の種類（好む順）。null は、一覧を変えない。 */
      mimes: string[] | null;
    }
  | { type: "dropRequest"; request: DropRequest }
  | { type: "dropFinished"; operation: DndOperation | null }
  | { type: "dragEnabled"; machineId: string | null }
  | { type: "dragDisabled" }
  | {
      type: "dragStart";
      /** プログラムが許す操作。 */
      operations: { copy: boolean; move: boolean };
      /** 申し出た MIME の種類（順番が番号になる）。 */
      mimes: string[];
      /** 先に送られたデータ。`mimes` と同じ並びで、送られていないものは null。 */
      data: (Uint8Array | null)[];
      images: DragImage[];
      /** 始めるときに使う画像の番号（0 始まり）。無ければ null。 */
      image: number | null;
      /** プログラムの機械の ID。 */
      machineId: string | null;
    }
  | { type: "dragImage"; index: number | null }
  | { type: "dragData"; index: number; data: Uint8Array }
  | {
      type: "dragDataError";
      index: number;
      error: string;
      description: string;
    }
  | {
      type: "dragEntry";
      /** `text/uri-list` の中の番号（1 始まり）。 */
      index: number;
      /** ディレクトリの中の番号（1 始まり）。トップの項目なら 0。 */
      subIndex: number;
      /** 親のディレクトリの handle。トップの項目なら 0。 */
      parent: number;
      form: "file" | "symlink" | "directory";
      /** `form` が `directory` のときの handle。 */
      handle: number;
      data: Uint8Array;
    }
  | { type: "dragCanceled" }
  | { type: "dragAborted"; error: string; description: string };

const DROP_REQUEST_KINDS = { data: 0, entry: 1, dirEntry: 2, dirClose: 3 };

function dropRequestParts(r: DropRequest): [number, number, number] {
  switch (r.type) {
    case "data":
      return [DROP_REQUEST_KINDS.data, r.index, 0];
    case "entry":
      return [DROP_REQUEST_KINDS.entry, r.index, r.subIndex];
    case "dirEntry":
      return [DROP_REQUEST_KINDS.dirEntry, r.handle, r.index];
    case "dirClose":
      return [DROP_REQUEST_KINDS.dirClose, r.handle, 0];
  }
}

// wasm の `dnd_ints` / `dnd_strings` / `dnd_blob` を、イベントにする。並びは wasm 側の説明のとおり。
function dndEventOf(ev: WasmEvent): DndEvent | undefined {
  const ints = ev.dnd_ints;
  const strings = ev.dnd_strings;
  switch (ev.dnd_kind) {
    case 0:
      return { type: "dropEnabled", mimes: strings };
    case 1:
      return { type: "dropDisabled" };
    case 2:
      return {
        type: "dropResponse",
        operation: operationOf(ints[0]),
        mimes: ints[1] ? strings : null,
      };
    case 3: {
      const [kind, a, b] = ints;
      const request: DropRequest | undefined =
        kind === 0
          ? { type: "data", index: a }
          : kind === 1
            ? { type: "entry", index: a, subIndex: b }
            : kind === 2
              ? { type: "dirEntry", handle: a, index: b }
              : kind === 3
                ? { type: "dirClose", handle: a }
                : undefined;
      return request && { type: "dropRequest", request };
    }
    case 4:
      return { type: "dropFinished", operation: operationOf(ints[0]) };
    case 5:
      return {
        type: "dragEnabled",
        machineId: ints[0] ? (strings[0] ?? null) : null,
      };
    case 6:
      return { type: "dragDisabled" };
    case 7: {
      const [ops, image, nMimes, nImages] = ints;
      const hasMachine = ints[4 + nMimes + nImages * 4] !== 0;
      const data: (Uint8Array | null)[] = [];
      for (let i = 0; i < nMimes; i++) {
        data.push(ints[4 + i] ? ev.dnd_blob(i) : null);
      }
      const images: DragImage[] = [];
      for (let i = 0; i < nImages; i++) {
        const at = 4 + nMimes + i * 4;
        images.push({
          format: (["rgb", "rgba", "png", "text"] as const)[ints[at]] ?? "png",
          width: ints[at + 1],
          height: ints[at + 2],
          opacity: ints[at + 3],
          data: ev.dnd_blob(nMimes + i),
        });
      }
      return {
        type: "dragStart",
        operations: { copy: (ops & 1) !== 0, move: (ops & 2) !== 0 },
        mimes: strings.slice(0, nMimes),
        data,
        images,
        image: image < 0 ? null : image,
        machineId: hasMachine ? (strings[nMimes] ?? null) : null,
      };
    }
    case 8:
      return { type: "dragImage", index: ints[0] < 0 ? null : ints[0] };
    case 9:
      return { type: "dragData", index: ints[0], data: ev.dnd_blob(0) };
    case 10:
      return {
        type: "dragDataError",
        index: ints[0],
        error: strings[0] ?? "EUNKNOWN",
        description: strings[1] ?? "",
      };
    case 11:
      return {
        type: "dragEntry",
        index: ints[0],
        subIndex: ints[1],
        parent: ints[2],
        form: (["file", "symlink", "directory"] as const)[ints[3]] ?? "file",
        handle: ints[4],
        data: ev.dnd_blob(0),
      };
    case 12:
      return { type: "dragCanceled" };
    case 13:
      return {
        type: "dragAborted",
        error: strings[0] ?? "EUNKNOWN",
        description: strings[1] ?? "",
      };
    default:
      return undefined;
  }
}

/** ファイル転送のセッションの始まり（`send` と `receive`）。 */
export interface FileTransferStart {
  /** セッションの `id`。答えに渡す。 */
  id: string;
  /** `pw` を戻した文字列（仕様では `sha256:<hash>`）。比べるのは呼ぶ側。無ければ `null`。 */
  bypass: string | null;
  /** `q`。0 は全部返す、1 はエラーだけ、2 は何も返さない。省くのは coneko-term がする。 */
  quiet: 0 | 1 | 2;
  /** `sz`。`receive` が頼むパスの数。 */
  paths: number | null;
}

/** ファイルの種類。 */
export type FileTransferFileType = "regular" | "directory" | "symlink" | "link";

/** ファイル転送のエラーの記号名。知らない値は `EIO`。 */
export type FileTransferErrorName =
  | "EPERM"
  | "ENOENT"
  | "EEXIST"
  | "EISDIR"
  | "ENOTDIR"
  | "EACCES"
  | "EINVAL"
  | "EIO"
  | "EFBIG"
  | "ENOSPC";

/**
 * kitty の file transfer protocol（OSC 5113）で、クライアントが送った命令。`onFileTransfer` で受ける。
 * データの圧縮（`compression: "zlib"`）と rsync の署名・差分は読み解かず、バイト列のまま渡す。
 */
export type FileTransferEvent =
  /** `ac=send`: ファイルを送りたい。`replyFileTransferOk` か `refuseFileTransfer` で答える。 */
  | ({ type: "send" } & FileTransferStart)
  /** `ac=receive`: ファイルを受け取りたい。答え方は `send` と同じ。 */
  | ({ type: "receive" } & FileTransferStart)
  /** `ac=file`: 送るファイルの情報、または受け取るパスの頼み。 */
  | {
      type: "file";
      id: string;
      fileId: string;
      /** 戻したパス（UTF-8。`~/` はホームからの相対）。coneko-term は検査しない。 */
      name: string;
      fileType: FileTransferFileType;
      compression: "none" | "zlib";
      transmission: "simple" | "rsync";
      /** 更新時刻（ナノ秒）。指定なしは `null`。 */
      mtime: bigint | null;
      permissions: number | null;
      size: number | null;
      parent: string | null;
    }
  /** `ac=data` と `ac=end_data`（`end` が真）。 */
  | { type: "data"; id: string; fileId: string; data: Uint8Array; end: boolean }
  /** `ac=finish`: クライアントがセッションを閉じた。 */
  | { type: "finish"; id: string }
  /** `ac=cancel`: クライアントが取り消した。`replyFileTransferCanceled` で答える。 */
  | { type: "cancel"; id: string };

/** 受け取りの一覧の 1 項目。 */
export interface FileTransferListing {
  /** クライアントの頼み（`file`）の `fileId`。ディレクトリを広げた項目も、頼みと同じ値。 */
  // `requestId` と `entryId`、`parent` は safe_string（`[0-9A-Za-z_:./@-]`）の字だけにすること。
  requestId: string;
  /** この項目自身の `fid`。 */
  entryId: string;
  /** 絶対パス。 */
  name: string;
  fileType: FileTransferFileType;
  mtime?: bigint;
  permissions?: number;
  size?: number;
  /** 入っているディレクトリの `entryId`。 */
  parent?: string;
  /** リンクの先（一覧にある先なら、その `entryId` の UTF-8。シンボリックリンクで一覧に無ければ、先のパス）。 */
  data?: Uint8Array;
}

const FILE_TYPES = ["regular", "directory", "symlink", "link"] as const;

function fileTransferEventOf(ev: WasmEvent): FileTransferEvent | undefined {
  const id = ev.ft_id;
  switch (ev.ft_action) {
    case 0:
    case 1: {
      const q = ev.ft_quiet;
      const size = ev.ft_size;
      return {
        type: ev.ft_action === 0 ? "send" : "receive",
        id,
        bypass: ev.ft_bypass ?? null,
        quiet: q === 1 ? 1 : q === 2 ? 2 : 0,
        paths: size ?? null,
      };
    }
    case 2:
      return {
        type: "file",
        id,
        fileId: ev.ft_file_id,
        name: ev.ft_name,
        fileType: FILE_TYPES[ev.ft_file_type] ?? "regular",
        compression: ev.ft_compression === 1 ? "zlib" : "none",
        transmission: ev.ft_transmission === 1 ? "rsync" : "simple",
        mtime: ev.ft_mtime ?? null,
        permissions: ev.ft_permissions ?? null,
        size: ev.ft_size ?? null,
        parent: ev.ft_parent ?? null,
      };
    case 3:
    case 4:
      return {
        type: "data",
        id,
        fileId: ev.ft_file_id,
        data: ev.ft_data(),
        end: ev.ft_action === 4,
      };
    case 5:
      return { type: "finish", id };
    case 6:
      return { type: "cancel", id };
    default:
      return undefined;
  }
}

/** カーソルの状態。 */
export interface CursorState {
  x: number;
  y: number;
  visible: boolean;
  blinking: boolean;
  shape: "block" | "underline" | "bar";
}

/**
 * 1 行ぶんのセル。`cells` は WASM のメモリの上の見え方で、次に `readRow` などを呼ぶか
 * 書き込みで画面が変わるまでしか使えない。セルの並びは `CELL_STRIDE` 個ずつ:
 * 字、幅、前景色、背景色、属性と下線の形、下線の色、複数のセルにまたがる字の印。
 * 複数のセルにまたがる字のセルは、字が 0、幅が 1。印は、下位 16 ビットが `sized` の番号 + 1
 * （0 ならそうではない）、ビット 16 から 23 が四角の左上から右へ何セル目か、ビット 24 から 31 が
 * 下へ何セル目か。
 */
export interface RowView {
  /** セルの数。 */
  readonly length: number;
  /** `length * CELL_STRIDE` 個の値。 */
  readonly cells: Uint32Array;
  /** 次の行へ折り返されているか。 */
  readonly wrapped: boolean;
  /** `CLUSTER_FLAG` つきの字の番号から、書記素を引く。 */
  cluster(index: number): string;
  /**
   * 複数のセルにまたがる字（OSC 66）の四角。番号は、セルの並びの 6 番目の下位 16 ビットから 1 を引いたもの。
   */
  sized(index: number): SizedInfo;
}

/** 複数のセルにまたがる字（kitty の text sizing、OSC 66）の四角。 */
export interface SizedInfo {
  /** 字。 */
  readonly text: string;
  /** 四角の幅（セルの数）。 */
  readonly cols: number;
  /** 大きさ `s`。四角の高さ（セルの数）でもある。 */
  readonly scale: number;
  /** 分数の大きさの分子。 */
  readonly numerator: number;
  /** 分数の大きさの分母（0 なら分数の大きさは無い）。 */
  readonly denominator: number;
  /** 縦の寄せ方（0 上、1 下、2 中央）。分数の大きさのときだけ効く。 */
  readonly vertical: number;
  /** 横の寄せ方（0 左、1 右、2 中央）。分数の大きさのときだけ効く。 */
  readonly horizontal: number;
}

const ANSI_NAMES = [
  "black",
  "red",
  "green",
  "yellow",
  "blue",
  "magenta",
  "cyan",
  "white",
  "brightBlack",
  "brightRed",
  "brightGreen",
  "brightYellow",
  "brightBlue",
  "brightMagenta",
  "brightCyan",
  "brightWhite",
] as const;

const DEFAULT_FONT_FAMILY = "monospace";
const DEFAULT_FONT_SIZE = 15;

// xterm.js 6.0.0 の既定のテーマ（Rust の本体の既定と同じ）
const DEFAULT_THEME: Required<Omit<Theme, "selectionBackground">> = {
  foreground: "#ffffff",
  background: "#000000",
  cursor: "#ffffff",
  black: "#2e3436",
  red: "#cc0000",
  green: "#4e9a06",
  yellow: "#c4a000",
  blue: "#3465a4",
  magenta: "#75507b",
  cyan: "#06989a",
  white: "#d3d7cf",
  brightBlack: "#555753",
  brightRed: "#ef2929",
  brightGreen: "#8ae234",
  brightYellow: "#fce94f",
  brightBlue: "#729fcf",
  brightMagenta: "#ad7fa8",
  brightCyan: "#34e2e2",
  brightWhite: "#eeeeec",
};

/** xterm.js 6.0.0 の選択範囲の既定の色（白の 30%）。 */
const DEFAULT_SELECTION = "#ffffff4d";

/** xterm.js 6.0.0 の `wordSeparator` の既定（Rust の本体の既定と同じ）。 */
const DEFAULT_WORD_SEPARATOR = " ()[]{}',\"`";

function parseSelectionColor(value: string): string {
  const v = value.trim();
  if (!/^#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(v)) {
    throw new Error(
      `coneko-term: unsupported selectionBackground "${value}" (use #rgb, #rgba, #rrggbb or #rrggbbaa)`,
    );
  }
  return v;
}

function parseColor(value: string): [number, number, number] {
  const m = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(value.trim());
  if (!m) {
    throw new Error(`coneko-term: unsupported color "${value}" (use #rgb or #rrggbb)`);
  }
  let h = m[1];
  if (h.length === 3) h = [...h].map((c) => c + c).join("");
  return [
    parseInt(h.slice(0, 2), 16),
    parseInt(h.slice(2, 4), 16),
    parseInt(h.slice(4, 6), 16),
  ];
}

/** 1 回に処理する最大のバイト数。 */
const SLICE_BYTES = 64 * 1024;
/** 1 回の処理で使う時間（ミリ秒）。これを超えたらブラウザーに制御を返す。 */
const SLICE_MS = 8;

interface WriteJob {
  bytes: Uint8Array;
  offset: number;
  done?: () => void;
}

const sleep0 = () => new Promise<void>((r) => setTimeout(r, 0));

/**
 * ターミナル。VT のバイト列を `write` で受け、画面の中身を持つ。DOM を使わないので
 * Node.js でも動く（`open` を呼ぶまで DOM に触らない）。`open` すると Canvas 2D で描く。
 *
 * 使う前に `init()` を呼ぶこと。
 */
export class Terminal {
  readonly options: Readonly<TerminalOptions>;
  private resolvedTheme: Required<Omit<Theme, "selectionBackground">> =
    DEFAULT_THEME;
  private renderer?: CanvasRenderer;
  private scrollbar?: Scrollbar;
  private focused = false;
  /** 画面の一番下から何行さかのぼって見ているか。0 なら一番下。 */
  private viewOffset = 0;
  private lastScrollback = 0;
  private scrollListeners = new Set<(viewportY: number) => void>();
  private cellSizeListeners = new Set<(size: CellSize) => void>();
  private wasm: WasmTerminal;
  private selectionColor = DEFAULT_SELECTION;
  private selection: SelectionModel;
  private selectionListeners = new Set<() => void>();
  private lastSelectionKey = "";
  private lastTrimmed = 0;
  private lastAlt = false;
  /** 押してから離すまで、マウスのイベントを受け持つもの。 */
  private mouseOwner?: "report" | "select";
  private reportButton = 0;
  private wheelPartial = 0;

  /** `dispose` のあとに触ると、分かりやすい例外にする。 */
  private get core(): WasmTerminal {
    if (this.disposed) throw new Error("coneko-term: terminal is disposed");
    return this.wasm;
  }
  private queue: WriteJob[] = [];
  private pumping = false;
  private disposed = false;
  private dataListeners = new Set<(data: Uint8Array) => void>();
  private titleListeners = new Set<(title: string) => void>();
  private bellListeners = new Set<() => void>();
  private notificationListeners = new Set<(n: TerminalNotification) => void>();
  private notificationCloseListeners = new Set<(id: string) => void>();
  private notificationQueryListeners = new Set<(id: string) => void>();
  private notificationAliveListeners = new Set<(id: string) => void>();
  private clipboardWriteListeners = new Set<
    (r: ClipboardWriteRequest) => void
  >();
  private clipboardReadListeners = new Set<
    (r: ClipboardReadRequest) => void
  >();
  private dndListeners = new Set<(e: DndEvent) => void>();
  private fileTransferListeners = new Set<(e: FileTransferEvent) => void>();
  private oscHandlers = new Map<number, Set<(data: string) => void>>();
  private element?: HTMLElement;
  private cleanups: (() => void)[] = [];
  private textarea?: HTMLTextAreaElement;
  private composition?: CompositionHelper;
  private customKeyHandler?: (ev: KeyboardEvent) => boolean;
  private decoder = new TextDecoder();
  private encoder = new TextEncoder();

  constructor(options: TerminalOptions = {}) {
    getMemory();
    this.options = { ...options };
    const cols = Math.max(1, options.cols ?? 80);
    const rows = Math.max(1, options.rows ?? 24);
    this.wasm = new WasmTerminal(rows, cols, options.scrollback ?? 1000);
    this.core.set_cursor_blink(options.cursorBlink ?? false);
    this.applyTheme(options.theme ?? {});
    this.selection = new SelectionModel(this.selectionHost());
    this.lastAlt = this.core.is_alt_screen();
    this.lastTrimmed = this.core.scrollback_trimmed();
  }

  private selectionHost(): SelectionHost {
    // eslint-disable-next-line @typescript-eslint/no-this-alias
    const t = this;
    return {
      get cols() {
        return t.cols;
      },
      totalRows: () => t.scrollbackLength + t.rows,
      trimmed: () => t.core.scrollback_trimmed(),
      wordRange: (row, col) =>
        Array.from(t.core.word_range(row, col, t.wordSeparator)),
      lineRange: (row) => Array.from(t.core.line_range(row)),
    };
  }

  private get wordSeparator(): string {
    return this.options.wordSeparator ?? DEFAULT_WORD_SEPARATOR;
  }

  private applyTheme(theme: Theme): void {
    this.selectionColor =
      theme.selectionBackground === undefined
        ? DEFAULT_SELECTION
        : parseSelectionColor(theme.selectionBackground);
    const t = { ...DEFAULT_THEME, ...stripUndefined(theme) };
    this.resolvedTheme = t;
    const ansi = new Uint8Array(48);
    ANSI_NAMES.forEach((name, i) => ansi.set(parseColor(t[name]), i * 3));
    this.core.set_theme(
      Uint8Array.from(parseColor(t.foreground)),
      Uint8Array.from(parseColor(t.background)),
      Uint8Array.from(parseColor(t.cursor)),
      ansi,
    );
  }

  private get fontFamily(): string {
    return this.options.fontFamily ?? DEFAULT_FONT_FAMILY;
  }

  private get fontSize(): number {
    return this.options.fontSize ?? DEFAULT_FONT_SIZE;
  }

  /** 列の数。 */
  get cols(): number {
    return this.core.cols();
  }

  /** 行の数。 */
  get rows(): number {
    return this.core.rows();
  }

  /** スクロールバックに入っている行の数。 */
  get scrollbackLength(): number {
    return this.core.scrollback_len();
  }

  /** カーソルの状態。 */
  get cursor(): CursorState {
    const [x, y, visible, blinking, shape] = this.core.cursor();
    return {
      x,
      y,
      visible: visible === 1,
      blinking: blinking === 1,
      shape: (["block", "underline", "bar"] as const)[shape],
    };
  }

  /** 代替画面にいるか。 */
  get isAlternateScreen(): boolean {
    return this.core.is_alt_screen();
  }

  /** bracketed paste（2004）が有効か。`paste` が使う。 */
  get bracketedPaste(): boolean {
    return this.core.bracketed_paste();
  }

  /**
   * kitty keyboard protocol のいまのフラグ（アプリが `CSI > u` などで決める）。
   * ビットは 1 disambiguate、2 report event types、4 report alternate keys、
   * 8 report all keys as escape codes、16 report associated text。
   */
  get keyboardFlags(): number {
    return this.core.keyboard_flags();
  }

  /** アプリケーションカーソルキー（DECCKM）か。矢印・Home・End の送り方が変わる。 */
  get applicationCursorKeys(): boolean {
    return this.core.application_cursor_keys();
  }

  /**
   * データを書く。文字列は UTF-8 にする。大きなデータは小分けにして処理し、間でブラウザーに
   * 制御を返す。処理は呼んだ順に行い、このデータを処理し終えたら `done` を呼ぶ。
   * `write` から戻った時点では、まだ画面に反映されていない。
   */
  write(data: string | Uint8Array, done?: () => void): void {
    if (this.disposed) return;
    // 非同期に処理するので、呼ぶ側がバッファーを使い回しても壊れないようにコピーする
    const bytes =
      typeof data === "string" ? this.encoder.encode(data) : data.slice();
    this.queue.push({ bytes, offset: 0, done });
    if (!this.pumping) {
      this.pumping = true;
      queueMicrotask(() => void this.pump());
    }
  }

  private async pump(): Promise<void> {
    try {
      let sliceStart = performance.now();
      while (!this.disposed && this.queue.length > 0) {
        const job = this.queue[0];
        const end = Math.min(job.offset + SLICE_BYTES, job.bytes.length);
        this.core.write(job.bytes.subarray(job.offset, end));
        job.offset = end;
        this.syncViewport();
        this.syncSelection();
        this.afterChange();
        // 聞き手や done の例外で残りの処理を止めない。投げ直して報告する
        this.guard(() => this.flush());
        if (this.disposed) break;
        if (job.offset >= job.bytes.length) {
          this.queue.shift();
          if (job.done) this.guard(job.done);
        }
        if (performance.now() - sliceStart >= SLICE_MS && this.queue.length > 0) {
          await sleep0();
          sliceStart = performance.now();
        }
      }
    } finally {
      this.pumping = false;
    }
  }

  private guard(f: () => void): void {
    try {
      f();
    } catch (e) {
      queueMicrotask(() => {
        throw e;
      });
    }
  }

  /** PTY へ送るバイト列とイベントを、聞き手に渡す。 */
  private flush(): void {
    for (;;) {
      const ev = this.core.next_event();
      if (!ev) break;
      try {
      const data = ev.data;
      switch (ev.kind) {
        case 0:
          for (const l of [...this.bellListeners]) l();
          break;
        case 1: {
          const title = this.decoder.decode(data);
          for (const l of [...this.titleListeners]) l(title);
          break;
        }
        case 4: {
          if (this.notificationListeners.size === 0) break;
          const urgency = ev.notification_urgency;
          const actions = ev.notification_actions;
          const timeout = ev.notification_timeout;
          const n: TerminalNotification = {
            id: ev.notification_id,
            title: ev.notification_title,
            body: ev.notification_body,
            urgency: urgency < 0 ? undefined : (urgency as 0 | 1 | 2),
            actions: { focus: (actions & 1) !== 0, report: (actions & 2) !== 0 },
            when: (["always", "unfocused", "invisible"] as const)[
              ev.notification_when
            ] ?? "always",
            reportClose: ev.notification_report_close,
            appName: ev.notification_app_name,
            types: ev.notification_types,
            timeout: timeout === -2 ? undefined : timeout,
            sound: ev.notification_sound,
            iconNames: ev.notification_icon_names,
            iconCacheId: ev.notification_icon_cache_id,
            buttons: ev.notification_buttons,
          };
          for (const l of [...this.notificationListeners]) l(n);
          break;
        }
        case 5:
        case 6:
        case 7: {
          const id = this.decoder.decode(data);
          const set =
            ev.kind === 5
              ? this.notificationCloseListeners
              : ev.kind === 6
                ? this.notificationQueryListeners
                : this.notificationAliveListeners;
          for (const l of [...set]) l(id);
          break;
        }
        case 8: {
          const id = this.decoder.decode(data);
          if (this.clipboardWriteListeners.size === 0) {
            // 聞き手がいなければ許さない。
            this.core.clipboard_write_reply(id, "EPERM");
            break;
          }
          const location: ClipboardLocation =
            clipboardLocationOf(ev.clipboard_location);
          const items = ev.clipboard_mimes.map((mime, i) => ({
            mime,
            data: ev.clipboard_item_data(i),
          }));
          for (const l of [...this.clipboardWriteListeners]) {
            l({ id, location, items });
          }
          break;
        }
        case 9: {
          const id = this.decoder.decode(data);
          if (this.clipboardReadListeners.size === 0) {
            // 聞き手がいなければ許さない。
            this.core.clipboard_read_reply_error(id, "EPERM");
            break;
          }
          const request: ClipboardReadRequest = {
            id,
            location: clipboardLocationOf(ev.clipboard_location),
            list: ev.clipboard_list,
            mimes: ev.clipboard_mimes,
          };
          for (const l of [...this.clipboardReadListeners]) l(request);
          break;
        }
        case 10: {
          const e = dndEventOf(ev);
          if (!e) break;
          if (this.dndListeners.size === 0) {
            // 聞き手がいなければ許さない。
            if (e.type === "dropRequest") {
              if (e.request.type !== "dirClose") {
                const [kind, a, b] = dropRequestParts(e.request);
                this.core.drop_error_reply(kind, a, b, "EPERM", "");
              }
            } else if (e.type === "dragStart") {
              this.core.drag_error("EPERM", "");
            }
            break;
          }
          for (const l of [...this.dndListeners]) l(e);
          break;
        }
        case 11: {
          const e = fileTransferEventOf(ev);
          if (!e) break;
          if (this.fileTransferListeners.size === 0) {
            // 聞き手がいなければ許さない。取り消しには、取り消したと答える。
            if (e.type === "send" || e.type === "receive") {
              this.core.file_transfer_refuse(e.id);
            } else if (e.type === "cancel") {
              this.core.file_transfer_canceled(e.id);
            }
            break;
          }
          for (const l of [...this.fileTransferListeners]) l(e);
          break;
        }
        case 3: {
          const text = this.decoder.decode(data);
          for (const l of [...(this.oscHandlers.get(ev.ident) ?? [])]) l(text);
          break;
        }
      }
      } finally {
        ev.free();
      }
    }
    const out = this.core.take_output();
    if (out.length > 0) {
      for (const l of [...this.dataListeners]) l(out);
    }
  }

  /**
   * キーボードやマウスなどの入力を、返事と同じ出口（`onData`）から出す。
   * 文字列は UTF-8 にする。
   */
  input(data: string | Uint8Array): void {
    if (this.disposed) return;
    this.core.queue_output(
      typeof data === "string" ? this.encoder.encode(data) : data,
    );
    this.flush();
  }

  /**
   * ユーザーの入力（キー・変換の確定・貼り付け）を出口から出す。`scrollOnUserInput` が false でなく、
   * さかのぼって見ていたら、一番下へ戻す。
   */
  private sendUserInput(data: string): void {
    if (this.disposed || data.length === 0) return;
    if (this.options.scrollOnUserInput !== false) this.scrollToBottom();
    this.input(data);
  }

  /**
   * 貼り付ける。改行は `\r` にそろえ（xterm.js と同じ）、bracketed paste（2004）が有効なら
   * `ESC [200~` と `ESC [201~` で囲んで、出口から出す。`paste` イベントも、これを呼ぶ。
   */
  paste(text: string): void {
    if (this.disposed) return;
    this.sendUserInput(
      bracketTextForPaste(prepareTextForTerminal(text), this.bracketedPaste),
    );
    if (this.textarea) this.textarea.value = "";
  }

  /**
   * キーのイベントを、ターミナルより先に見るハンドラー（xterm.js の `attachCustomKeyEventHandler`）。
   * `false` を返したキーは、ターミナルが処理しない。外すには `undefined`。
   */
  attachCustomKeyEventHandler(
    handler: ((ev: KeyboardEvent) => boolean) | undefined,
  ): void {
    this.customKeyHandler = handler;
  }

  /**
   * PTY へ送るバイト列の出口。問い合わせへの返事（DSR など）と、`input` に渡したものが出る。
   * 渡す `Uint8Array` は聞き手のもの（コピー済み）。
   */
  onData(listener: (data: Uint8Array) => void): Disposable {
    return addListener(this.dataListeners, listener);
  }

  /** 題名が変わった（OSC 0、OSC 2）。 */
  onTitleChange(listener: (title: string) => void): Disposable {
    return addListener(this.titleListeners, listener);
  }

  /** BEL が来た。 */
  onBell(listener: () => void): Disposable {
    return addListener(this.bellListeners, listener);
  }

  /**
   * 通知（kitty の desktop notifications、OSC 99）がそろった。分けて送られた通知は、
   * そろった時点で 1 回だけ呼ぶ。OS の通知を出すのは呼ぶ側。聞き手がいなければ何もしない。
   */
  onNotification(listener: (n: TerminalNotification) => void): Disposable {
    return addListener(this.notificationListeners, listener);
  }

  /** アプリケーションが通知を閉じるよう求めた（`p=close`）。`id` は `i`。 */
  onNotificationClose(listener: (id: string) => void): Disposable {
    return addListener(this.notificationCloseListeners, listener);
  }

  /**
   * アプリケーションが対応を尋ねた（`p=?`）。`replyNotificationQuery` で答える。
   * `id` は `i`（無ければ空文字列）。
   */
  onNotificationQuery(listener: (id: string) => void): Disposable {
    return addListener(this.notificationQueryListeners, listener);
  }

  /**
   * アプリケーションが生きている通知を尋ねた（`p=alive`）。`replyNotificationAlive` で答える。
   */
  onNotificationAlive(listener: (id: string) => void): Disposable {
    return addListener(this.notificationAliveListeners, listener);
  }

  /**
   * 通知が押されたことをアプリケーションへ返す（`onData` から出る）。通知の `actions.report` が
   * 真のときだけ呼ぶ。`button` はボタンを押したときの番号（1 始まり）。`id` が空なら `i=0` で送る（仕様）。
   */
  notificationActivated(id: string, button?: number): void {
    if (this.disposed) return;
    this.core.notification_activated(id, button && button > 0 ? button : 0);
    this.flush();
  }

  /** 通知が閉じられたことを返す。通知の `reportClose` が真のときだけ呼ぶ。 */
  notificationClosed(id: string): void {
    if (this.disposed) return;
    this.core.notification_closed(id);
    this.flush();
  }

  /** `p=?` の問い合わせ（`onNotificationQuery`）に答える。 */
  replyNotificationQuery(id: string, support: NotificationSupport = {}): void {
    if (this.disposed) return;
    this.core.notification_query_reply(
      id,
      support.actions ?? ["focus", "report"],
      support.when ?? ["always", "unfocused", "invisible"],
      support.payloads ?? ["title", "body", "?", "close", "alive"],
      Uint8Array.from(support.urgencies ?? [0, 1, 2]),
      support.closeReport ?? true,
      support.timeout ?? false,
      support.sounds ?? [],
      support.types ?? [],
    );
    this.flush();
  }

  /** `p=alive` の問い合わせ（`onNotificationAlive`）に答える。`ids` はまだ生きている通知の `i`。 */
  replyNotificationAlive(id: string, ids: string[]): void {
    if (this.disposed) return;
    this.core.notification_alive_reply(id, ids);
    this.flush();
  }

  /**
   * クリップボードへの書き込み（kitty の clipboard protocol、OSC 5522）がそろった。分けて
   * 送られたデータは、そろった時点で 1 回だけ呼ぶ。OS のクリップボードに書くかどうかは呼ぶ側が決め、
   * どちらにしても `replyClipboardWrite` で答える。聞き手がいなければ、`EPERM` で答える。
   */
  onClipboardWrite(listener: (r: ClipboardWriteRequest) => void): Disposable {
    return addListener(this.clipboardWriteListeners, listener);
  }

  /**
   * クリップボードの読み取りの頼み（OSC 5522 の `type=read`）が来た。読ませるかどうかは呼ぶ側が決め、
   * `replyClipboardReadTypes`・`replyClipboardReadData`・`replyClipboardReadError` のどれかで答える。
   * 聞き手がいなければ、`EPERM`（許されない）で答える。
   */
  onClipboardRead(listener: (r: ClipboardReadRequest) => void): Disposable {
    return addListener(this.clipboardReadListeners, listener);
  }

  /** 書き込み（`onClipboardWrite`）に答える。`status` の既定は成功（`DONE`）。 */
  replyClipboardWrite(id: string, status: ClipboardWriteStatus = "DONE"): void {
    if (this.disposed) return;
    this.core.clipboard_write_reply(id, status);
    this.flush();
  }

  /** 種類の一覧の頼み（`list` が真）に答える。`mimes` が空なら、データの無い OK と DONE だけを返す。 */
  replyClipboardReadTypes(id: string, mimes: string[]): void {
    if (this.disposed) return;
    this.core.clipboard_read_reply_types(id, mimes);
    this.flush();
  }

  /**
   * データの頼みに答える。`items` は、頼まれた種類のうち渡せるもの。空なら、データの無い OK と DONE だけを返す。
   * 4096 バイトごとのパケットに分けて送る。
   */
  replyClipboardReadData(id: string, items: ClipboardItem[]): void {
    if (this.disposed) return;
    for (const item of items) this.core.clipboard_reply_item(item.mime, item.data);
    this.core.clipboard_read_reply_data(id);
    this.flush();
  }

  /** 読み取りの頼みを断る・失敗を知らせる。読ませないなら `EPERM`。 */
  replyClipboardReadError(id: string, error: ClipboardReadError): void {
    if (this.disposed) return;
    this.core.clipboard_read_reply_error(id, error);
    this.flush();
  }

  /**
   * kitty の drag and drop protocol（OSC 72）のイベントを受ける。種類は `DndEvent` を見ること。
   * OS のドラッグとドロップにつなぐのは呼ぶ側の仕事。聞き手がいないとき、ドロップされたデータの
   * 頼みには `EPERM` で、ドラッグを始める求めにも `EPERM` で答える。
   */
  onDnd(listener: (e: DndEvent) => void): Disposable {
    return addListener(this.dndListeners, listener);
  }

  /**
   * kitty の file transfer protocol（OSC 5113）の、クライアントからの命令を受ける。種類は
   * `FileTransferEvent` を見ること。許すかどうかと、OS のファイルの読み書きは呼ぶ側の仕事で、
   * `replyFileTransfer*`・`refuseFileTransfer` で答える。聞き手がいないとき、`send` と `receive` には
   * `EPERM`（断る）で、`cancel` には `CANCELED` で答える。
   */
  onFileTransfer(listener: (e: FileTransferEvent) => void): Disposable {
    return addListener(this.fileTransferListeners, listener);
  }

  /**
   * 成功の知らせ（`status=OK`）。セッションの始まり（`send`・`receive`）を許すときは `id` だけ。
   * ディレクトリの受け付けと、`end_data` のあとの完了は `fileId`（完了には `size` に書いたバイト数）。
   * 受け取りの一覧を並べ終えたら、`name` にホームディレクトリを付ける。`q` に従って省くことがある。
   */
  replyFileTransferOk(
    id: string,
    opts: { fileId?: string; size?: number; name?: string } = {},
  ): void {
    if (this.disposed) return;
    this.core.file_transfer_ok(id, opts.fileId ?? "", opts.size, opts.name);
    this.flush();
  }

  /** ファイルを受け付けた（`status=STARTED`）。`rsync` は、署名を送れるとき。 */
  replyFileTransferStarted(id: string, fileId: string, rsync = false): void {
    if (this.disposed) return;
    this.core.file_transfer_started(id, fileId, rsync);
    this.flush();
  }

  /** データを書いた（`status=PROGRESS`）。`written` はここまでに書いたバイト数。 */
  replyFileTransferProgress(id: string, fileId: string, written: number): void {
    if (this.disposed) return;
    this.core.file_transfer_progress(id, fileId, written);
    this.flush();
  }

  /** エラーで答える。`fileId` を省くと、セッション全体のエラー。 */
  replyFileTransferError(
    id: string,
    error: FileTransferErrorName,
    opts: { fileId?: string; message?: string } = {},
  ): void {
    if (this.disposed) return;
    this.core.file_transfer_error(
      id,
      opts.fileId ?? "",
      error,
      opts.message ?? "",
    );
    this.flush();
  }

  /** セッションの始まりを断る（`EPERM`）。 */
  refuseFileTransfer(id: string): void {
    if (this.disposed) return;
    this.core.file_transfer_refuse(id);
    this.flush();
  }

  /** 取り消し（`cancel`）に答える（`status=CANCELED`）。 */
  replyFileTransferCanceled(id: string): void {
    if (this.disposed) return;
    this.core.file_transfer_canceled(id);
    this.flush();
  }

  /** 受け取りの一覧に項目を足す。全部並べたら `replyFileTransferOk(id, { name: ホーム })` で終える。 */
  replyFileTransferListing(id: string, entry: FileTransferListing): void {
    if (this.disposed) return;
    this.core.file_transfer_listing(
      id,
      entry.requestId,
      entry.entryId,
      entry.name,
      Math.max(0, FILE_TYPES.indexOf(entry.fileType)),
      entry.mtime,
      entry.permissions,
      entry.size,
      entry.parent,
      entry.data,
    );
    this.flush();
  }

  /**
   * 受け取るファイルのデータを送る。4096 バイトごとのパケットに分ける。`end` が真なら最後を
   * `end_data` にする（`data` が空でも送る）。圧縮や rsync の差分は、作ったバイト列をそのまま渡す。
   */
  replyFileTransferData(
    id: string,
    fileId: string,
    data: Uint8Array,
    end = false,
  ): void {
    if (this.disposed) return;
    this.core.file_transfer_data(id, fileId, data, end);
    this.flush();
  }

  /** プログラムがドロップを受け付けると言っているか。偽の間、`dropMove` などは何もしない。 */
  get dropAccepting(): boolean {
    return this.core.drop_accepting();
  }

  /** プログラムが受け付けると言った MIME の種類（空でもよい）。 */
  get dropAcceptedMimes(): string[] {
    return this.core.drop_accepted_mimes();
  }

  /**
   * プログラムが渡した機械の ID（ドロップ側）。自分の機械の ID と比べて別の機械なら、
   * `text/uri-list` の答えに `remote: true` を付ける。知らない版の ID は別の機械として扱う。
   */
  get dropMachineId(): string | null {
    return this.core.drop_machine_id() ?? null;
  }

  /**
   * ドラッグが窓の上を動いた。`mimes` はドラッグされているものの MIME の種類。MIME の一覧は、
   * 最初の移動と変わったときだけ PTY へ付ける。受け付けていないとき・位置が負のときは何もせず、false を返す。
   */
  dropMove(
    location: DndLocation,
    mimes: string[],
    allowed: DndOperations = { copy: true, move: true },
  ): boolean {
    if (this.disposed) return false;
    const ok = this.core.drop_move(
      location.x,
      location.y,
      location.pixelX,
      location.pixelY,
      mimes,
      operationBits(allowed),
    );
    this.flush();
    return ok;
  }

  /** ドラッグが窓から出た（位置が -1 の移動を送る）。窓の上に無かったときは何もせず、false を返す。 */
  dropLeave(): boolean {
    if (this.disposed) return false;
    const ok = this.core.drop_leave();
    this.flush();
    return ok;
  }

  /**
   * ドロップされた。`mimes` は落とされたものの MIME の種類で、空ではいけない。`fromThisWindow` は、
   * ドラッグがこの窓から始まったか（真なら、遠くの機械向けのファイルの読み取りを `EPERM` で断る）。
   */
  dropDrop(
    location: DndLocation,
    mimes: string[],
    allowed: DndOperations = { copy: true, move: true },
    fromThisWindow = false,
  ): boolean {
    if (this.disposed) return false;
    const ok = this.core.drop_drop(
      location.x,
      location.y,
      location.pixelX,
      location.pixelY,
      mimes,
      operationBits(allowed),
      fromThisWindow,
    );
    this.flush();
    return ok;
  }

  /**
   * `dropRequest` に、データで答える。データは全部を一度に渡す。4096 バイトのパケットに分け、
   * 終わりの空のパケットまで送る。
   */
  replyDropData(
    request: DropRequest,
    form: DropDataForm,
    data: Uint8Array,
  ): void {
    if (this.disposed) return;
    const [kind, a, b] = dropRequestParts(request);
    const code =
      form.kind === "mime"
        ? 0
        : form.kind === "file"
          ? 1
          : form.kind === "symlink"
            ? 2
            : 3;
    this.core.drop_data_reply(
      kind,
      a,
      b,
      code,
      form.kind === "directory" ? form.handle : 0,
      form.kind === "mime" && form.remote === true,
      data,
    );
    this.flush();
  }

  /** `dropRequest` に、エラーで答える。`error` は POSIX の記号名。 */
  replyDropError(
    request: DropRequest,
    error: DndErrorName,
    description = "",
  ): void {
    if (this.disposed) return;
    const [kind, a, b] = dropRequestParts(request);
    this.core.drop_error_reply(kind, a, b, error, description);
    this.flush();
  }

  /** プログラムがドラッグを始められると言っているか。 */
  get dragOffersEnabled(): boolean {
    return this.core.drag_offers_enabled();
  }

  /** プログラムが渡した機械の ID（ドラッグ側）。 */
  get dragMachineId(): string | null {
    return this.core.drag_machine_id() ?? null;
  }

  /**
   * ドラッグの身振りを見つけた。プログラムへ位置を送り、続く申し出を 1 回だけ受ける。申し出ていない
   * とき・ドラッグの途中のときは何もせず、false を返す。
   */
  dragGesture(location: DndLocation): boolean {
    if (this.disposed) return false;
    const ok = this.core.drag_gesture(
      location.x,
      location.y,
      location.pixelX,
      location.pixelY,
    );
    this.flush();
    return ok;
  }

  /** `dragStart` に答える: OS のドラッグが始まった。 */
  dragStarted(): boolean {
    if (this.disposed) return false;
    const ok = this.core.drag_started();
    this.flush();
    return ok;
  }

  /**
   * `dragStart` に失敗で答える、またはドラッグを止める。すでにユーザーが取り消していたら
   * `EPERM`、画像が大きすぎれば `EFBIG`。ドラッグの状態は捨てる。
   */
  dragError(error: DndErrorName, description = ""): boolean {
    if (this.disposed) return false;
    const ok = this.core.drag_error(error, description);
    this.flush();
    return ok;
  }

  /** ドラッグが受け入れられた。`index` は、相手が欲しがりそうな MIME の種類（0 始まり）。 */
  dragAccepted(index: number): boolean {
    if (this.disposed) return false;
    const ok = this.core.drag_accepted(index);
    this.flush();
    return ok;
  }

  /** 相手が行いそうな操作が変わった。null は受け付けない。 */
  dragAction(operation: DndOperation | null): boolean {
    if (this.disposed) return false;
    const ok = this.core.drag_action(
      operation === "copy" ? 1 : operation === "move" ? 2 : 0,
    );
    this.flush();
    return ok;
  }

  /** ドラッグが相手に落とされた。このあと `requestDragData` で、データを頼む。 */
  dragDropped(): boolean {
    if (this.disposed) return false;
    const ok = this.core.drag_dropped();
    this.flush();
    return ok;
  }

  /** ドラッグが終わった。`canceled` が真ならユーザーが取り消した。ドラッグの状態は捨てる。 */
  dragFinished(canceled = false): boolean {
    if (this.disposed) return false;
    const ok = this.core.drag_finished(canceled);
    this.flush();
    return ok;
  }

  /** MIME の種類 `index`（0 始まり）のデータをプログラムに頼む。答えは `dragData` か `dragDataError`。 */
  requestDragData(index: number): boolean {
    if (this.disposed) return false;
    const ok = this.core.drag_data_request(index);
    this.flush();
    return ok;
  }

  /** 遠くの機械へのドラッグで、`text/uri-list` の `index` 番目（1 始まり）の項目を頼む。答えは `dragEntry`。 */
  requestDragEntry(index: number): boolean {
    if (this.disposed) return false;
    const ok = this.core.drag_entry_request(index);
    this.flush();
    return ok;
  }

  /**
   * OSC の番号 `ident` のハンドラーを登録する（xterm.js の `parser.registerOscHandler` に当たる）。
   * `data` は `OSC ident ;` のあとの文字列（UTF-8）。登録した番号は、ターミナルの内蔵の処理の
   * 代わりにハンドラーが受ける。同じ番号に複数登録できる。
   */
  registerOscHandler(
    ident: number,
    handler: (data: string) => void,
  ): Disposable {
    let set = this.oscHandlers.get(ident);
    if (!set) {
      set = new Set();
      this.oscHandlers.set(ident, set);
      this.core.register_osc(ident);
    }
    set.add(handler);
    return {
      dispose: () => {
        const s = this.oscHandlers.get(ident);
        if (!s?.delete(handler) || s.size > 0) return;
        this.oscHandlers.delete(ident);
        if (!this.disposed) this.core.unregister_osc(ident);
      },
    };
  }

  /** 初期状態に戻す。書き込み待ちのデータは捨てる（`done` は呼ばない）。 */
  reset(): void {
    this.queue.length = 0;
    this.core.reset();
    this.selection.clear();
    this.syncSelection();
    this.viewOffset = 0;
    this.lastScrollback = this.core.scrollback_len();
    this.afterChange();
  }

  /** 大きさを変える。 */
  resize(cols: number, rows: number): void {
    this.core.resize(Math.max(1, rows), Math.max(1, cols));
    // 折り返しの組み直しで行の位置が変わるので、選択は消す
    this.selection.clear();
    this.syncSelection();
    this.lastScrollback = this.core.scrollback_len();
    this.viewOffset = Math.min(this.viewOffset, this.lastScrollback);
    this.renderer?.resize();
    this.afterChange();
  }

  /**
   * 欄（`open` に渡した要素）の大きさに合わせて列と行の数を決め、大きさを変える
   * （xterm.js の FitAddon の `fit()` と同じ式。スクロールバーの幅を引く）。
   * 返すのは変えたあとの大きさ。`open` の前や、欄が見えていない（大きさが 0）ときや、
   * フォントの大きさがまだ測れていないときは、何もせず、いまの大きさを返す。
   */
  fit(): { cols: number; rows: number } {
    const dims = this.proposeDimensions();
    if (dims && (dims.cols !== this.cols || dims.rows !== this.rows)) {
      this.resize(dims.cols, dims.rows);
    }
    return { cols: this.cols, rows: this.rows };
  }

  /** `fit()` が使う列と行の数を、変えずに出す。 */
  proposeDimensions(): { cols: number; rows: number } | undefined {
    const host = this.host;
    const cell = this.renderer?.cellSize;
    if (!host || !cell) return undefined;
    const style = host.ownerDocument.defaultView?.getComputedStyle(host);
    const px = (v: string | undefined) => Number.parseFloat(v ?? "") || 0;
    const width =
      host.clientWidth - px(style?.paddingLeft) - px(style?.paddingRight);
    const height =
      host.clientHeight - px(style?.paddingTop) - px(style?.paddingBottom);
    return proposeDimensions({
      width,
      height,
      cellWidth: cell.width,
      cellHeight: cell.height,
      scrollbarWidth: this.options.scrollback === 0 ? 0 : SCROLLBAR_WIDTH,
    });
  }

  /** フォーカスがあるか（`setFocused` か、要素のフォーカスで決まる）。 */
  get isFocused(): boolean {
    return this.focused;
  }

  /** ウィンドウの代わりに、フォーカスを得た・失ったことを知らせる。 */
  setFocused(focused: boolean): void {
    this.focused = focused;
    this.core.set_focus(focused);
    this.flush();
    this.renderer?.touch();
  }

  /**
   * 欄の要素に付ける。canvas・スクロールバー・見えない `textarea`（フォーカスを受ける）を持つ要素を作り、
   * Canvas 2D で描き始める。字の幅と高さは、フォントを読み終えてから測り直す。
   * 欄の大きさに合わせるには `fit()` を呼ぶ。
   */
  open(host: HTMLElement): void {
    if (this.element) throw new Error("coneko-term: already opened");
    const doc = host.ownerDocument;
    const el = doc.createElement("div");
    el.className = "coneko-term";
    Object.assign(el.style, {
      position: "relative",
      width: "100%",
      height: "100%",
      overflow: "hidden",
      outline: "none",
      background: this.resolvedTheme.background,
    });
    const canvas = doc.createElement("canvas");
    canvas.className = "coneko-term-screen";
    Object.assign(canvas.style, { position: "absolute", left: "0", top: "0" });
    el.appendChild(canvas);
    this.cleanups.push(() => el.remove());
    host.appendChild(el);
    this.element = el;
    this.host = host;
    this.syncPointerShape();

    this.scrollbar = new Scrollbar(el, {
      total: () => this.scrollbackLength + this.rows,
      rows: () => this.rows,
      viewportY: () => this.viewportY,
      scrollToLine: (n) => this.scrollToLine(n),
      scrollLines: (n) => this.scrollLines(n),
    });
    this.cleanups.push(() => this.scrollbar?.dispose());

    const renderer = new CanvasRenderer(canvas, this, this.renderOptions());
    this.renderer = renderer;
    this.syncCellSize();
    renderer.onCellSizeChange((size) => {
      this.syncCellSize();
      for (const l of [...this.cellSizeListeners]) l(size);
    });
    this.cleanups.push(() => renderer.dispose());
    this.cleanups.push(
      this.bindPointer(canvas),
      this.bindWheel(el, canvas),
      this.bindCopy(el),
    );
    this.setupInput(doc, el);
    this.afterChange();

    this.cleanups.push(this.watchFonts(doc, renderer));
    this.cleanups.push(this.watchDevicePixelRatio(doc, renderer));
  }

  /**
   * 見えない `textarea` を置いて、キー・変換・貼り付け・フォーカスを受ける。
   * 変換中の字は、カーソルの位置に重ねて見せる。
   */
  private setupInput(doc: Document, el: HTMLElement): void {
    const textarea = createInputTextarea(doc);
    const { element: viewEl, view } = createCompositionView(
      doc,
      this.resolvedTheme.background,
      this.resolvedTheme.foreground,
    );
    el.append(textarea, viewEl);
    const composition = new CompositionHelper(
      textarea,
      view,
      (data) => this.sendUserInput(data),
      () => this.compositionAnchor(),
      () => ({ family: this.fontFamily, size: this.fontSize }),
    );
    this.composition = composition;
    this.textarea = textarea;
    const keys = this.createKeyHandler(textarea, composition);
    this.bindInputEvents(textarea, el, keys, composition);
  }

  /** 変換中の字を出す位置。さかのぼって見ていて、カーソルが見えないときは `undefined`。 */
  private compositionAnchor(): CompositionAnchor | undefined {
    const cell = this.renderer?.cellSize;
    if (!cell || this.viewportOffset !== 0) return undefined;
    const c = this.cursor;
    return {
      left: Math.min(c.x, this.cols - 1) * cell.width,
      top: c.y * cell.height,
      cellHeight: cell.height,
    };
  }

  private createKeyHandler(
    textarea: HTMLTextAreaElement,
    composition: CompositionHelper,
  ): KeyHandler {
    const platform = detectPlatform();
    return new KeyHandler(
      {
        send: (data) => this.sendUserInput(data),
        applicationCursorKeys: () => this.applicationCursorKeys,
        keyboardFlags: () => this.keyboardFlags,
        scrollPages: (n) => this.scrollLines(n * (this.rows - 1)),
        scrollToBottom: () => {
          if (this.options.scrollOnUserInput !== false) this.scrollToBottom();
        },
        isMac: platform.isMac,
        isWindows: platform.isWindows,
        macOptionIsMeta: this.options.macOptionIsMeta ?? false,
        customKeyHandler: () => this.customKeyHandler,
      },
      textarea,
      composition,
    );
  }

  private bindInputEvents(
    textarea: HTMLTextAreaElement,
    el: HTMLElement,
    keys: KeyHandler,
    composition: CompositionHelper,
  ): void {
    const on = <E extends Event>(
      target: EventTarget,
      type: string,
      f: (ev: E) => void,
      capture = false,
    ) => {
      const h = f as EventListener;
      target.addEventListener(type, h, capture);
      this.cleanups.push(() => target.removeEventListener(type, h, capture));
    };
    on<KeyboardEvent>(textarea, "keydown", (e) => void keys.keyDown(e), true);
    on<KeyboardEvent>(textarea, "keyup", (e) => keys.keyUp(e), true);
    on<KeyboardEvent>(textarea, "keypress", (e) => void keys.keyPress(e), true);
    on<InputEvent>(textarea, "input", (e) => void keys.input(e), true);
    on<CompositionEvent>(textarea, "compositionstart", () =>
      composition.compositionstart(),
    );
    on<CompositionEvent>(textarea, "compositionupdate", (e) =>
      composition.compositionupdate(e.data),
    );
    on<CompositionEvent>(textarea, "compositionend", () =>
      composition.compositionend(),
    );
    const onPaste = (e: ClipboardEvent) => {
      e.stopPropagation();
      e.preventDefault();
      const text = e.clipboardData?.getData("text/plain");
      if (text) this.paste(text);
    };
    on<ClipboardEvent>(textarea, "paste", onPaste);
    on<ClipboardEvent>(el, "paste", onPaste);
    on<FocusEvent>(textarea, "focus", () => this.setFocused(true));
    on<FocusEvent>(textarea, "blur", () => {
      keys.reset();
      textarea.value = "";
      this.setFocused(false);
    });
    // 欄を押したら、フォーカスは `textarea` へ（押した先に動かさない）。マウスの報告や選択は #10
    on<MouseEvent>(el, "mousedown", (e) => {
      if (e.target !== textarea) e.preventDefault();
      textarea.focus();
    });
  }

  /** フォントを読み終えてから測り直す（読む前は代わりのフォントの大きさになる）。戻り値は後始末。 */
  private watchFonts(doc: Document, renderer: CanvasRenderer): () => void {
    const fonts = doc.fonts;
    if (!fonts) return () => {};
    const remeasure = () => {
      if (!this.disposed) renderer.measure();
    };
    const spec = `${this.fontSize}px ${this.fontFamily}`;
    void fonts.load(spec).then(remeasure, () => {});
    void fonts.ready.then(remeasure, () => {});
    fonts.addEventListener("loadingdone", remeasure);
    return () => fonts.removeEventListener("loadingdone", remeasure);
  }

  /** `devicePixelRatio` が変わったら（別のディスプレイへ動かした、ズームした）測り直す。戻り値は後始末。 */
  private watchDevicePixelRatio(
    doc: Document,
    renderer: CanvasRenderer,
  ): () => void {
    const win = doc.defaultView;
    if (!win?.matchMedia) return () => {};
    let mq: MediaQueryList | undefined;
    const watch = () => {
      mq?.removeEventListener("change", onChange);
      mq = win.matchMedia(`(resolution: ${win.devicePixelRatio}dppx)`);
      mq.addEventListener("change", onChange);
    };
    const onChange = () => {
      watch();
      if (!this.disposed) renderer.measure();
    };
    watch();
    return () => mq?.removeEventListener("change", onChange);
  }

  private renderOptions() {
    const t = this.resolvedTheme;
    return {
      fontFamily: this.fontFamily,
      fontSize: this.fontSize,
      lineHeight: this.options.lineHeight ?? 1,
      palette: new Palette(
        ANSI_NAMES.map((n) => t[n]),
        t.foreground,
        t.background,
        t.cursor,
        this.selectionColor,
      ),
    };
  }

  /** セルの大きさ（CSS ピクセル）。`open` の前は `undefined`。 */
  get cellSize(): CellSize | undefined {
    return this.renderer?.cellSize;
  }

  /** セルの大きさが変わった（フォントを読み終えた、`devicePixelRatio` が変わった）。`fit()` を呼び直す目印に使う。 */
  onCellSizeChange(listener: (size: CellSize) => void): Disposable {
    return addListener(this.cellSizeListeners, listener);
  }

  // ---- 表示位置（viewport）

  /** 実際に使う、さかのぼっている行数。代替画面では常に 0。 */
  get viewportOffset(): number {
    return this.core.is_alt_screen() ? 0 : this.viewOffset;
  }

  /** 一番上に見えている行の番号。0 がスクロールバックの一番古い行で、一番下を見ているときは `scrollbackLength`。 */
  get viewportY(): number {
    return this.scrollbackLength - this.viewportOffset;
  }

  /** 一番下を見ているか（新しい出力に付いていく状態）。 */
  get isAtBottom(): boolean {
    return this.viewportOffset === 0;
  }

  /** `n` 行ぶん動かす。負の数で上（古い方）、正の数で下。 */
  scrollLines(n: number): void {
    this.setViewOffset(this.viewOffset - Math.trunc(n));
  }

  /** 一番上に見える行を `line`（0 がスクロールバックの一番古い行）にする。 */
  scrollToLine(line: number): void {
    this.setViewOffset(this.scrollbackLength - Math.trunc(line));
  }

  /** 一番古い行へ。 */
  scrollToTop(): void {
    this.setViewOffset(this.scrollbackLength);
  }

  /** 一番下へ。 */
  scrollToBottom(): void {
    this.setViewOffset(0);
  }

  /** 表示位置が変わった。引数は新しい `viewportY`。 */
  onScroll(listener: (viewportY: number) => void): Disposable {
    return addListener(this.scrollListeners, listener);
  }

  private setViewOffset(offset: number): void {
    if (this.disposed) return;
    const before = this.viewportY;
    this.viewOffset = Math.max(0, Math.min(offset, this.scrollbackLength));
    if (this.viewportY === before) return;
    this.renderer?.invalidate();
    this.afterChange();
    for (const l of [...this.scrollListeners]) this.guard(() => l(this.viewportY));
  }

  /** 書き込みでスクロールバックが増えたとき、さかのぼって見ているなら同じ内容を見続ける。 */
  private syncViewport(): void {
    const len = this.core.scrollback_len();
    const grown = len - this.lastScrollback;
    this.lastScrollback = len;
    if (this.viewOffset > 0 && grown > 0) this.viewOffset += grown;
    this.viewOffset = Math.min(this.viewOffset, len);
  }

  /** 描画とスクロールバーに、画面が変わったことを知らせる。 */
  private afterChange(): void {
    this.syncPointerShape();
    const r = this.renderer;
    if (!r) return;
    r.touch();
    this.composition?.updateCompositionElements(true);
    const cell = r.cellSize;
    this.scrollbar?.update(
      this.rows * cell.height,
      !this.core.is_alt_screen(),
    );
  }

  /**
   * プログラムが OSC 22（kitty の mouse pointer shapes）で決めた、ポインターの形。CSS の `cursor` の値で、
   * 決まっていなければ `undefined`。マウスの報告が有効かどうかには関わらない。
   * `open` した要素の `cursor` には、これが自動で反映される。
   */
  get pointerShape(): string | undefined {
    return this.core.pointer_shape() ?? undefined;
  }

  private syncPointerShape(): void {
    const el = this.element;
    if (!el || this.disposed) return;
    const shape = this.pointerShape ?? "";
    if (el.style.cursor !== shape) el.style.cursor = shape;
  }

  /** 画面の `y` 行目（表示位置を考えた、いま見えている行）のセル。 */
  readViewportRow(y: number): RowView {
    const i = this.viewportY + y;
    const sb = this.scrollbackLength;
    return i < sb ? this.readScrollbackRow(i) : this.readRow(i - sb);
  }

  /** 欄の要素。 */
  private host?: HTMLElement;

  /** `open` で作った要素。`open` の前は `undefined`。 */
  get hostElement(): HTMLElement | undefined {
    return this.element;
  }

  /** 見えない `textarea` にフォーカスを当てる。`open` の前は何もしない。 */
  focus(): void {
    this.textarea?.focus();
  }

  /** フォーカスを外す。`open` の前は何もしない。 */
  blur(): void {
    this.textarea?.blur();
  }

  /** セルの画素の大きさをコアに教える（`c`・`r` が無い画像の置く大きさを決めるのに使う）。 */
  private syncCellSize(): void {
    const cell = this.renderer?.deviceCellSize;
    if (cell) this.core.set_cell_size(cell.width, cell.height);
  }

  /** 画像の配置の並び（描画が使う）。`PLACEMENT_STRIDE` 個ずつ。 */
  readPlacements(): Float64Array {
    return this.core.placements();
  }

  /** 持っている画像の `[ID, 通し番号]` の並び（描画が使う）。 */
  imageSerials(): Float64Array {
    return this.core.image_serials();
  }

  /** 画像の情報 `[通し番号, 形式, 圧縮, 幅, 高さ]`（描画が使う）。画像が無ければ空。 */
  imageInfo(id: number): Float64Array {
    return this.core.image_info(id);
  }

  /** 画像のバイト列（描画が使う）。 */
  imageData(id: number): Uint8Array {
    return this.core.image_data(id);
  }

  /** 画像のアニメーションの状態 `[コマの数, いまのコマ, 再生の状態（1 止まる、2 待ちながら回す、3 回す）, いまのコマの表示時間（ミリ秒）, 操作の数]`（描画が使う）。画像が無ければ空。 */
  animationInfo(id: number): Float64Array {
    return this.core.animation_info(id);
  }

  /** いまのコマの表示時間が過ぎたと知らせ、次のコマへ進める（描画が使う）。進んだら `true`。 */
  animationStep(id: number): boolean {
    return this.core.animation_step(id);
  }

  /** コマを作る `index` 番目の操作（描画が使う）。並びは `WasmTerminal.frame_op` の説明を参照。 */
  frameOp(id: number, index: number): Float64Array {
    return this.core.frame_op(id, index);
  }

  /** `frameOp` の「データを置く」操作のバイト列（描画が使う）。 */
  frameOpData(id: number, index: number): Uint8Array {
    return this.core.frame_op_data(id, index);
  }

  /** 変わった行（見えている画面の行番号）を返し、「変わった」の印を消す。 */
  takeDirtyRows(): number[] {
    return Array.from(this.core.take_dirty_rows());
  }

  /** 見えている画面の `y` 行目のセル。 */
  readRow(y: number): RowView {
    return this.view(this.core.read_row(y));
  }

  /** スクロールバックの `i` 行目（0 が一番古い）のセル。 */
  readScrollbackRow(i: number): RowView {
    return this.view(this.core.read_scrollback_row(i));
  }

  /** 見えている画面の `y` 行目の字（空のセルは空白）。末尾の空白は残す。 */
  getRowText(y: number): string {
    const row = this.readRow(y);
    let s = "";
    for (let x = 0; x < row.length; x++) {
      const cp = row.cells[x * CELL_STRIDE];
      const width = row.cells[x * CELL_STRIDE + 1];
      if (width === 0) continue;
      const mark = row.cells[x * CELL_STRIDE + 6];
      if (mark & 0xffff) {
        // 四角の左上だけが字を持つ
        if (mark >>> 16 === 0) s += row.sized((mark & 0xffff) - 1).text;
        continue;
      }
      if (cp === 0) s += " ";
      else if (cp & CLUSTER_FLAG) s += row.cluster(cp & ~CLUSTER_FLAG);
      else s += String.fromCodePoint(cp);
    }
    return s;
  }

  private view(length: number): RowView {
    const core = this.core;
    const wrapped = core.row_wrapped();
    const cells = new Uint32Array(
      getMemory().buffer,
      core.cells_ptr() as unknown as number,
      length * CELL_STRIDE,
    );
    return {
      length,
      cells,
      wrapped,
      cluster: (i) => core.cluster(i),
      sized: (i) => {
        const [cols, scale, numerator, denominator, vertical, horizontal] =
          core.sized_info(i);
        return {
          text: core.sized_text(i),
          cols,
          scale,
          numerator,
          denominator,
          vertical,
          horizontal,
        };
      },
    };
  }

  // ---- 選択

  /** 選択範囲が変わった（広げた、消えた、書き込みで行がずれた）。 */
  onSelectionChange(listener: () => void): Disposable {
    return addListener(this.selectionListeners, listener);
  }

  /** 選択があるか。 */
  hasSelection(): boolean {
    return this.selection.range() !== undefined;
  }

  /**
   * 選択範囲。位置は、スクロールバックの一番古い行を 0 とした行と列で、`end` は含まない
   * （代替画面では画面の 0 行目が 0）。選択が無ければ `undefined`。
   */
  getSelectionRange(): SelectionRange | undefined {
    return this.selection.range();
  }

  /**
   * 選択した字。折り返しでつながった行は改行を入れずにつなぎ、行末の空のセルは捨てる。
   * 選択が無ければ空文字列。
   */
  getSelection(): string {
    const r = this.selection.range();
    if (!r) return "";
    return this.core.selection_text(
      r.start.row,
      r.start.col,
      r.end.row,
      r.end.col,
    );
  }

  /** 全部（スクロールバックも）を選ぶ。 */
  selectAll(): void {
    this.selection.selectAll();
    this.syncSelection();
  }

  /** 選択を消す。 */
  clearSelection(): void {
    this.selection.clear();
    this.syncSelection();
  }

  /**
   * 選択した字をクリップボードのデータに入れる（`copy` イベントの `clipboardData` を渡す）。
   * 選択が無ければ何もせず `false`。`open` した要素は、自分で `copy` イベントを受けて
   * これを呼ぶ（xterm.js と同じ）ので、ふつうは呼ばなくてよい。
   */
  copySelection(data: { setData(format: string, data: string): void }): boolean {
    if (!this.hasSelection()) return false;
    data.setData("text/plain", this.getSelection());
    return true;
  }

  /** 選択範囲が変わっていたら、描き直して、聞き手に知らせる。書き込みのあとにも呼ぶ。 */
  private syncSelection(): void {
    if (this.disposed) return;
    const trimmed = this.core.scrollback_trimmed();
    const alt = this.core.is_alt_screen();
    // 主画面と代替画面は別の行の並びなので、切り替わったら選択は消す
    if (alt !== this.lastAlt) this.selection.clear();
    this.lastAlt = alt;
    const moved = trimmed !== this.lastTrimmed;
    this.lastTrimmed = trimmed;
    const r = this.selection.range();
    const key = r
      ? `${r.start.row}:${r.start.col}-${r.end.row}:${r.end.col}`
      : "";
    if (key === this.lastSelectionKey && !(moved && r)) return;
    this.lastSelectionKey = key;
    this.renderer?.invalidate();
    for (const l of [...this.selectionListeners]) this.guard(l);
  }

  /** 表示位置を考えた、いま見えている `y` 行目の選択範囲 `[開始の列, 終わりの列)`（描画が使う）。 */
  selectionSpan(y: number): [number, number] | undefined {
    const r = this.selection.range();
    if (!r) return undefined;
    const row = this.viewportY + y;
    if (row < r.start.row || row > r.end.row) return undefined;
    const c0 = row === r.start.row ? r.start.col : 0;
    const c1 = row === r.end.row ? r.end.col : this.cols;
    return c1 > c0 ? [c0, c1] : undefined;
  }

  // ---- マウス

  /** マウスの報告が有効か（アプリが `CSI ? 1000 h` などで頼んだ）。 */
  get mouseReporting(): boolean {
    return this.core.mouse_reporting();
  }

  /**
   * ポインターのイベントを渡す。`open` した要素は、自分で DOM のイベントを受けて、これを呼ぶ。
   *
   * - マウスの報告が有効なら、押した・離した・動かした（ボタンを押したまま、または何も押さずに。
   *   モードによる）を PTY へ送るバイト列にして、`onData` の出口から出す。Shift を押して押した
   *   ときは報告せず、選択になる（xterm.js と同じ）。
   * - そうでなければ、左ボタンのドラッグで範囲、ダブルクリックで単語、トリプルクリックで行を選ぶ。
   *   Shift を押してクリックすると、いまの選択を広げる。
   *
   * 押してから離すまでは、押したときに決めた側が受け持つ。受け持った（ブラウザーの既定の動きを
   * 止めてよい）ときは `true`。
   */
  handleMouse(ev: MouseInput): boolean {
    if (this.disposed) return false;
    const col = clamp(Math.trunc(ev.col), 0, this.cols - 1);
    const row = clamp(Math.trunc(ev.row), 0, this.rows - 1);
    const shift = ev.shiftKey === true;
    const report = (button: number, action: number): boolean =>
      this.core.report_mouse(
        button,
        action,
        col,
        row,
        Math.max(0, Math.floor(ev.x ?? 0)),
        Math.max(0, Math.floor(ev.y ?? 0)),
        (shift ? 1 : 0) | (ev.altKey ? 2 : 0) | (ev.ctrlKey ? 4 : 0),
      );
    const flushReport = (sent: boolean) => {
      if (sent) this.flush();
    };

    if (ev.type === "down") {
      if (this.mouseOwner) return true;
      if (this.core.mouse_reporting() && !shift) {
        const button = domButton(ev.button);
        this.mouseOwner = "report";
        this.reportButton = button;
        flushReport(report(button, 0));
        return true;
      }
      if (ev.button !== 0) return false;
      this.mouseOwner = "select";
      const at = { row: this.viewportY + row, col };
      if (shift && this.selection.range()) {
        this.selection.extendSelection(at);
      } else {
        const n = ((Math.max(1, ev.clickCount ?? 1) - 1) % 3) + 1;
        this.selection.start(n === 1 ? "cell" : n === 2 ? "word" : "line", at);
      }
      this.syncSelection();
      return true;
    }

    if (ev.type === "move") {
      if (this.mouseOwner === "report") {
        flushReport(report(this.reportButton, 2));
        return true;
      }
      if (this.mouseOwner === "select") {
        this.selection.extend({ row: this.viewportY + row, col });
        this.syncSelection();
        return true;
      }
      if (this.core.mouse_reporting() && !shift) {
        flushReport(report(3, 2));
        return true;
      }
      return false;
    }

    // up
    const owner = this.mouseOwner;
    if (!owner) return false;
    this.mouseOwner = undefined;
    if (owner === "report") flushReport(report(this.reportButton, 1));
    return true;
  }

  /**
   * ホイールのイベントを渡す。`open` した要素は、自分で DOM のイベントを受けて、これを呼ぶ。
   *
   * - マウスの報告が有効なら、ホイールとして報告する（1 回の `wheel` に 1 つ）。
   * - 代替画面でマウスの報告が無効なら、動かした行の数だけ上下の矢印キーを送る
   *   （アプリケーションカーソルキーなら `ESC O A`、そうでなければ `ESC [ A`）。
   * - 通常の画面なら、表示位置を動かす。
   *
   * 縦に回さないとき（`deltaY` が 0 か Shift つき）は何もしない。ブラウザーの既定の動きを
   * 止めてよいときは `true`。
   */
  handleWheel(ev: WheelInput): boolean {
    if (this.disposed || ev.deltaY === 0) return false;
    const up = ev.deltaY < 0;
    if (this.core.mouse_reporting()) {
      const col = clamp(Math.trunc(ev.col ?? 0), 0, this.cols - 1);
      const row = clamp(Math.trunc(ev.row ?? 0), 0, this.rows - 1);
      const sent = this.core.report_mouse(
        up ? 6 : 7,
        0,
        col,
        row,
        Math.max(0, Math.floor(ev.x ?? 0)),
        Math.max(0, Math.floor(ev.y ?? 0)),
        (ev.shiftKey ? 1 : 0) | (ev.altKey ? 2 : 0) | (ev.ctrlKey ? 4 : 0),
      );
      if (sent) this.flush();
      return true;
    }
    const lines = this.wheelLines(ev);
    if (this.core.is_alt_screen()) {
      if (lines === 0) return false;
      const key = up ? "A" : "B";
      const seq = (this.core.application_cursor_keys() ? "\x1bO" : "\x1b[") + key;
      this.input(seq.repeat(Math.abs(lines)));
      return true;
    }
    if (this.scrollbackLength === 0) return false;
    if (lines !== 0) this.scrollLines(lines);
    return true;
  }

  /** ホイールが動かす行の数（上が負）。Shift つきは 0。xterm.js の `getLinesScrolled` と同じ式。 */
  private wheelLines(ev: WheelInput): number {
    if (ev.shiftKey) return 0;
    let amount = ev.deltaY;
    if (ev.deltaMode === 0) {
      // ピクセル: 行の高さで割り、端数は次に回す
      const h = this.renderer?.cellSize.height || 1;
      this.wheelPartial += amount / h;
      amount = Math.trunc(this.wheelPartial);
      this.wheelPartial -= amount;
    } else if (ev.deltaMode === 2) {
      amount *= this.rows;
    }
    return Math.trunc(amount);
  }

  /** DOM のマウスのイベントを `MouseInput` にする。 */
  private pointerInput(
    canvas: HTMLCanvasElement,
    e: MouseEvent,
    type: MouseInput["type"],
  ): MouseInput & { outside: -1 | 0 | 1 } {
    const rect = canvas.getBoundingClientRect();
    const cell = this.renderer?.cellSize ?? { width: 1, height: 1 };
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    return {
      type,
      button: e.button,
      col: Math.floor(x / cell.width),
      row: Math.floor(y / cell.height),
      x,
      y,
      shiftKey: e.shiftKey,
      altKey: e.altKey,
      ctrlKey: e.ctrlKey,
      clickCount: e.detail,
      outside: y < 0 ? -1 : y >= this.rows * cell.height ? 1 : 0,
    };
  }

  /** 選択中に画面の外へ出たとき、その向きにスクロールし続ける。`move` を渡し続け、`stop` で止める。 */
  private autoScroller(canvas: HTMLCanvasElement): {
    move(e: MouseEvent): void;
    stop(): void;
  } {
    let timer: ReturnType<typeof setInterval> | undefined;
    let last: MouseEvent | undefined;
    const stop = () => {
      if (timer !== undefined) clearInterval(timer);
      timer = undefined;
    };
    return {
      stop,
      move: (e) => {
        last = e;
        const outside = this.pointerInput(canvas, e, "move").outside;
        if (this.mouseOwner !== "select" || outside === 0) return stop();
        timer ??= setInterval(() => {
          const i = last && this.pointerInput(canvas, last, "move");
          if (!i || this.mouseOwner !== "select" || i.outside === 0) return stop();
          this.scrollLines(i.outside);
          this.handleMouse(i);
        }, 50);
      },
    };
  }

  /** 押す・動かす・離すを受ける。戻り値は後始末。 */
  private bindPointer(canvas: HTMLCanvasElement): () => void {
    const doc = canvas.ownerDocument;
    const auto = this.autoScroller(canvas);
    const onMove = (e: MouseEvent) => {
      this.handleMouse(this.pointerInput(canvas, e, "move"));
      auto.move(e);
    };
    const onUp = (e: MouseEvent) => {
      auto.stop();
      doc.removeEventListener("mousemove", onMove);
      doc.removeEventListener("mouseup", onUp);
      this.handleMouse(this.pointerInput(canvas, e, "up"));
    };
    const onDown = (e: MouseEvent) => {
      if (this.handleMouse(this.pointerInput(canvas, e, "down"))) {
        // ブラウザーの既定の選択やドラッグを止め、代わりにフォーカスを当てる
        e.preventDefault();
        this.focus();
        doc.addEventListener("mousemove", onMove);
        doc.addEventListener("mouseup", onUp);
      }
    };
    // ボタンを押していない動きは、canvas の上のものだけ報告する
    const onHover = (e: MouseEvent) => {
      if (!this.mouseOwner) this.handleMouse(this.pointerInput(canvas, e, "move"));
    };
    canvas.addEventListener("mousedown", onDown);
    canvas.addEventListener("mousemove", onHover);
    return () => {
      auto.stop();
      canvas.removeEventListener("mousedown", onDown);
      canvas.removeEventListener("mousemove", onHover);
      doc.removeEventListener("mousemove", onMove);
      doc.removeEventListener("mouseup", onUp);
    };
  }

  /** ホイールを受ける。戻り値は後始末。 */
  private bindWheel(el: HTMLElement, canvas: HTMLCanvasElement): () => void {
    const onWheel = (e: WheelEvent) => {
      const i = this.pointerInput(canvas, e, "move");
      const consumed = this.handleWheel({
        deltaY: e.deltaY,
        deltaMode: e.deltaMode,
        col: i.col,
        row: i.row,
        x: i.x,
        y: i.y,
        shiftKey: e.shiftKey,
        altKey: e.altKey,
        ctrlKey: e.ctrlKey,
      });
      if (consumed) e.preventDefault();
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }

  /**
   * `copy` を受ける。ブラウザーは `copy` を、編集できる要素か選択の始点か `body` に送り、
   * 欄の要素には送らないことがあるので、document で受けて、フォーカスが欄の中にあるときだけ扱う
   * （別の入力欄でのコピーは邪魔しない）。戻り値は後始末。
   */
  private bindCopy(el: HTMLElement): () => void {
    const doc = el.ownerDocument;
    const onCopy = (e: ClipboardEvent) => {
      const active = doc.activeElement;
      if (!active || !el.contains(active)) return;
      if (e.clipboardData && this.copySelection(e.clipboardData)) {
        e.preventDefault();
      }
    };
    doc.addEventListener("copy", onCopy);
    return () => doc.removeEventListener("copy", onCopy);
  }

  /** 後始末をする。書き込み待ちのデータは捨てる（`done` は呼ばない）。 */
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.queue.length = 0;
    for (const c of this.cleanups) c();
    this.cleanups.length = 0;
    this.element = undefined;
    this.textarea = undefined;
    this.composition = undefined;
    this.host = undefined;
    this.renderer = undefined;
    this.scrollbar = undefined;
    this.scrollListeners.clear();
    this.selectionListeners.clear();
    this.cellSizeListeners.clear();
    this.dataListeners.clear();
    this.titleListeners.clear();
    this.bellListeners.clear();
    this.notificationListeners.clear();
    this.notificationCloseListeners.clear();
    this.notificationQueryListeners.clear();
    this.notificationAliveListeners.clear();
    this.dndListeners.clear();
    this.fileTransferListeners.clear();
    this.oscHandlers.clear();
    this.wasm.free();
  }
}

function clamp(n: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, n));
}

/** DOM の `MouseEvent.button` を、WASM のボタンの番号にする。 */
function domButton(b: number): number {
  return b === 3 ? 4 : b === 4 ? 5 : b >= 0 && b <= 2 ? b : 3;
}

function addListener<T>(set: Set<T>, listener: T): Disposable {
  set.add(listener);
  return { dispose: () => void set.delete(listener) };
}

function stripUndefined<T extends object>(o: T): Partial<T> {
  return Object.fromEntries(
    Object.entries(o).filter(([, v]) => v !== undefined),
  ) as Partial<T>;
}

/** キーと変換を受ける、見えない `textarea`。 */
function createInputTextarea(doc: Document): HTMLTextAreaElement {
  const textarea = doc.createElement("textarea");
  textarea.className = "coneko-term-input";
  textarea.setAttribute("aria-label", "Terminal input");
  textarea.setAttribute("autocorrect", "off");
  textarea.setAttribute("autocapitalize", "off");
  textarea.setAttribute("autocomplete", "off");
  textarea.setAttribute("spellcheck", "false");
  textarea.setAttribute("wrap", "off");
  textarea.tabIndex = 0;
  Object.assign(textarea.style, {
    position: "absolute",
    left: "-9999em",
    top: "0",
    width: "0",
    height: "0",
    padding: "0",
    border: "0",
    margin: "0",
    opacity: "0",
    zIndex: "-5",
    whiteSpace: "nowrap",
    overflow: "hidden",
    resize: "none",
  });
  return textarea;
}

/** 変換中の字を見せる要素と、`CompositionHelper` が使う見え方。 */
function createCompositionView(
  doc: Document,
  background: string,
  color: string,
): { element: HTMLElement; view: CompositionViewLike } {
  const element = doc.createElement("div");
  element.className = "coneko-term-composition";
  Object.assign(element.style, {
    display: "none",
    position: "absolute",
    whiteSpace: "nowrap",
    zIndex: "1",
    background,
    color,
  });
  const view: CompositionViewLike = {
    get textContent() {
      return element.textContent;
    },
    set textContent(v) {
      element.textContent = v;
    },
    style: element.style,
    getBoundingClientRect: () => element.getBoundingClientRect(),
    show: () => void (element.style.display = "inline-block"),
    hide: () => void (element.style.display = "none"),
  };
  return { element, view };
}
