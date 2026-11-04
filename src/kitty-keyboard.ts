// kitty keyboard protocol（https://sw.kovidgoyal.net/kitty/keyboard-protocol/ ）のキーのエンコード。
// 形は `CSI unicode-key-code:alternate-key-codes ; modifiers:event-type ; text-as-codepoints u`。
// 矢印・Home・F5 などの「機能キー」は、`u` の代わりに今までの終端（`A`・`~` など）を使う。
//
// 呼ぶ側は `keyboard.ts` の `evaluateKeyboardEvent`。ここは「どのキーか」と「どう書くか」だけを決め、
// 従来の（xterm.js と同じ）バイト列には触らない。

import type { KeyEventLike } from "./keyboard.js";

/** フラグ 1。disambiguate escape codes。 */
export const KITTY_DISAMBIGUATE = 1;
/** フラグ 2。report event types（押す・繰り返し・離す）。 */
export const KITTY_REPORT_EVENT_TYPES = 2;
/** フラグ 4。report alternate keys（shift を押したときのキーと、ベースのレイアウトのキー）。 */
export const KITTY_REPORT_ALTERNATE_KEYS = 4;
/** フラグ 8。report all keys as escape codes。 */
export const KITTY_REPORT_ALL_KEYS = 8;
/** フラグ 16。report associated text。 */
export const KITTY_REPORT_TEXT = 16;

/** キーの動き。1 押す、2 繰り返し、3 離す（仕様の event-type の値）。 */
export type KeyEventType = 1 | 2 | 3;

// 修飾キーのビット（modifiers は 1 + ビットの和）
export const MOD_SHIFT = 1;
export const MOD_ALT = 2;
export const MOD_CTRL = 4;
export const MOD_SUPER = 8;

/** エンコードするキー 1 つ。 */
export interface KittyKey {
  /** `u` ならキーの番号（Unicode か、仕様の私用領域の番号）。ほかの終端なら、パラメーターの数。 */
  num: number;
  /** 終端。`u`・`~`、または `A B C D H F P Q S`。 */
  final: string;
  /**
   * フラグ 8 が無くても、押したときに `CSI` で送るか。送らない（今までの送り方のまま）キーは
   * 繰り返し・離すだけを `CSI` で送る。
   */
  escapedOnPress: boolean;
  /** `:` で続ける別のキー（フラグ 4）。例: `:65`、`::99`、`:65:99`。 */
  alternates?: string;
  /** 添える字のコードポイント（フラグ 16）。`:` で区切る。 */
  text?: string;
}

// ---- 字を打つキー ----

// `KeyboardEvent.code` から、ベースのレイアウト（PC-101 の US 配列）の字。[そのまま, Shift つき]
const BASE_LAYOUT: Record<string, [string, string]> = {
  Minus: ["-", "_"],
  Equal: ["=", "+"],
  BracketLeft: ["[", "{"],
  BracketRight: ["]", "}"],
  Backslash: ["\\", "|"],
  Semicolon: [";", ":"],
  Quote: ["'", '"'],
  Backquote: ["`", "~"],
  Comma: [",", "<"],
  Period: [".", ">"],
  Slash: ["/", "?"],
  Space: [" ", " "],
};
for (let i = 0; i < 26; i++) {
  const c = String.fromCharCode(97 + i);
  BASE_LAYOUT[`Key${c.toUpperCase()}`] = [c, c.toUpperCase()];
}
")!@#$%^&*(".split("").forEach((s, i) => {
  BASE_LAYOUT[`Digit${i}`] = [String(i), s];
});

// `code` が空のとき（古いブラウザーやテスト）の代わりに、`keyCode` から `code` を決める
const CODE_BY_KEYCODE: Record<number, string> = {
  32: "Space",
  186: "Semicolon",
  187: "Equal",
  188: "Comma",
  189: "Minus",
  190: "Period",
  191: "Slash",
  192: "Backquote",
  219: "BracketLeft",
  220: "Backslash",
  221: "BracketRight",
  222: "Quote",
};

function baseLayoutOf(ev: KeyEventLike): [string, string] | undefined {
  let code = ev.code;
  if (!code) {
    const kc = ev.keyCode;
    if (kc >= 65 && kc <= 90) code = `Key${String.fromCharCode(kc)}`;
    else if (kc >= 48 && kc <= 57) code = `Digit${kc - 48}`;
    else code = CODE_BY_KEYCODE[kc] ?? "";
  }
  return BASE_LAYOUT[code];
}

/** 印字できる 1 コードポイントならその字。キーの名前（"Enter" など）や制御文字は `undefined`。 */
function singleChar(key: string): string | undefined {
  if (!key) return undefined;
  const cps = Array.from(key);
  if (cps.length !== 1) return undefined;
  const cp = cps[0].codePointAt(0)!;
  return cp < 0x20 || cp === 0x7f ? undefined : cps[0];
}

function lowerSingle(s: string): string {
  const l = s.toLowerCase();
  return Array.from(l).length === 1 ? l : s;
}

function upperSingle(s: string): string {
  const u = s.toUpperCase();
  return Array.from(u).length === 1 ? u : s;
}

const cp = (s: string) => s.codePointAt(0)!;

interface TextKey {
  /** Shift なしの字（キーの番号になる）。 */
  code: number;
  /** Shift を押しているとき、Shift をかけた字。 */
  shifted?: number;
  /** ベースのレイアウトの字。キーの番号と違うときだけ。 */
  base?: number;
}

/**
 * 字を打つキーの番号と、別のキー。`undefined` なら字を打つキーではない（テンキーも含む。
 * テンキーは `numpadKey`）。
 *
 * 番号は、いまの配列でそのキーが Shift なしで出す字。ブラウザーは Shift をかけた字を `key` に
 * 入れるので、英字は小文字にし、それ以外は US 配列の対応（`code` から決める）で戻す。US 配列と
 * 違う配列の記号は戻せないので、`key` そのままになる。
 */
function textKey(ev: KeyEventLike): TextKey | undefined {
  if (ev.code.startsWith("Numpad")) return undefined;
  const shift = ev.shiftKey;
  const us = baseLayoutOf(ev);
  const k = singleChar(ev.key) ?? us?.[shift ? 1 : 0];
  if (k === undefined) return undefined;
  const lower = lowerSingle(k);
  const unshifted = shift
    ? lower !== k
      ? lower
      : us && us[1] === k
        ? us[0]
        : k
    : lower;
  const out: TextKey = { code: cp(unshifted) };
  if (shift) {
    const shifted = k !== unshifted ? k : upperSingle(unshifted);
    if (shifted !== unshifted) out.shifted = cp(shifted);
  }
  if (us && us[0] !== unshifted) out.base = cp(us[0]);
  return out;
}

// ---- 機能キー ----

interface Functional {
  num: number;
  final: string;
}

// 従来のバイト列があるキー（`keyboard.ts` の switch と同じ keyCode）
const LEGACY_FUNCTIONAL: Record<number, Functional> = {
  37: { num: 1, final: "D" },
  38: { num: 1, final: "A" },
  39: { num: 1, final: "C" },
  40: { num: 1, final: "B" },
  36: { num: 1, final: "H" },
  35: { num: 1, final: "F" },
  45: { num: 2, final: "~" },
  46: { num: 3, final: "~" },
  33: { num: 5, final: "~" },
  34: { num: 6, final: "~" },
  112: { num: 1, final: "P" },
  113: { num: 1, final: "Q" },
  // F3 の `CSI R` はカーソル位置の報告とぶつかるので `CSI 13 ~`
  114: { num: 13, final: "~" },
  115: { num: 1, final: "S" },
  116: { num: 15, final: "~" },
  117: { num: 17, final: "~" },
  118: { num: 18, final: "~" },
  119: { num: 19, final: "~" },
  120: { num: 20, final: "~" },
  121: { num: 21, final: "~" },
  122: { num: 23, final: "~" },
  123: { num: 24, final: "~" },
};

// 従来のバイト列が無いキー（フラグ 8 のときだけ送る）。keyCode から
const EXTRA_BY_KEYCODE: Record<number, number> = {
  20: 57358, // Caps Lock
  145: 57359, // Scroll Lock
  144: 57360, // Num Lock
  44: 57361, // Print Screen
  19: 57362, // Pause
  93: 57363, // Menu
};
for (let n = 13; n <= 24; n++) EXTRA_BY_KEYCODE[124 + n - 13] = 57376 + n - 13; // F13〜F24

// 修飾キーそのもの。`code` から
const MODIFIER_BY_CODE: Record<string, number> = {
  ShiftLeft: 57441,
  ControlLeft: 57442,
  AltLeft: 57443,
  MetaLeft: 57444,
  OSLeft: 57444,
  ShiftRight: 57447,
  ControlRight: 57448,
  AltRight: 57449,
  MetaRight: 57450,
  OSRight: 57450,
  ContextMenu: 57363,
};
// `code` が空のときの代わり（左側として扱う）
const MODIFIER_BY_KEYCODE: Record<number, number> = {
  16: 57441,
  17: 57442,
  18: 57443,
  91: 57444,
  224: 57444,
  92: 57450,
};

// テンキー。`code` から
const NUMPAD_BY_CODE: Record<string, number> = {
  NumpadDecimal: 57409,
  NumpadDivide: 57410,
  NumpadMultiply: 57411,
  NumpadSubtract: 57412,
  NumpadAdd: 57413,
  NumpadEnter: 57414,
  NumpadEqual: 57415,
  NumpadComma: 57416,
};
// NumLock が切れているとき、テンキーは矢印などの keyCode になる
const NUMPAD_NAV_BY_KEYCODE: Record<number, number> = {
  37: 57417,
  39: 57418,
  38: 57419,
  40: 57420,
  33: 57421,
  34: 57422,
  36: 57423,
  35: 57424,
  45: 57425,
  46: 57426,
  12: 57427, // 5（NumLock なし）
};

function numpadKey(ev: KeyEventLike): number | undefined {
  if (!ev.code.startsWith("Numpad")) return undefined;
  if (ev.code === "NumpadEnter") return NUMPAD_BY_CODE.NumpadEnter;
  const nav = NUMPAD_NAV_BY_KEYCODE[ev.keyCode];
  if (nav !== undefined) return nav;
  const digit = /^Numpad(\d)$/.exec(ev.code);
  if (digit) return 57399 + Number(digit[1]);
  return NUMPAD_BY_CODE[ev.code];
}

function pressKey(num: number, final = "u", escapedOnPress = true): KittyKey {
  return { num, final, escapedOnPress };
}

/**
 * このキーイベントを、kitty のどのキーにするか。エンコードしないキーは `undefined`
 * （従来の送り方に任せる）。`flags` はいまの kitty のフラグ。
 *
 * - フラグ 8: すべてのキー（修飾キーだけ・テンキー・Enter・Tab・Backspace・字のキーを含む）。
 * - フラグ 8 が無い: Esc（フラグ 1）、修飾キーつきの Enter・Tab・Backspace（フラグ 1）、
 *   Ctrl・Alt・Super つきの字（フラグ 1）、F3（フラグ 1）。矢印・Home・F5 などは押すときは従来のまま
 *   （`escapedOnPress` が false）で、繰り返し・離すだけがここを通る。
 */
export function kittyKeyOf(
  ev: KeyEventLike,
  modifiers: number,
  flags: number,
  isMac: boolean,
  macOptionIsMeta: boolean,
): KittyKey | undefined {
  const all = (flags & KITTY_REPORT_ALL_KEYS) !== 0;
  const disambiguate = all || (flags & KITTY_DISAMBIGUATE) !== 0;
  const withText = (flags & KITTY_REPORT_TEXT) !== 0;
  const withAlternates = (flags & KITTY_REPORT_ALTERNATE_KEYS) !== 0;

  // 修飾キーそのもの・ロックキーなど（フラグ 8 のときだけ）
  if (all) {
    const mod = MODIFIER_BY_CODE[ev.code] ?? (ev.code ? undefined : MODIFIER_BY_KEYCODE[ev.keyCode]);
    if (mod !== undefined) return pressKey(ev.key === "AltGraph" ? 57453 : mod);
    if (ev.key === "AltGraph") return pressKey(57453);
    const extra = EXTRA_BY_KEYCODE[ev.keyCode];
    if (extra !== undefined) return pressKey(extra);
    const kp = numpadKey(ev);
    if (kp !== undefined) {
      const k = pressKey(kp);
      const ch = singleChar(ev.key);
      if (withText && ch !== undefined && !(modifiers & (MOD_CTRL | MOD_ALT | MOD_SUPER))) {
        k.text = String(cp(ch));
      }
      return k;
    }
  }

  switch (ev.keyCode) {
    case 27:
      return disambiguate ? pressKey(27) : undefined;
    case 8:
      return all || (disambiguate && modifiers) ? pressKey(127) : undefined;
    case 9:
      return all || (disambiguate && modifiers) ? pressKey(9) : undefined;
    case 13:
      return all || (disambiguate && modifiers) ? pressKey(13) : undefined;
  }

  const fn = LEGACY_FUNCTIONAL[ev.keyCode];
  if (fn) {
    // F3 は フラグ 1 以上なら `CSI 13 ~` で送る。ほかは今までのまま
    return pressKey(fn.num, fn.final, all || (disambiguate && ev.keyCode === 114));
  }
  // 字を打つキー
  if (!all && !(disambiguate && modifiers & (MOD_ALT | MOD_CTRL | MOD_SUPER))) return undefined;
  // macOS の Option（macOptionIsMeta が false）は字を打つキー。Windows の AltGr は呼ぶ側が除く
  if (isMac && !macOptionIsMeta && ev.altKey && !ev.ctrlKey && !ev.metaKey) return undefined;
  const tk = textKey(ev);
  if (!tk) return undefined;
  const k = pressKey(tk.code);
  if (withAlternates && (tk.shifted !== undefined || tk.base !== undefined)) {
    k.alternates = `:${tk.shifted ?? ""}${tk.base !== undefined ? `:${tk.base}` : ""}`;
  }
  // 字を添えるのは、修飾キーが字を作るのを邪魔していない（Shift だけ）ときの、押す・繰り返し
  if (withText && !(modifiers & (MOD_ALT | MOD_CTRL | MOD_SUPER))) {
    const ch = singleChar(ev.key);
    if (ch !== undefined) k.text = String(cp(ch));
  }
  return k;
}

/**
 * `CSI ... final` を作る。`modifiers` は `MOD_*` の和（0 なら無し）。
 *
 * - 修飾キーが無く、押す動きで、字も無ければ、その欄は省く（`CSI 97 u`、`CSI A`）。
 * - 字だけがあるときは、修飾キーの欄を空にする（`CSI 97 ; ; 97 u`）。
 * - `A` `B` `C` `D` `H` `F` `P` `Q` `S` は、欄が要るときだけ番号 `1` を付ける（`CSI 1 ; 1:3 A`）。
 */
export function formatKittyKey(
  k: KittyKey,
  modifiers: number,
  eventType: KeyEventType,
): string {
  const text = eventType !== 3 ? k.text : undefined;
  const modField =
    modifiers || eventType !== 1
      ? `${modifiers + 1}${eventType !== 1 ? `:${eventType}` : ""}`
      : "";
  const needsField = modField !== "" || text !== undefined;
  let code: string;
  if (k.final === "u" || k.final === "~") code = `${k.num}${k.alternates ?? ""}`;
  else code = needsField ? String(k.num) : "";
  let params = code;
  if (needsField) params += `;${modField}`;
  if (text !== undefined) params += `;${text}`;
  return `\x1b[${params}${k.final}`;
}
