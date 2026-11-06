//! kitty keyboard protocol のモードのフラグ（<https://sw.kovidgoyal.net/kitty/keyboard-protocol/>）。
//!
//! フラグは積み重ね（スタック）で持つ。いまのフラグは一番上の値で、空なら 0。主画面と
//! 代替画面で別々のスタックを持つ（仕様どおり）。キーのエンコードは呼ぶ側（TypeScript）が
//! このフラグを読んで決める。

/// フラグ 1。disambiguate escape codes。
pub const DISAMBIGUATE: u32 = 1;

/// フラグ 2。report event types。
pub const REPORT_EVENT_TYPES: u32 = 2;
/// フラグ 4。report alternate keys。
pub const REPORT_ALTERNATE_KEYS: u32 = 4;
/// フラグ 8。report all keys as escape codes。
pub const REPORT_ALL_KEYS: u32 = 8;
/// フラグ 16。report associated text。
pub const REPORT_TEXT: u32 = 16;

/// 対応しているフラグ（1・2・4・8・16）。ほかのビットは捨てる（アプリに、効いていないフラグを
/// 効いていると思わせないため）。
const SUPPORTED: u32 =
    DISAMBIGUATE | REPORT_EVENT_TYPES | REPORT_ALTERNATE_KEYS | REPORT_ALL_KEYS | REPORT_TEXT;

/// スタックの深さの上限。超えたら古いものを捨てる。仕様は「適切な上限を置け」とだけ言い、
/// 値は決めていない（kitty 本体は 8）。ここでは 100 にした。
pub const MAX_DEPTH: usize = 100;

/// フラグのスタック 1 つ分。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyboardFlagStack {
    stack: Vec<u32>,
}

impl KeyboardFlagStack {
    /// 積んでいるフラグ。古いものが先。
    pub(crate) fn entries(&self) -> &[u32] {
        &self.stack
    }

    /// いまのフラグ。
    pub fn flags(&self) -> u32 {
        self.stack.last().copied().unwrap_or(0)
    }

    /// `CSI > flags u`。積む。
    pub fn push(&mut self, flags: u32) {
        if self.stack.len() >= MAX_DEPTH {
            self.stack.remove(0);
        }
        self.stack.push(flags & SUPPORTED);
    }

    /// `CSI < n u`。`n` 個降ろす。積んでいる数より多くても、空になるだけ。
    pub fn pop(&mut self, n: usize) {
        let keep = self.stack.len().saturating_sub(n);
        self.stack.truncate(keep);
    }

    /// `CSI = flags ; mode u`。一番上を置き換える（1）・足す（2）・引く（3）。
    /// ほかの `mode` は捨てる。空なら、0 を積んだものとして変える。
    pub fn modify(&mut self, flags: u32, mode: u32) {
        let flags = flags & SUPPORTED;
        let cur = self.flags();
        let new = match mode {
            1 => flags,
            2 => cur | flags,
            3 => cur & !flags,
            _ => return,
        };
        match self.stack.last_mut() {
            Some(top) => *top = new,
            None => self.stack.push(new),
        }
    }
}
