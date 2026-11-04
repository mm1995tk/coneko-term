// キーを PTY へ送るバイト列にする。xterm.js 6.0.0 の `src/common/input/Keyboard.ts`
// （`evaluateKeyboardEvent`）と同じ値を返す。xterm.js と同じく `keyCode` で決める。

/** `evaluateKeyboardEvent` が読むキーボードイベントの部分。`KeyboardEvent` がそのまま入る。 */
export interface KeyEventLike {
  readonly keyCode: number;
  readonly key: string;
  readonly code: string;
  readonly shiftKey: boolean;
  readonly altKey: boolean;
  readonly ctrlKey: boolean;
  readonly metaKey: boolean;
}

/** `evaluateKeyboardEvent` の結果の種類。 */
export type KeyResultType = "send" | "pageUp" | "pageDown";

/** `evaluateKeyboardEvent` の結果。 */
export interface KeyResult {
  /** `send` なら `key` を送る。`pageUp`・`pageDown` は表示位置を 1 ページ動かす（Shift+PageUp・PageDown）。 */
  type: KeyResultType;
  /** イベントの既定の動きを止めるか。 */
  cancel: boolean;
  /** 送る文字列。送らないなら `undefined`。 */
  key: string | undefined;
}

const ESC = "\x1b";

// 数字と記号のキーの、そのまま・Shift つきの字
const KEYCODE_KEY_MAPPINGS: Record<number, [string, string]> = {
  48: ["0", ")"],
  49: ["1", "!"],
  50: ["2", "@"],
  51: ["3", "#"],
  52: ["4", "$"],
  53: ["5", "%"],
  54: ["6", "^"],
  55: ["7", "&"],
  56: ["8", "*"],
  57: ["9", "("],
  186: [";", ":"],
  187: ["=", "+"],
  188: [",", "<"],
  189: ["-", "_"],
  190: [".", ">"],
  191: ["/", "?"],
  192: ["`", "~"],
  219: ["[", "{"],
  220: ["\\", "|"],
  221: ["]", "}"],
  222: ["'", '"'],
};

// F1〜F4 は `SS3 P`〜`SS3 S`（修飾キーつきは `CSI 1;m P`〜`S`）、F5〜F12 は `CSI n ~`
const FUNCTION_KEYS: Record<number, [number | string, boolean]> = {
  112: ["P", true],
  113: ["Q", true],
  114: ["R", true],
  115: ["S", true],
  116: [15, false],
  117: [17, false],
  118: [18, false],
  119: [19, false],
  120: [20, false],
  121: [21, false],
  122: [23, false],
  123: [24, false],
};

/** kitty keyboard protocol のフラグ 1（disambiguate escape codes）。 */
export const KITTY_DISAMBIGUATE = 1;

/**
 * フラグ 1 のときの `CSI ... u` を作る。このフラグで形が変わるキーでなければ `undefined`
 * （そのときは xterm.js と同じ送り方に任せる）。
 *
 * - Esc: `CSI 27 u`（修飾キーつきは `CSI 27 ; m u`）。
 * - Enter・Tab・Backspace: 修飾キーなしは今のまま。修飾キーつきは `CSI 13|9|127 ; m u`。
 * - F3: `CSI R` は カーソル位置の報告とぶつかるので、`CSI 13 ~`（修飾キーつきは `CSI 13 ; m ~`）。
 * - 字・数字・記号・空白: Ctrl・Alt・Super のどれかが付いているとき、押したキーの字（Shift なしの
 *   小文字）を番号にして `CSI code ; m u`。Shift だけなら字そのもの。
 * - 矢印・Home・End・PageUp・PageDown・Insert・Delete・F1・F2・F4〜F12 は今のまま。
 *
 * `m` は 1 + (Shift 1, Alt 2, Ctrl 4, Super 8)。Super は `metaKey`。
 */
function kittyDisambiguate(
  ev: KeyEventLike,
  modifiers: number,
  isMac: boolean,
  macOptionIsMeta: boolean,
): string | undefined {
  const u = (code: number) =>
    modifiers ? `${ESC}[${code};${modifiers + 1}u` : `${ESC}[${code}u`;
  switch (ev.keyCode) {
    case 27:
      return u(27);
    case 8:
      return modifiers ? u(127) : undefined;
    case 9:
      return modifiers ? u(9) : undefined;
    case 13:
      return modifiers ? u(13) : undefined;
    case 114:
      return modifiers ? `${ESC}[13;${modifiers + 1}~` : `${ESC}[13~`;
  }
  // 字を打つキーで、Ctrl・Alt・Super が付いているもの
  if (!(modifiers & 14)) return undefined;
  // macOS の Option（macOptionIsMeta が false）は字を打つキー。Windows の AltGr は呼ぶ側が除く
  if (isMac && !macOptionIsMeta && ev.altKey && !ev.ctrlKey && !ev.metaKey) {
    return undefined;
  }
  if (ev.keyCode >= 65 && ev.keyCode <= 90) return u(ev.keyCode + 32);
  if (ev.keyCode === 32) return u(32);
  const mapping = KEYCODE_KEY_MAPPINGS[ev.keyCode];
  if (mapping) return u(mapping[0].charCodeAt(0));
  return undefined;
}

/** 矢印・Home・End。修飾キーが無ければ、アプリケーションカーソルキーで `SS3`、そうでなければ `CSI`。 */
function cursorKey(
  letter: string,
  modifiers: number,
  applicationCursorMode: boolean,
): string {
  if (modifiers) return `${ESC}[1;${modifiers + 1}${letter}`;
  return applicationCursorMode ? `${ESC}O${letter}` : `${ESC}[${letter}`;
}

/**
 * キーが押されたとき、PTY へ送る文字列を決める（xterm.js 6.0.0 の `evaluateKeyboardEvent` と同じ）。
 *
 * - `applicationCursorMode`: DECCKM（#5）。矢印・Home・End が `CSI` か `SS3` かを決める。
 * - `kittyFlags`: kitty keyboard protocol のフラグ（`Terminal.keyboardFlags`）。1 が立っていれば、
 *   Esc や、Ctrl・Alt つきの字などを `CSI ... u` で送る。0 なら xterm.js と同じ。
 * - `isMac`: macOS か。Option は `macOptionIsMeta` が false なら字を打つキーとして扱い、ここでは送らない。
 * - Cmd（`metaKey`）つきのキーをアプリに任せるかどうかは呼ぶ側が決める（`KeyHandler` が送らない）。
 */
export function evaluateKeyboardEvent(
  ev: KeyEventLike,
  applicationCursorMode: boolean,
  isMac: boolean,
  macOptionIsMeta: boolean,
  kittyFlags = 0,
): KeyResult {
  const result: KeyResult = { type: "send", cancel: false, key: undefined };
  const modifiers =
    (ev.shiftKey ? 1 : 0) |
    (ev.altKey ? 2 : 0) |
    (ev.ctrlKey ? 4 : 0) |
    (ev.metaKey ? 8 : 0);
  if (kittyFlags & KITTY_DISAMBIGUATE) {
    const key = kittyDisambiguate(ev, modifiers, isMac, macOptionIsMeta);
    if (key !== undefined) {
      result.key = key;
      result.cancel = true;
      return result;
    }
  }
  switch (ev.keyCode) {
    case 0: {
      // iOS の矢印
      const letter = {
        UIKeyInputUpArrow: "A",
        UIKeyInputDownArrow: "B",
        UIKeyInputRightArrow: "C",
        UIKeyInputLeftArrow: "D",
      }[ev.key];
      if (letter) result.key = cursorKey(letter, 0, applicationCursorMode);
      break;
    }
    case 8:
      // Backspace: ^H か ^?
      result.key = ev.ctrlKey ? "\b" : "\x7f";
      if (ev.altKey) result.key = ESC + result.key;
      break;
    case 9:
      // Tab
      if (ev.shiftKey) {
        result.key = `${ESC}[Z`;
        break;
      }
      result.key = "\t";
      result.cancel = true;
      break;
    case 13:
      // Enter
      result.key = ev.altKey ? `${ESC}\r` : "\r";
      result.cancel = true;
      break;
    case 27:
      // Esc
      result.key = ev.altKey ? ESC + ESC : ESC;
      result.cancel = true;
      break;
    case 37: // 左
    case 38: // 上
    case 39: // 右
    case 40: // 下
      if (ev.metaKey) break;
      result.key = cursorKey(
        { 37: "D", 38: "A", 39: "C", 40: "B" }[ev.keyCode]!,
        modifiers,
        applicationCursorMode,
      );
      break;
    case 45:
      // Insert。Ctrl・Shift つきはコピーと貼り付けに使われることがあるので送らない
      if (!ev.shiftKey && !ev.ctrlKey) result.key = `${ESC}[2~`;
      break;
    case 46:
      // Delete
      result.key = modifiers
        ? `${ESC}[3;${modifiers + 1}~`
        : `${ESC}[3~`;
      break;
    case 36:
      result.key = cursorKey("H", modifiers, applicationCursorMode);
      break;
    case 35:
      result.key = cursorKey("F", modifiers, applicationCursorMode);
      break;
    case 33:
      if (ev.shiftKey) result.type = "pageUp";
      else if (ev.ctrlKey) result.key = `${ESC}[5;${modifiers + 1}~`;
      else result.key = `${ESC}[5~`;
      break;
    case 34:
      if (ev.shiftKey) result.type = "pageDown";
      else if (ev.ctrlKey) result.key = `${ESC}[6;${modifiers + 1}~`;
      else result.key = `${ESC}[6~`;
      break;
    default: {
      const fn = FUNCTION_KEYS[ev.keyCode];
      if (fn) {
        const [id, ss3] = fn;
        if (ss3) {
          result.key = modifiers
            ? `${ESC}[1;${modifiers + 1}${id}`
            : `${ESC}O${id}`;
        } else {
          result.key = modifiers
            ? `${ESC}[${id};${modifiers + 1}~`
            : `${ESC}[${id}~`;
        }
        break;
      }
      // a-z、空白、記号
      if (ev.ctrlKey && !ev.shiftKey && !ev.altKey && !ev.metaKey) {
        if (ev.keyCode >= 65 && ev.keyCode <= 90) {
          result.key = String.fromCharCode(ev.keyCode - 64);
        } else if (ev.keyCode === 32) {
          result.key = "\0";
        } else if (ev.keyCode >= 51 && ev.keyCode <= 55) {
          // ESC・FS・GS・RS・US
          result.key = String.fromCharCode(ev.keyCode - 51 + 27);
        } else if (ev.keyCode === 56) {
          result.key = "\x7f";
        } else if (ev.keyCode === 219) {
          result.key = ESC;
        } else if (ev.keyCode === 220) {
          result.key = "\x1c";
        } else if (ev.keyCode === 221) {
          result.key = "\x1d";
        }
      } else if ((!isMac || macOptionIsMeta) && ev.altKey && !ev.metaKey) {
        // macOS で macOptionIsMeta が false のときの Option は、字を打つ 3 段目のシフト。
        // ここでは ESC の前置きにする
        const mapping = KEYCODE_KEY_MAPPINGS[ev.keyCode];
        const key = mapping?.[ev.shiftKey ? 1 : 0];
        if (key) {
          result.key = ESC + key;
        } else if (ev.keyCode >= 65 && ev.keyCode <= 90) {
          const code = ev.ctrlKey ? ev.keyCode - 64 : ev.keyCode + 32;
          let s = String.fromCharCode(code);
          if (ev.shiftKey) s = s.toUpperCase();
          result.key = ESC + s;
        } else if (ev.keyCode === 32) {
          result.key = ESC + (ev.ctrlKey ? "\0" : " ");
        } else if (ev.key === "Dead" && ev.code.startsWith("Key")) {
          // US 配列の Alt+N・E・U などは、変換の途中の字（dead key）になる
          // （https://github.com/xtermjs/xterm.js/issues/3725）
          let s = ev.code.slice(3, 4);
          if (!ev.shiftKey) s = s.toLowerCase();
          result.key = ESC + s;
          result.cancel = true;
        }
      } else if (isMac && !ev.altKey && !ev.ctrlKey && !ev.shiftKey && ev.metaKey) {
        // Cmd+A などは送らない（xterm.js はここで「すべて選択」にする。選択は #10）
      } else if (
        ev.key &&
        !ev.ctrlKey &&
        !ev.altKey &&
        !ev.metaKey &&
        ev.keyCode >= 48 &&
        ev.key.length === 1
      ) {
        // 1 字になるキーだけ（NumLock や音量などは含めない）
        result.key = ev.key;
      } else if (ev.key && ev.ctrlKey) {
        if (ev.key === "_") result.key = "\x1f"; // ^_
        if (ev.key === "@") result.key = "\0"; // Ctrl+Shift+2
      }
      break;
    }
  }
  return result;
}

/** 貼り付ける文字の改行を `\r` にそろえる（xterm.js の `prepareTextForTerminal`）。 */
export function prepareTextForTerminal(text: string): string {
  return text.replace(/\r?\n/g, "\r");
}

/** bracketed paste（2004）が有効なら、`ESC [200~` と `ESC [201~` で囲む（xterm.js の `bracketTextForPaste`）。 */
export function bracketTextForPaste(
  text: string,
  bracketedPasteMode: boolean,
): string {
  return bracketedPasteMode ? `\x1b[200~${text}\x1b[201~` : text;
}
