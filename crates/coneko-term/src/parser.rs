//! VT のパーサー。PTY から来るバイト列を、文字と制御シーケンスに分ける。
//!
//! Paul Williams の DEC ANSI パーサーの状態機械
//! (<https://vt100.net/emu/dec_ansi_parser>) に沿って書いてある。
//! 既存の crate は使わない。APC (`ESC _ ... ESC \`) を確実に扱うため。
//!
//! 分けたシーケンスの意味は扱わない。[`Perform`] の各メソッドに渡すだけ。
//!
//! # Williams の状態機械との違い
//!
//! - 入力は UTF-8 のバイト列。Ground 状態で復号して [`Perform::print`] に渡す。
//!   不正なバイト列は U+FFFD にする。書き込みの途中で切れた字は次の書き込みに続ける。
//! - 8 ビットの C1 制御 (0x80〜0x9F) は解釈しない。0x9C で文字列を終わらせない
//!   (UTF-8 の継続バイトに 0x9C が入るため)。
//! - OSC・DCS・APC・SOS・PM の中の 0x80〜0xFF は文字列の一部として扱う。
//!   文字列を終えるのは `ESC \`、BEL (OSC のとき)、CAN、SUB だけ。
//!   ESC のあとが `\` でなくても、ESC で文字列は終わる (そのあとは新しい ESC シーケンス)。
//!   CAN・SUB で終わった文字列は、渡さずに捨てる。
//! - CSI のパラメーターは `:` で区切るサブパラメーターを持つ。
//! - CSI・ESC の途中に 0x80〜0xFF が来たら、そのシーケンスを捨てて Ground で読み直す。
//! - SOS と PM は読み捨てる。

/// CSI・DCS のパラメーターの数の上限。超えた分は捨てる。
pub const MAX_PARAMS: usize = 32;

/// 中間バイトの数の上限。超えたシーケンスは捨てる。
pub const MAX_INTERMEDIATES: usize = 2;

/// OSC・DCS・APC の中身の長さの既定の上限 (バイト)。超えた分は捨てる。
pub const DEFAULT_MAX_STRING_LEN: usize = 2 * 1024 * 1024;

/// CSI・DCS のパラメーター。
///
/// `;` で区切られたものが 1 つのパラメーター。その中の `:` で区切られたものが
/// サブパラメーター。`CSI 38:2::255:0:0 m` は、パラメーターが 1 つで、
/// 中身は `[38, 2, 0, 255, 0, 0]`。
/// 空のパラメーターは 0 として入る。数が大きすぎるときは `u32::MAX` で止める。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Params {
    values: Vec<u32>,
    /// `values[i]` が、前の値と `:` でつながっているか。
    sub: Vec<bool>,
}

impl Params {
    /// パラメーターの数 (`;` で区切られたものの数)。
    pub fn len(&self) -> usize {
        self.sub.iter().filter(|s| !**s).count()
    }

    /// パラメーターが 1 つも無いか。
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// パラメーターを順に返す。各要素は、先頭が値で、残りがサブパラメーター。
    pub fn iter(&self) -> ParamsIter<'_> {
        ParamsIter {
            values: &self.values,
            sub: &self.sub,
            pos: 0,
        }
    }

    /// `index` 番目のパラメーター (サブパラメーター込み)。
    pub fn get(&self, index: usize) -> Option<&[u32]> {
        self.iter().nth(index)
    }

    fn clear(&mut self) {
        self.values.clear();
        self.sub.clear();
    }

    fn push(&mut self, value: u32, is_sub: bool) {
        if self.values.len() < MAX_PARAMS {
            self.values.push(value);
            self.sub.push(is_sub);
        }
    }
}

impl<'a> IntoIterator for &'a Params {
    type Item = &'a [u32];
    type IntoIter = ParamsIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// [`Params::iter`] が返すイテレーター。各要素は、先頭が値で、残りがサブパラメーター。
pub struct ParamsIter<'a> {
    values: &'a [u32],
    sub: &'a [bool],
    pos: usize,
}

impl<'a> Iterator for ParamsIter<'a> {
    type Item = &'a [u32];

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.values.len() {
            return None;
        }
        let start = self.pos;
        self.pos += 1;
        while self.pos < self.values.len() && self.sub[self.pos] {
            self.pos += 1;
        }
        Some(&self.values[start..self.pos])
    }
}

/// パーサーが分けた結果を受け取る。使わないメソッドは実装しなくてよい。
pub trait Perform {
    /// 表示する字。
    fn print(&mut self, _c: char) {}

    /// C0 の制御 (0x00〜0x1F。ESC は除く)。
    fn execute(&mut self, _byte: u8) {}

    /// ESC シーケンス。`ESC [ intermediates ] byte`。
    fn esc_dispatch(&mut self, _intermediates: &[u8], _byte: u8) {}

    /// CSI。`marker` は `?` `>` `<` `=` のうち先頭に付いたもの。
    fn csi_dispatch(
        &mut self,
        _params: &Params,
        _intermediates: &[u8],
        _marker: Option<u8>,
        _action: u8,
    ) {
    }

    /// OSC。`data` は ESC ] の後から終端の前まで。`bell_terminated` は BEL で終わったとき真。
    fn osc_dispatch(&mut self, _data: &[u8], _bell_terminated: bool) {}

    /// DCS。`data` は最終バイトの後から ST の前まで。
    fn dcs_dispatch(
        &mut self,
        _params: &Params,
        _intermediates: &[u8],
        _marker: Option<u8>,
        _action: u8,
        _data: &[u8],
    ) {
    }

    /// APC。`data` は ESC _ の後から ST の前まで (kitty graphics なら `G...`)。
    fn apc_dispatch(&mut self, _data: &[u8]) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    CsiIgnore,
    DcsEntry,
    DcsParam,
    DcsIntermediate,
    DcsPassthrough,
    DcsIgnore,
    OscString,
    ApcString,
    SosPmString,
}

/// 書き込みの途中で切れた UTF-8 の字を持っておく。
#[derive(Debug, Default)]
struct Utf8 {
    buf: [u8; 4],
    len: usize,
    /// あと何バイト要るか。0 なら字の途中ではない。
    need: usize,
    /// 次のバイトが取れる範囲 (過長符号・サロゲート・範囲外を弾くため)。
    lo: u8,
    hi: u8,
}

/// VT のパーサー。
#[derive(Debug)]
pub struct Parser {
    state: State,
    utf8: Utf8,
    params: Params,
    cur: u32,
    /// パラメーターが 1 文字でも始まったか (`CSI m` と `CSI 0 m` を区別しない代わりに、
    /// `CSI ; m` は 2 つと数えるため)。
    param_started: bool,
    /// 次に入れる値が `:` でつながるか。
    pending_sub: bool,
    intermediates: Vec<u8>,
    marker: Option<u8>,
    /// 中間バイトが多すぎるなど、捨てるシーケンスか。
    ignore: bool,
    dcs_action: u8,
    data: Vec<u8>,
    max_string_len: usize,
    /// 直前に ESC で文字列を終えた。次の `\` (ST) は読み捨てる。
    st_pending: bool,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    /// 既定の上限 ([`DEFAULT_MAX_STRING_LEN`]) で作る。
    pub fn new() -> Self {
        Self::with_max_string_len(DEFAULT_MAX_STRING_LEN)
    }

    /// OSC・DCS・APC の中身の長さの上限を決めて作る。
    pub fn with_max_string_len(max_string_len: usize) -> Self {
        Parser {
            state: State::Ground,
            utf8: Utf8::default(),
            params: Params::default(),
            cur: 0,
            param_started: false,
            pending_sub: false,
            intermediates: Vec::new(),
            marker: None,
            ignore: false,
            dcs_action: 0,
            data: Vec::new(),
            max_string_len,
            st_pending: false,
        }
    }

    /// バイト列を読み、結果を `performer` に渡す。
    /// 途中で切れた字やシーケンスは、次の呼び出しに続く。
    pub fn advance<P: Perform>(&mut self, performer: &mut P, bytes: &[u8]) {
        for &b in bytes {
            self.step(performer, b);
        }
    }

    fn step<P: Perform>(&mut self, p: &mut P, b: u8) {
        match self.state {
            State::Ground => self.ground(p, b),
            State::OscString
            | State::ApcString
            | State::DcsPassthrough
            | State::DcsIgnore
            | State::SosPmString => self.string_byte(p, b),
            _ => self.sequence_byte(p, b),
        }
    }

    fn ground<P: Perform>(&mut self, p: &mut P, b: u8) {
        if self.utf8.need > 0 {
            if (self.utf8.lo..=self.utf8.hi).contains(&b) {
                self.utf8.buf[self.utf8.len] = b;
                self.utf8.len += 1;
                self.utf8.need -= 1;
                self.utf8.lo = 0x80;
                self.utf8.hi = 0xBF;
                if self.utf8.need == 0 {
                    let c = std::str::from_utf8(&self.utf8.buf[..self.utf8.len])
                        .ok()
                        .and_then(|s| s.chars().next())
                        .unwrap_or('\u{FFFD}');
                    p.print(c);
                }
                return;
            }
            // 途中で壊れた。壊れた分を 1 字の U+FFFD にして、このバイトを読み直す。
            self.utf8.need = 0;
            p.print('\u{FFFD}');
        }
        match b {
            0x1B => self.enter_escape(false),
            0x00..=0x1A | 0x1C..=0x1F => p.execute(b),
            0x20..=0x7E => p.print(b as char),
            0x7F => {}
            0xC2..=0xF4 => {
                let (need, lo, hi) = match b {
                    0xC2..=0xDF => (1, 0x80, 0xBF),
                    0xE0 => (2, 0xA0, 0xBF),
                    0xED => (2, 0x80, 0x9F),
                    0xE1..=0xEF => (2, 0x80, 0xBF),
                    0xF0 => (3, 0x90, 0xBF),
                    0xF4 => (3, 0x80, 0x8F),
                    _ => (3, 0x80, 0xBF),
                };
                self.utf8.buf[0] = b;
                self.utf8.len = 1;
                self.utf8.need = need;
                self.utf8.lo = lo;
                self.utf8.hi = hi;
            }
            // 継続バイトだけ・0xC0・0xC1・0xF5 以上。8 ビットの C1 もここ。
            _ => p.print('\u{FFFD}'),
        }
    }

    fn enter_escape(&mut self, after_string: bool) {
        self.state = State::Escape;
        self.clear();
        self.st_pending = after_string;
    }

    fn clear(&mut self) {
        self.params.clear();
        self.cur = 0;
        self.param_started = false;
        self.pending_sub = false;
        self.intermediates.clear();
        self.marker = None;
        self.ignore = false;
    }

    fn collect(&mut self, b: u8) {
        if self.intermediates.len() < MAX_INTERMEDIATES {
            self.intermediates.push(b);
        } else {
            self.ignore = true;
        }
    }

    fn param_digit(&mut self, b: u8) {
        self.param_started = true;
        self.cur = self
            .cur
            .saturating_mul(10)
            .saturating_add(u32::from(b - b'0'));
    }

    /// `;` か `:` が来た。
    fn param_sep(&mut self, b: u8) {
        self.param_started = true;
        self.params.push(self.cur, self.pending_sub);
        self.cur = 0;
        self.pending_sub = b == b':';
    }

    fn param_finish(&mut self) {
        if self.param_started {
            self.params.push(self.cur, self.pending_sub);
        }
        self.cur = 0;
        self.param_started = false;
        self.pending_sub = false;
    }

    /// ESC・CSI・DCS の、最終バイトより前の状態。
    fn sequence_byte<P: Perform>(&mut self, p: &mut P, b: u8) {
        let st_pending = self.st_pending;
        let in_dcs = matches!(
            self.state,
            State::DcsEntry | State::DcsParam | State::DcsIntermediate
        );
        match b {
            0x18 | 0x1A => {
                self.state = State::Ground;
                p.execute(b);
                return;
            }
            0x1B => {
                self.enter_escape(false);
                return;
            }
            0x00..=0x1F => {
                if !in_dcs {
                    p.execute(b);
                }
                return;
            }
            0x7F => return,
            0x80..=0xFF => {
                self.state = State::Ground;
                self.ground(p, b);
                return;
            }
            _ => self.st_pending = false,
        }
        match self.state {
            State::Escape => match b {
                b'\\' if st_pending => self.state = State::Ground,
                0x20..=0x2F => {
                    self.collect(b);
                    self.state = State::EscapeIntermediate;
                }
                b'[' => {
                    self.state = State::CsiEntry;
                }
                b']' => self.enter_string(State::OscString),
                b'P' => {
                    self.state = State::DcsEntry;
                }
                b'X' | b'^' => self.enter_string(State::SosPmString),
                b'_' => self.enter_string(State::ApcString),
                _ => self.esc_final(p, b),
            },
            State::EscapeIntermediate => match b {
                0x20..=0x2F => self.collect(b),
                _ => self.esc_final(p, b),
            },
            State::CsiEntry | State::CsiParam => match b {
                b'0'..=b'9' => {
                    self.param_digit(b);
                    self.state = State::CsiParam;
                }
                b':' | b';' => {
                    self.param_sep(b);
                    self.state = State::CsiParam;
                }
                b'<'..=b'?' => {
                    if self.state == State::CsiEntry {
                        self.marker = Some(b);
                        self.state = State::CsiParam;
                    } else {
                        self.state = State::CsiIgnore;
                    }
                }
                0x20..=0x2F => {
                    self.collect(b);
                    self.state = State::CsiIntermediate;
                }
                _ => self.csi_final(p, b),
            },
            State::CsiIntermediate => match b {
                0x20..=0x2F => self.collect(b),
                0x30..=0x3F => self.state = State::CsiIgnore,
                _ => self.csi_final(p, b),
            },
            State::CsiIgnore => {
                if (0x40..=0x7E).contains(&b) {
                    self.state = State::Ground;
                }
            }
            State::DcsEntry | State::DcsParam => match b {
                b'0'..=b'9' => {
                    self.param_digit(b);
                    self.state = State::DcsParam;
                }
                b':' | b';' => {
                    self.param_sep(b);
                    self.state = State::DcsParam;
                }
                b'<'..=b'?' => {
                    if self.state == State::DcsEntry {
                        self.marker = Some(b);
                        self.state = State::DcsParam;
                    } else {
                        self.state = State::DcsIgnore;
                    }
                }
                0x20..=0x2F => {
                    self.collect(b);
                    self.state = State::DcsIntermediate;
                }
                _ => self.dcs_final(b),
            },
            State::DcsIntermediate => match b {
                0x20..=0x2F => self.collect(b),
                0x30..=0x3F => self.state = State::DcsIgnore,
                _ => self.dcs_final(b),
            },
            // 文字列の状態は string_byte が扱う。
            _ => {}
        }
    }

    fn esc_final<P: Perform>(&mut self, p: &mut P, b: u8) {
        self.state = State::Ground;
        if !self.ignore {
            p.esc_dispatch(&self.intermediates, b);
        }
    }

    fn csi_final<P: Perform>(&mut self, p: &mut P, b: u8) {
        self.state = State::Ground;
        self.param_finish();
        if !self.ignore {
            p.csi_dispatch(&self.params, &self.intermediates, self.marker, b);
        }
    }

    fn dcs_final(&mut self, b: u8) {
        self.param_finish();
        self.dcs_action = b;
        self.data.clear();
        self.state = if self.ignore {
            State::DcsIgnore
        } else {
            State::DcsPassthrough
        };
    }

    fn enter_string(&mut self, state: State) {
        self.state = state;
        self.data.clear();
    }

    /// OSC・DCS・APC・SOS・PM の中。0x80〜0xFF も含めてバイトのまま扱う。
    fn string_byte<P: Perform>(&mut self, p: &mut P, b: u8) {
        match b {
            0x1B => {
                self.finish_string(p, false);
                self.enter_escape(true);
            }
            0x18 | 0x1A => {
                self.state = State::Ground;
                p.execute(b);
            }
            0x07 if self.state == State::OscString => self.finish_string(p, true),
            _ => {
                let keep = match self.state {
                    // OSC の中の C0 と DEL は読み捨てる。
                    State::OscString => !(b < 0x20 || b == 0x7F),
                    State::ApcString | State::DcsPassthrough => true,
                    _ => false,
                };
                if keep && self.data.len() < self.max_string_len {
                    self.data.push(b);
                }
            }
        }
    }

    fn finish_string<P: Perform>(&mut self, p: &mut P, bell: bool) {
        match self.state {
            State::OscString => p.osc_dispatch(&self.data, bell),
            State::ApcString => p.apc_dispatch(&self.data),
            State::DcsPassthrough => p.dcs_dispatch(
                &self.params,
                &self.intermediates,
                self.marker,
                self.dcs_action,
                &self.data,
            ),
            _ => {}
        }
        self.state = State::Ground;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Ev {
        Print(char),
        Exec(u8),
        Esc(Vec<u8>, u8),
        Csi {
            params: Vec<Vec<u32>>,
            inter: Vec<u8>,
            marker: Option<u8>,
            action: u8,
        },
        Osc(Vec<u8>, bool),
        Dcs {
            params: Vec<Vec<u32>>,
            inter: Vec<u8>,
            marker: Option<u8>,
            action: u8,
            data: Vec<u8>,
        },
        Apc(Vec<u8>),
    }

    #[derive(Default)]
    struct Rec(Vec<Ev>);

    fn groups(p: &Params) -> Vec<Vec<u32>> {
        p.iter().map(|g| g.to_vec()).collect()
    }

    impl Perform for Rec {
        fn print(&mut self, c: char) {
            self.0.push(Ev::Print(c));
        }
        fn execute(&mut self, b: u8) {
            self.0.push(Ev::Exec(b));
        }
        fn esc_dispatch(&mut self, i: &[u8], b: u8) {
            self.0.push(Ev::Esc(i.to_vec(), b));
        }
        fn csi_dispatch(&mut self, p: &Params, i: &[u8], m: Option<u8>, a: u8) {
            self.0.push(Ev::Csi {
                params: groups(p),
                inter: i.to_vec(),
                marker: m,
                action: a,
            });
        }
        fn osc_dispatch(&mut self, d: &[u8], bell: bool) {
            self.0.push(Ev::Osc(d.to_vec(), bell));
        }
        fn dcs_dispatch(&mut self, p: &Params, i: &[u8], m: Option<u8>, a: u8, d: &[u8]) {
            self.0.push(Ev::Dcs {
                params: groups(p),
                inter: i.to_vec(),
                marker: m,
                action: a,
                data: d.to_vec(),
            });
        }
        fn apc_dispatch(&mut self, d: &[u8]) {
            self.0.push(Ev::Apc(d.to_vec()));
        }
    }

    fn parse(input: &[u8]) -> Vec<Ev> {
        let mut parser = Parser::new();
        let mut rec = Rec::default();
        parser.advance(&mut rec, input);
        rec.0
    }

    /// 全部を 1 回で渡す。1 バイトずつ渡す。どの位置で 2 つに割っても同じ結果になる。
    fn parse_all_splits(input: &[u8]) -> Vec<Ev> {
        let whole = parse(input);
        let mut parser = Parser::new();
        let mut rec = Rec::default();
        for b in input {
            parser.advance(&mut rec, &[*b]);
        }
        assert_eq!(rec.0, whole, "1 バイトずつ: {input:?}");
        for at in 0..=input.len() {
            let mut parser = Parser::new();
            let mut rec = Rec::default();
            parser.advance(&mut rec, &input[..at]);
            parser.advance(&mut rec, &input[at..]);
            assert_eq!(rec.0, whole, "{at} で分割: {input:?}");
        }
        whole
    }

    fn csi(params: &[&[u32]], inter: &[u8], marker: Option<u8>, action: u8) -> Ev {
        Ev::Csi {
            params: params.iter().map(|g| g.to_vec()).collect(),
            inter: inter.to_vec(),
            marker,
            action,
        }
    }

    // ---- Ground ----

    #[test]
    fn ground_prints_ascii_and_executes_c0() {
        assert_eq!(
            parse(b"a\r\n\x07\x08\x7fb"),
            vec![
                Ev::Print('a'),
                Ev::Exec(b'\r'),
                Ev::Exec(b'\n'),
                Ev::Exec(7),
                Ev::Exec(8),
                Ev::Print('b'),
            ]
        );
    }

    #[test]
    fn ground_decodes_utf8() {
        assert_eq!(
            parse("aé日😀".as_bytes()),
            vec![
                Ev::Print('a'),
                Ev::Print('é'),
                Ev::Print('日'),
                Ev::Print('😀')
            ]
        );
    }

    #[test]
    fn invalid_utf8_becomes_replacement_char() {
        let r = '\u{FFFD}';
        // 継続バイトだけ、0xC0、0xF5、8 ビットの C1 (0x9C)
        assert_eq!(parse(&[0x80]), vec![Ev::Print(r)]);
        assert_eq!(parse(&[0xC0, 0xAF]), vec![Ev::Print(r), Ev::Print(r)]);
        assert_eq!(parse(&[0xF5]), vec![Ev::Print(r)]);
        assert_eq!(parse(&[0x9C]), vec![Ev::Print(r)]);
        // 過長符号 E0 80 80、サロゲート ED A0 80、範囲外 F4 90 80 80
        assert_eq!(
            parse(&[0xE0, 0x80, 0x80]),
            vec![Ev::Print(r), Ev::Print(r), Ev::Print(r)]
        );
        assert_eq!(
            parse(&[0xED, 0xA0, 0x80]),
            vec![Ev::Print(r), Ev::Print(r), Ev::Print(r)]
        );
        assert_eq!(
            parse(&[0xF4, 0x90, 0x80, 0x80]),
            vec![Ev::Print(r), Ev::Print(r), Ev::Print(r), Ev::Print(r)]
        );
        // 途中で途切れて ASCII が来る
        assert_eq!(
            parse(&[0xE6, 0x97, b'a']),
            vec![Ev::Print(r), Ev::Print('a')]
        );
        // 途中で ESC が来る
        assert_eq!(
            parse(&[0xE6, 0x1B, b'7']),
            vec![Ev::Print(r), Ev::Esc(vec![], b'7')]
        );
    }

    #[test]
    fn c1_bytes_are_not_interpreted() {
        // 0x9B は 8 ビットの CSI だが、解釈しない。
        assert_eq!(
            parse(&[0x9B, b'1', b'm']),
            vec![Ev::Print('\u{FFFD}'), Ev::Print('1'), Ev::Print('m')]
        );
    }

    // ---- ESC ----

    #[test]
    fn escape_dispatch() {
        assert_eq!(parse(b"\x1b7"), vec![Ev::Esc(vec![], b'7')]);
        assert_eq!(parse(b"\x1bM"), vec![Ev::Esc(vec![], b'M')]);
        assert_eq!(parse(b"\x1b(B"), vec![Ev::Esc(vec![b'('], b'B')]);
        assert_eq!(parse(b"\x1b#8"), vec![Ev::Esc(vec![b'#'], b'8')]);
        assert_eq!(parse(b"\x1b $F"), vec![Ev::Esc(vec![b' ', b'$'], b'F')]);
    }

    #[test]
    fn escape_with_too_many_intermediates_is_dropped() {
        assert_eq!(parse(b"\x1b !\"Fa"), vec![Ev::Print('a')]);
    }

    #[test]
    fn c0_inside_escape_is_executed_and_sequence_continues() {
        assert_eq!(
            parse(b"\x1b\n7"),
            vec![Ev::Exec(b'\n'), Ev::Esc(vec![], b'7')]
        );
    }

    #[test]
    fn esc_restarts_escape_and_can_sub_cancel() {
        assert_eq!(parse(b"\x1b\x1b7"), vec![Ev::Esc(vec![], b'7')]);
        assert_eq!(parse(b"\x1b[1\x18a"), vec![Ev::Exec(0x18), Ev::Print('a')]);
        assert_eq!(parse(b"\x1b(\x1aa"), vec![Ev::Exec(0x1A), Ev::Print('a')]);
    }

    #[test]
    fn non_ascii_aborts_escape_sequence_and_is_read_in_ground() {
        assert_eq!(parse("\x1b[1あ".as_bytes()), vec![Ev::Print('あ')]);
        assert_eq!(parse("\x1bあ".as_bytes()), vec![Ev::Print('あ')]);
    }

    // ---- CSI ----

    #[test]
    fn csi_basic() {
        assert_eq!(parse(b"\x1b[m"), vec![csi(&[], &[], None, b'm')]);
        assert_eq!(parse(b"\x1b[0m"), vec![csi(&[&[0]], &[], None, b'm')]);
        assert_eq!(
            parse(b"\x1b[1;31m"),
            vec![csi(&[&[1], &[31]], &[], None, b'm')]
        );
        assert_eq!(
            parse(b"\x1b[;5H"),
            vec![csi(&[&[0], &[5]], &[], None, b'H')]
        );
        assert_eq!(
            parse(b"\x1b[5;H"),
            vec![csi(&[&[5], &[0]], &[], None, b'H')]
        );
    }

    #[test]
    fn csi_markers() {
        for m in *b"?><=" {
            assert_eq!(
                parse(&[0x1B, b'[', m, b'1', b'h']),
                vec![csi(&[&[1]], &[], Some(m), b'h')]
            );
        }
        // 先頭以外の印は、シーケンスごと捨てる。
        assert_eq!(parse(b"\x1b[1?ha"), vec![Ev::Print('a')]);
    }

    #[test]
    fn csi_intermediates() {
        assert_eq!(parse(b"\x1b[2 q"), vec![csi(&[&[2]], b" ", None, b'q')]);
        assert_eq!(
            parse(b"\x1b[?1$p"),
            vec![csi(&[&[1]], b"$", Some(b'?'), b'p')]
        );
        assert_eq!(parse(b"\x1b[ q"), vec![csi(&[], b" ", None, b'q')]);
        // 中間バイトのあとの数字は不正。捨てる。
        assert_eq!(parse(b"\x1b[ 1qa"), vec![Ev::Print('a')]);
        // 中間バイトが多すぎる。
        assert_eq!(parse(b"\x1b[ !\"qa"), vec![Ev::Print('a')]);
    }

    #[test]
    fn csi_ignore_until_final() {
        assert_eq!(parse(b"\x1b[1?2;3mA"), vec![Ev::Print('A')]);
    }

    #[test]
    fn csi_c0_is_executed_inside() {
        assert_eq!(
            parse(b"\x1b[1\n;2H"),
            vec![Ev::Exec(b'\n'), csi(&[&[1], &[2]], &[], None, b'H')]
        );
    }

    #[test]
    fn csi_del_is_ignored_inside() {
        assert_eq!(parse(b"\x1b[1\x7f2m"), vec![csi(&[&[12]], &[], None, b'm')]);
    }

    #[test]
    fn csi_subparams() {
        assert_eq!(parse(b"\x1b[4:3m"), vec![csi(&[&[4, 3]], &[], None, b'm')]);
        assert_eq!(
            parse(b"\x1b[38:2::255:0:0m"),
            vec![csi(&[&[38, 2, 0, 255, 0, 0]], &[], None, b'm')]
        );
        assert_eq!(
            parse(b"\x1b[1;38:5:196;4:3m"),
            vec![csi(&[&[1], &[38, 5, 196], &[4, 3]], &[], None, b'm')]
        );
        // ; と : の区別が params に残る。
        assert_eq!(
            parse(b"\x1b[38;2;255m"),
            vec![csi(&[&[38], &[2], &[255]], &[], None, b'm')]
        );
        // 先頭の : は空の値 0。
        assert_eq!(parse(b"\x1b[:1m"), vec![csi(&[&[0, 1]], &[], None, b'm')]);
    }

    #[test]
    fn params_len_get_and_iter() {
        struct P(Option<(usize, Vec<u32>, bool)>);
        impl Perform for P {
            fn csi_dispatch(&mut self, p: &Params, _: &[u8], _: Option<u8>, _: u8) {
                self.0 = Some((p.len(), p.get(1).unwrap().to_vec(), p.get(2).is_none()));
            }
        }
        let mut parser = Parser::new();
        let mut p = P(None);
        parser.advance(&mut p, b"\x1b[1;2:3:4m");
        assert_eq!(p.0, Some((2, vec![2, 3, 4], true)));
    }

    #[test]
    fn csi_param_overflow_saturates_and_extra_params_are_dropped() {
        let ev = parse(b"\x1b[99999999999999999999m");
        assert_eq!(ev, vec![csi(&[&[u32::MAX]], &[], None, b'm')]);

        let mut input = b"\x1b[".to_vec();
        for _ in 0..1000 {
            input.extend_from_slice(b"1;");
        }
        input.push(b'm');
        match &parse(&input)[0] {
            Ev::Csi { params, .. } => assert_eq!(params.len(), MAX_PARAMS),
            e => panic!("{e:?}"),
        }
    }

    #[test]
    fn csi_split_across_writes() {
        let ev = parse_all_splits(b"a\x1b[?25;1:2$pb");
        assert_eq!(
            ev,
            vec![
                Ev::Print('a'),
                csi(&[&[25], &[1, 2]], b"$", Some(b'?'), b'p'),
                Ev::Print('b'),
            ]
        );
    }

    // ---- OSC ----

    #[test]
    fn osc_terminated_by_bel_and_st() {
        assert_eq!(
            parse(b"\x1b]0;title\x07"),
            vec![Ev::Osc(b"0;title".to_vec(), true)]
        );
        assert_eq!(
            parse(b"\x1b]0;title\x1b\\x"),
            vec![Ev::Osc(b"0;title".to_vec(), false), Ev::Print('x')]
        );
    }

    #[test]
    fn osc_keeps_0x9c_and_high_bytes() {
        // 「あ」(E3 81 82) と「œ」(C5 93)、「連」(E9 80 A3)、「ᅜ」(E1 85 9C): 継続バイトに 0x9C。
        let text = "\u{115C}連œあ";
        let mut input = b"\x1b]2;".to_vec();
        input.extend_from_slice(text.as_bytes());
        input.push(0x07);
        let ev = parse_all_splits(&input);
        let mut want = b"2;".to_vec();
        want.extend_from_slice(text.as_bytes());
        assert_eq!(ev, vec![Ev::Osc(want, true)]);
        assert!(text.as_bytes().contains(&0x9C));
        // 生の 0x9C だけでも終わらない。
        assert_eq!(
            parse(&[0x1B, b']', b'0', b';', 0x9C, 0x07]),
            vec![Ev::Osc(vec![b'0', b';', 0x9C], true)]
        );
    }

    #[test]
    fn osc_c0_is_dropped_and_can_sub_cancel() {
        assert_eq!(
            parse(b"\x1b]0;a\nb\x00c\x07"),
            vec![Ev::Osc(b"0;abc".to_vec(), true)]
        );
        assert_eq!(
            parse(b"\x1b]0;abc\x18d"),
            vec![Ev::Exec(0x18), Ev::Print('d')]
        );
        assert_eq!(
            parse(b"\x1b]0;abc\x1ad"),
            vec![Ev::Exec(0x1A), Ev::Print('d')]
        );
    }

    #[test]
    fn osc_esc_followed_by_other_starts_new_sequence() {
        assert_eq!(
            parse(b"\x1b]0;a\x1b[1m"),
            vec![
                Ev::Osc(b"0;a".to_vec(), false),
                csi(&[&[1]], &[], None, b'm')
            ]
        );
    }

    #[test]
    fn osc_split_across_writes() {
        let ev = parse_all_splits(b"\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x07");
        assert_eq!(
            ev,
            vec![
                Ev::Osc(b"8;;https://example.com".to_vec(), false),
                Ev::Print('l'),
                Ev::Print('i'),
                Ev::Print('n'),
                Ev::Print('k'),
                Ev::Osc(b"8;;".to_vec(), true),
            ]
        );
    }

    #[test]
    fn empty_osc_is_dispatched() {
        assert_eq!(parse(b"\x1b]\x07"), vec![Ev::Osc(vec![], true)]);
    }

    // ---- DCS ----

    #[test]
    fn dcs_dispatch_with_params_and_data() {
        assert_eq!(
            parse(b"\x1bP1;2$qm\x1b\\"),
            vec![Ev::Dcs {
                params: vec![vec![1], vec![2]],
                inter: vec![b'$'],
                marker: None,
                action: b'q',
                data: b"m".to_vec(),
            }]
        );
        assert_eq!(
            parse(b"\x1bP+q544e\x1b\\"),
            vec![Ev::Dcs {
                params: vec![],
                inter: vec![b'+'],
                marker: None,
                action: b'q',
                data: b"544e".to_vec(),
            }]
        );
        assert_eq!(
            parse(b"\x1bP=1sx\x1b\\"),
            vec![Ev::Dcs {
                params: vec![vec![1]],
                inter: vec![],
                marker: Some(b'='),
                action: b's',
                data: b"x".to_vec(),
            }]
        );
    }

    #[test]
    fn dcs_keeps_high_bytes_and_split() {
        let mut input = b"\x1bPqab".to_vec();
        input.extend_from_slice("\u{115C}".as_bytes());
        input.extend_from_slice(b"\x1b\\z");
        let ev = parse_all_splits(&input);
        assert_eq!(ev.len(), 2);
        match &ev[0] {
            Ev::Dcs { data, .. } => {
                let mut want = b"ab".to_vec();
                want.extend_from_slice("\u{115C}".as_bytes());
                assert_eq!(data, &want);
            }
            e => panic!("{e:?}"),
        }
    }

    #[test]
    fn dcs_invalid_is_ignored_until_st() {
        // 中間バイトのあとの数字。
        assert_eq!(parse(b"\x1bP$1qdata\x1b\\a"), vec![Ev::Print('a')]);
        // 先頭以外の印。
        assert_eq!(parse(b"\x1bP1?qdata\x1b\\a"), vec![Ev::Print('a')]);
    }

    #[test]
    fn dcs_cancelled_by_can() {
        assert_eq!(
            parse(b"\x1bPqdata\x18a"),
            vec![Ev::Exec(0x18), Ev::Print('a')]
        );
    }

    #[test]
    fn dcs_c0_in_header_is_ignored() {
        assert_eq!(
            parse(b"\x1bP1\n2qx\x1b\\"),
            vec![Ev::Dcs {
                params: vec![vec![12]],
                inter: vec![],
                marker: None,
                action: b'q',
                data: b"x".to_vec(),
            }]
        );
    }

    // ---- APC ----

    #[test]
    fn apc_dispatch_kitty_graphics() {
        let ev = parse_all_splits(b"\x1b_Ga=T,f=100;iVBORw0KGgo=\x1b\\");
        assert_eq!(ev, vec![Ev::Apc(b"Ga=T,f=100;iVBORw0KGgo=".to_vec())]);
    }

    #[test]
    fn apc_keeps_0x9c_and_c0_inside() {
        let mut input = b"\x1b_G".to_vec();
        input.extend_from_slice(&[0xE1, 0x85, 0x9C, b'\n', 0x07, 0x9C]);
        input.extend_from_slice(b"\x1b\\");
        let ev = parse_all_splits(&input);
        assert_eq!(
            ev,
            vec![Ev::Apc(vec![b'G', 0xE1, 0x85, 0x9C, b'\n', 0x07, 0x9C])]
        );
    }

    #[test]
    fn apc_is_not_ended_by_bel() {
        assert_eq!(
            parse(b"\x1b_Ga\x07b\x1b\\"),
            vec![Ev::Apc(b"Ga\x07b".to_vec())]
        );
    }

    #[test]
    fn apc_cancelled_by_can_sub_without_dispatch() {
        assert_eq!(
            parse(b"\x1b_Gabc\x18x"),
            vec![Ev::Exec(0x18), Ev::Print('x')]
        );
        assert_eq!(
            parse(b"\x1b_Gabc\x1ax"),
            vec![Ev::Exec(0x1A), Ev::Print('x')]
        );
    }

    #[test]
    fn c0_after_string_esc_keeps_st_pending() {
        assert_eq!(
            parse(b"\x1b_Gx\x1b\n\\a"),
            vec![Ev::Apc(b"Gx".to_vec()), Ev::Exec(b'\n'), Ev::Print('a')]
        );
    }

    #[test]
    fn apc_terminated_by_esc_only_swallows_backslash() {
        // ST の `\` は読み捨てる。それ以外は新しいシーケンス。
        assert_eq!(
            parse(b"\x1b_Gx\x1b\\\\"),
            vec![Ev::Apc(b"Gx".to_vec()), Ev::Print('\\')]
        );
        assert_eq!(
            parse(b"\x1b_Gx\x1b7"),
            vec![Ev::Apc(b"Gx".to_vec()), Ev::Esc(vec![], b'7')]
        );
    }

    // ---- SOS / PM ----

    #[test]
    fn sos_and_pm_are_discarded() {
        assert_eq!(parse(b"\x1bXsecret\x1b\\a"), vec![Ev::Print('a')]);
        assert_eq!(parse(b"\x1b^secret\x1b\\a"), vec![Ev::Print('a')]);
        // 0x9C や BEL では終わらない。
        assert_eq!(
            parse(&[0x1B, b'X', 0x9C, 0x07, b'x', 0x1B, b'\\', b'a']),
            vec![Ev::Print('a')]
        );
    }

    // ---- 上限 ----

    #[test]
    fn string_length_is_capped() {
        let mut parser = Parser::with_max_string_len(4);
        let mut rec = Rec::default();
        parser.advance(
            &mut rec,
            b"\x1b]0;abcdefgh\x07\x1b_Gabcdefgh\x1b\\\x1bPqabcdefgh\x1b\\",
        );
        assert_eq!(
            rec.0,
            vec![
                Ev::Osc(b"0;ab".to_vec(), true),
                Ev::Apc(b"Gabc".to_vec()),
                Ev::Dcs {
                    params: vec![],
                    inter: vec![],
                    marker: None,
                    action: b'q',
                    data: b"abcd".to_vec(),
                },
            ]
        );
        // 上限のあとも、次の列は正しく読める。
        parser.advance(&mut rec, b"z");
        assert_eq!(rec.0.last(), Some(&Ev::Print('z')));
    }

    #[test]
    fn default_limit_is_2mib() {
        let mut input = b"\x1b]0;".to_vec();
        input.extend(std::iter::repeat_n(b'a', DEFAULT_MAX_STRING_LEN + 100));
        input.push(0x07);
        match &parse(&input)[0] {
            Ev::Osc(d, true) => assert_eq!(d.len(), DEFAULT_MAX_STRING_LEN),
            e => panic!("{:?}", std::mem::discriminant(e)),
        }
    }

    // ---- UTF-8 が書き込みの途中で切れる ----

    #[test]
    fn utf8_split_across_writes() {
        let ev = parse_all_splits("a日😀b".as_bytes());
        assert_eq!(
            ev,
            vec![
                Ev::Print('a'),
                Ev::Print('日'),
                Ev::Print('😀'),
                Ev::Print('b')
            ]
        );
    }

    #[test]
    fn utf8_pending_at_end_of_write_is_not_flushed() {
        let mut parser = Parser::new();
        let mut rec = Rec::default();
        parser.advance(&mut rec, &"日".as_bytes()[..2]);
        assert!(rec.0.is_empty());
        parser.advance(&mut rec, &"日".as_bytes()[2..]);
        assert_eq!(rec.0, vec![Ev::Print('日')]);
    }

    #[test]
    fn mixed_stream_is_split_invariant() {
        let input = "x\x1b[1;38:2::1:2:3mé\x1b]0;タイトル\x07\x1bP1$rm\x1b\\\x1b_Ga=q\x1b\\\x1bXjunk\x1b\\日\x1b(0y".as_bytes();
        let ev = parse_all_splits(input);
        assert!(ev.contains(&Ev::Print('日')));
        assert!(ev.contains(&Ev::Apc(b"Ga=q".to_vec())));
    }

    // ---- ランダム入力 ----

    struct XorShift(u64);
    impl XorShift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    /// 何も panic しない。書き込みの区切り方で結果が変わらない。
    #[test]
    fn random_bytes_do_not_panic() {
        const SPECIAL: &[u8] = b"\x1b\x18\x1a\x07\x9c\x90\x9b\x9d\x98\x9e\x9f[]P_X^\\;:?><= 0123456789mHqG\xe3\xe1\x85\xf0\xf4\xc0\xff\x7f\x00\n";
        let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
        let mut parser = Parser::with_max_string_len(64);
        let mut rec = Rec::default();
        for _ in 0..4000 {
            let len = (rng.next() % 300) as usize;
            let biased = rng.next().is_multiple_of(2);
            let buf: Vec<u8> = (0..len)
                .map(|_| {
                    let r = rng.next();
                    if biased {
                        SPECIAL[(r >> 8) as usize % SPECIAL.len()]
                    } else {
                        (r >> 8) as u8
                    }
                })
                .collect();
            // 同じパーサーを使い続け、ランダムな位置で割って渡す。
            let cut = if buf.is_empty() {
                0
            } else {
                (rng.next() as usize) % buf.len()
            };
            parser.advance(&mut rec, &buf[..cut]);
            parser.advance(&mut rec, &buf[cut..]);
            rec.0.clear();

            // 割り方で結果が変わらない (新しいパーサーで確認)。
            let mut whole = Parser::with_max_string_len(64);
            let mut a = Rec::default();
            whole.advance(&mut a, &buf);
            let mut split = Parser::with_max_string_len(64);
            let mut b = Rec::default();
            split.advance(&mut b, &buf[..cut]);
            split.advance(&mut b, &buf[cut..]);
            assert_eq!(a.0, b.0);
        }
    }

    #[test]
    fn every_single_byte_after_every_state_does_not_panic() {
        let prefixes: &[&[u8]] = &[
            b"",
            b"\x1b",
            b"\x1b ",
            b"\x1b[",
            b"\x1b[1",
            b"\x1b[?",
            b"\x1b[ ",
            b"\x1b[1?",
            b"\x1bP",
            b"\x1bP1",
            b"\x1bP ",
            b"\x1bPq",
            b"\x1bP1?",
            b"\x1b]",
            b"\x1b_",
            b"\x1bX",
            b"\x1b]0;\x1b",
            b"\xe3",
            b"\xe3\x81",
        ];
        for prefix in prefixes {
            for b in 0..=255u8 {
                let mut parser = Parser::new();
                let mut rec = Rec::default();
                parser.advance(&mut rec, prefix);
                parser.advance(&mut rec, &[b]);
                parser.advance(&mut rec, b"\x1b\\a");
            }
        }
    }
}
