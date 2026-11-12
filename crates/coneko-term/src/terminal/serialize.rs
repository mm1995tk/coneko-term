//! 画面の状態を、別の [`Terminal`] に書けば同じ画面になるバイト列として書き出す
//! （[`Terminal::serialize`]）。
//!
//! 書き出すもの:
//!
//! - 主画面の scrollback（呼ぶ側が行数を決める）と見えている画面。文字と属性（SGR）、
//!   折り返した行のつながり。
//! - 代替画面に切り替わっていれば、代替画面の中身。そのとき主画面も（戻ったときのために）書く。
//! - カーソルの位置（折り返し待ちを含む）と、DECSC で保存したカーソル、スクロール範囲、タブ位置。
//! - カーソルの形・点滅・表示、字を書くときの属性。
//! - kitty keyboard protocol のフラグの積み重ね（主画面と代替画面で別々、積んだ段も）。
//! - モード: DECCKM・DECOM・自動折り返し・逆方向の折り返し・アプリケーションキーパッド・
//!   フォーカスの報告・bracketed paste・同期した出力・クリップボードの貼り付けイベント・
//!   マウス報告の種類と書き方・IRM・LNM。
//! - 文字・背景・カーソルの色とパレットのうち、[`Theme`](super::Theme) の色から変わっているもの。
//!   書く側と書かれる側のテーマが同じであることを前提にする。
//!
//! - シェルが入力を受けている行の印（OSC 133 の `B` と `D` にして書く。`C` は、一度でも出していたときだけ）。
//!   書かれる側がこの印を戻すので、書き直したあとも、コマンドの行は出力の行と見分けられる。
//!
//! - kitty graphics の画像と配置（下の「画像」の節）。
//!
//! 書き出さないもの: 文字集合（G0〜G3）、`CSI ? s` で保存した
//! モード、追加のカーソル、色の積み重ね、選択範囲の色など OSC 21 だけで変わる色、
//! 通知・クリップボード・drag and drop・file transfer の途中の状態、題名、プログラムの状態の記録。
//! 複数の行にまたがる文字（OSC 66 で `s` が 2 以上）は、scrollback の中では書かない。
//!
//! # 画像
//!
//! kitty graphics の画像を `a=t`（`i=` で ID を名指し、`I=` の番号があれば番号も、`q=2` で返事なし）
//! で送り直し、配置を `a=p`（`C=1` でカーソルを動かさない）で置き直す。画像が 1 つも無ければ何も
//! 書かない（バイト列は画像を扱う前と同じ）。
//!
//! - 送る画像は、書き出す配置（見えている画面、書き出す範囲の scrollback、代替画面、`U=1` の仮の
//!   配置）が使っているものだけ。配置の無い画像は送らない（見えないうえ、上限まで溜まると数百 MB に
//!   なるため）。そのため、送っただけで置いていない画像を、書き戻したあとに `a=p` で置くことはできない。
//! - 送る画像のデータの合計は [`MAX_IMAGE_BYTES`] まで。超えるときは、見えている画面の下の配置から
//!   上へ、次に scrollback の新しい行から古い行へと選び、入らない画像（とその配置）は書かない。
//! - ID と番号は保つ（書き戻したあとに、シェルの側のプログラムが同じ ID・番号で置き直したり
//!   消したりできるように）。`I=` で送った画像の ID はターミナルが決めた ID なので、`i=` と `I=` を
//!   両方付けて送る（[`crate::graphics`] はこの組み合わせを、ID がターミナルの決める範囲のときだけ
//!   受け入れる）。ID が 0 の画像は名指しできないので送らない。
//! - 配置は、その配置を持つ行を書いたあとに置く（折り返しでつながる行は、つながりを書き終えてから、
//!   カーソルを上に戻して置く）。行と一緒に scrollback へ押し出されるので、scrollback の中の配置も
//!   同じ行に戻る。カーソルは、そのあといつもどおり置き直す。
//! - `c`・`r` を決めずに（画素のまま）置いた配置は、`c`・`r` を書かない。書かれる側のセルの大きさで
//!   セルの数を決め直す。
//! - アニメーションは送らない。根のコマ（1 コマ目）だけの止まった画像になる。コマの操作は数も
//!   大きさもあり、再生の状態も書かれる側では時計がずれるため。
//! - 書き出す範囲より古い scrollback の行に付いた配置は、下の行まで画像が伸びていても書かない。
//!   scrollback の中で、画面の行数より長く折り返してつながった行の、上のほう（カーソルを戻して
//!   届かない行）に付いた配置も書かない。
//!
//! 問い合わせ（DA・DSR・DECRQM・`CSI ? u` など、返事を求めるシーケンス）は入れない。
//! 書き出したバイト列は、先頭の RIS で書かれる側を初期状態に戻してから状態を作るので、
//! 新しい `Terminal` に書いても [`Terminal::take_output`] は空のまま。

use std::collections::HashSet;
use std::fmt::Write as _;

use super::clipboard::encode_base64;
use super::{MouseEncoding, MouseProtocol, Terminal};
use crate::graphics::{Image, ImageFormat, Placement};
use crate::screen::cell::{AttrFlags, Attrs, Cell, Color, Content, SizedBlock, UnderlineStyle};
use crate::screen::{Align, Buffer, CursorShape, Line};

/// 書き出す画像のデータ（base64 にする前）の合計の上限（バイト）。
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;

/// 画像のデータを送るときの、1 つの APC に入れる base64 の字数（kitty の推奨と同じ）。
const CHUNK: usize = 4096;

impl Terminal {
    /// 画面の状態を、新しい `Terminal`（同じ大きさ）に書けば同じ画面になるバイト列にして返す。
    ///
    /// `max_scrollback` は、主画面の scrollback を新しいほうから何行まで書くか。
    /// 何を書き何を書かないかは、このモジュールの説明を見ること。
    /// 返すバイト列に問い合わせは入っていない。書いても `take_output` は空のまま。
    pub fn serialize(&self, max_scrollback: usize) -> Vec<u8> {
        self.serialize_with_limit(max_scrollback, MAX_IMAGE_BYTES)
    }

    /// [`Terminal::serialize`] の、画像のデータの上限を決められるもの。
    pub(super) fn serialize_with_limit(
        &self,
        max_scrollback: usize,
        image_limit: usize,
    ) -> Vec<u8> {
        let st = &self.state;
        let screen = &st.screen;
        let (primary, alt) = screen.buffers();
        let alt_active = screen.is_alt_screen();
        let mut out = Out {
            images: self.images_to_write(max_scrollback, image_limit),
            ..Out::default()
        };

        // 書かれる側を初期状態に戻す。
        out.raw("\x1bc");
        if st.shell_marks_output {
            // 出力の始まりを出すシェルだと覚えさせる（A のプロンプトの印を、幅が変わったときの組み直しに使うため）
            out.raw("\x1b]133;C\x07");
        }

        // 画像のデータ（持っている順。書かれる側で、古いものから捨てる順が同じになる）と、仮の配置。
        // 画面の配置は、行を書くときに置く。
        for image in st.graphics.iter() {
            if out.images.contains(&image.id) {
                out.transmit(image);
            }
        }
        for v in st.graphics.virtuals() {
            if out.images.contains(&v.image_id) {
                out.place(v, true);
            }
        }

        // 主画面。
        out.buffer(primary, max_scrollback, true);
        out.keyboard_stack(st.keyboard_flags[0].entries());
        if alt_active {
            // 1047 は保存のカーソルを動かさず、主画面のカーソルを代替画面へ引き継ぐだけ。
            out.raw("\x1b[?1047h");
            out.buffer(alt, 0, false);
            out.keyboard_stack(st.keyboard_flags[1].entries());
        }

        out.input_state(st.shell_input);
        out.colors(&st.colors, &st.theme);

        // モード。字を書き終えてから立てる（自動折り返しや挿入モードが書き出しを邪魔しない）。
        let m = &st.modes;
        out.mode(1, m.application_cursor_keys);
        out.mode(45, m.reverse_wraparound);
        out.mode(66, m.application_keypad);
        out.mode(1004, m.send_focus);
        out.mode(2004, m.bracketed_paste);
        out.mode(2026, m.synchronized_output);
        out.mode(5522, m.clipboard_paste_events);
        match m.mouse_protocol {
            MouseProtocol::X10 => out.mode(9, true),
            MouseProtocol::Vt200 => out.mode(1000, true),
            MouseProtocol::Drag => out.mode(1002, true),
            MouseProtocol::Any => out.mode(1003, true),
            _ => {}
        }
        match m.mouse_encoding {
            MouseEncoding::Utf8 => out.mode(1005, true),
            MouseEncoding::Sgr => out.mode(1006, true),
            MouseEncoding::Urxvt => out.mode(1015, true),
            MouseEncoding::SgrPixels => out.mode(1016, true),
            _ => {}
        }
        if !screen.wraparound {
            out.mode(7, false);
        }
        if !screen.cursor_visible {
            out.mode(25, false);
        }

        // 原点モードは、立てるとカーソルが左上へ行く。立ててから、動いている画面のカーソルを置く。
        let active = if alt_active { alt } else { primary };
        if m.origin {
            out.mode(6, true);
        }
        out.place_cursor(
            active,
            active.cursor.x,
            active.cursor.y,
            active.cursor.pending_wrap,
            m.origin,
        );

        if screen.insert_mode {
            out.raw("\x1b[4h");
        }
        if m.linefeed_newline {
            out.raw("\x1b[20h");
        }
        let style = match (screen.cursor_shape, screen.cursor_blinking) {
            (CursorShape::Block, true) => 1,
            (CursorShape::Block, false) => 2,
            (CursorShape::Underline, true) => 3,
            (CursorShape::Underline, false) => 4,
            (CursorShape::Bar, true) => 5,
            (CursorShape::Bar, false) => 6,
        };
        let _ = write!(out.buf, "\x1b[{style} q");
        out.set_pen(screen.pen);
        out.buf.into_bytes()
    }
}

impl Terminal {
    /// 書き出す画像の ID。書き出す配置が使う画像を、代替画面と主画面の見えている画面の下から上へ、
    /// 仮の配置、主画面の scrollback の新しい行から古い行へ、の順に選び、データの合計が `limit` を
    /// 超える画像は飛ばす。
    fn images_to_write(&self, max_scrollback: usize, limit: usize) -> HashSet<u32> {
        let st = &self.state;
        let (primary, alt) = st.screen.buffers();
        let ids = |lines: &[&Line]| -> Vec<u32> {
            lines
                .iter()
                .rev()
                .flat_map(|l| l.placements.iter().map(|p| p.image_id))
                .collect()
        };
        let total = primary.scrollback_len();
        let scrollback: Vec<&Line> = primary
            .lines
            .iter()
            .skip(total - total.min(max_scrollback))
            .take(total.min(max_scrollback))
            .collect();
        let visible: Vec<&Line> = primary.lines.iter().skip(total).collect();
        let mut wanted: Vec<u32> = Vec::new();
        if st.screen.is_alt_screen() {
            wanted.extend(ids(&alt.lines.iter().collect::<Vec<_>>()));
        }
        wanted.extend(ids(&visible));
        wanted.extend(st.graphics.virtuals().iter().map(|v| v.image_id));
        wanted.extend(ids(&scrollback));

        let mut chosen = HashSet::new();
        let mut bytes = 0usize;
        for id in wanted {
            if id == 0 || chosen.contains(&id) {
                continue;
            }
            let Some(image) = st.graphics.get(id) else {
                continue;
            };
            let n = image.data.len();
            if bytes + n > limit {
                continue;
            }
            bytes += n;
            chosen.insert(id);
        }
        chosen
    }
}

/// 出力と、いま書き手に効いている属性。
#[derive(Default)]
struct Out {
    buf: String,
    pen: Attrs,
    /// 書き手が、シェルの入力を受けている間（OSC 133 の `B` のあと）にいるか。
    shell_input: bool,
    /// 書き出す画像の ID。この画像の配置だけを置く。
    images: HashSet<u32>,
}

impl Out {
    /// 画像のデータを送る（`a=t`。返事はさせない）。
    fn transmit(&mut self, image: &Image) {
        let format = match image.format {
            ImageFormat::Rgb => 24,
            ImageFormat::Rgba => 32,
            ImageFormat::Png => 100,
        };
        let mut head = format!("a=t,i={},f={format}", image.id);
        if let Some(n) = image.number {
            let _ = write!(head, ",I={n}");
        }
        if image.compressed {
            head.push_str(",o=z");
        }
        if image.width > 0 {
            let _ = write!(head, ",s={}", image.width);
        }
        if image.height > 0 {
            let _ = write!(head, ",v={}", image.height);
        }
        head.push_str(",q=2");
        let data = encode_base64(&image.data);
        let chunks: Vec<&str> = data
            .as_bytes()
            .chunks(CHUNK)
            .map(|c| std::str::from_utf8(c).unwrap_or_default())
            .collect();
        for (k, chunk) in chunks.iter().enumerate() {
            let more = k + 1 < chunks.len();
            self.buf.push_str("\x1b_G");
            if k == 0 {
                self.buf.push_str(&head);
                if more {
                    self.buf.push_str(",m=1");
                }
            } else {
                self.buf.push_str(if more { "m=1" } else { "m=0" });
            }
            self.buf.push(';');
            self.buf.push_str(chunk);
            self.buf.push_str("\x1b\\");
        }
    }

    /// 配置を、いまのカーソルの位置に置く（`a=p`。カーソルは動かさず、返事はさせない）。
    /// `virtual_placement` なら、`U=1` の仮の配置にする。
    fn place(&mut self, p: &Placement, virtual_placement: bool) {
        let _ = write!(self.buf, "\x1b_Ga=p,i={}", p.image_id);
        if p.placement_id != 0 {
            let _ = write!(self.buf, ",p={}", p.placement_id);
        }
        for (key, v) in [
            ('x', p.src_x),
            ('y', p.src_y),
            ('w', p.src_w),
            ('h', p.src_h),
            ('X', p.offset_x),
            ('Y', p.offset_y),
        ] {
            if v != 0 {
                let _ = write!(self.buf, ",{key}={v}");
            }
        }
        // 画素のままの配置は、書かれる側のセルの大きさでセルの数を決め直す。
        if virtual_placement || !p.native_size {
            let _ = write!(self.buf, ",c={},r={}", p.cols, p.rows);
        }
        if p.z != 0 {
            let _ = write!(self.buf, ",z={}", p.z);
        }
        if virtual_placement {
            self.buf.push_str(",U=1");
        }
        self.buf.push_str(",C=1,q=2\x1b\\");
    }

    /// 行 `lines` に付いた配置を置く。`absolute` なら行 `j` は画面の `j` 行目。そうでなければ、
    /// カーソルは最後の行（`last`）にいて、行 `j` はその `last - j` 行上（`rows` 行より上は届かない）。
    fn line_placements(
        &mut self,
        lines: &[&Line],
        from: usize,
        last: usize,
        absolute: bool,
        rows: usize,
    ) {
        for (j, line) in lines.iter().enumerate().take(last + 1).skip(from) {
            let ps: Vec<&Placement> = line
                .placements
                .iter()
                .filter(|p| self.images.contains(&p.image_id))
                .collect();
            if ps.is_empty() {
                continue;
            }
            let up = last - j;
            if !absolute && up >= rows {
                continue;
            }
            if !absolute && up > 0 {
                let _ = write!(self.buf, "\x1b[{up}A");
            }
            for p in ps {
                if absolute {
                    self.cup(p.col, j);
                } else {
                    self.cha(p.col);
                }
                self.place(p, false);
            }
            if !absolute && up > 0 {
                let _ = write!(self.buf, "\x1b[{up}B");
            }
        }
    }

    /// 次に書く字が、シェルの入力の行になるかどうかを、書かれる側に伝える。
    fn input_state(&mut self, on: bool) {
        if self.shell_input != on {
            self.shell_input = on;
            self.raw(if on {
                "\x1b]133;B\x07"
            } else {
                "\x1b]133;D\x07"
            });
        }
    }

    fn raw(&mut self, s: &str) {
        self.buf.push_str(s);
    }

    fn mode(&mut self, n: u32, on: bool) {
        if on {
            let _ = write!(self.buf, "\x1b[?{n}h");
        } else {
            let _ = write!(self.buf, "\x1b[?{n}l");
        }
    }

    fn cup(&mut self, x: usize, y: usize) {
        let _ = write!(self.buf, "\x1b[{};{}H", y + 1, x + 1);
    }

    fn cha(&mut self, x: usize) {
        let _ = write!(self.buf, "\x1b[{}G", x + 1);
    }

    fn keyboard_stack(&mut self, entries: &[u32]) {
        for f in entries {
            let _ = write!(self.buf, "\x1b[>{f}u");
        }
    }

    /// 属性を `a` にする。いまと同じなら何も書かない。
    fn set_pen(&mut self, a: Attrs) {
        if a == self.pen {
            return;
        }
        self.pen = a;
        self.buf.push_str("\x1b[0");
        let f = a.flags;
        for (flag, code) in [
            (AttrFlags::BOLD, 1),
            (AttrFlags::DIM, 2),
            (AttrFlags::ITALIC, 3),
            (AttrFlags::BLINK, 5),
            (AttrFlags::INVERSE, 7),
            (AttrFlags::INVISIBLE, 8),
            (AttrFlags::STRIKETHROUGH, 9),
        ] {
            if f.contains(flag) {
                let _ = write!(self.buf, ";{code}");
            }
        }
        match a.underline.style {
            UnderlineStyle::None => {}
            UnderlineStyle::Single => self.buf.push_str(";4"),
            UnderlineStyle::Double => self.buf.push_str(";4:2"),
            UnderlineStyle::Curly => self.buf.push_str(";4:3"),
            UnderlineStyle::Dotted => self.buf.push_str(";4:4"),
            UnderlineStyle::Dashed => self.buf.push_str(";4:5"),
        }
        self.color(38, a.fg);
        self.color(48, a.bg);
        self.color(58, a.underline.color);
        self.buf.push('m');
    }

    fn color(&mut self, base: u32, c: Color) {
        match c {
            Color::Default => {}
            Color::Palette(n) => {
                let _ = write!(self.buf, ";{base};5;{n}");
            }
            Color::Rgb(r, g, b) => {
                let _ = write!(self.buf, ";{base};2;{r};{g};{b}");
            }
        }
    }

    /// 文字・背景・カーソルの色とパレットのうち、テーマから変わっているもの。
    fn colors(&mut self, cur: &super::Colors, theme: &super::Theme) {
        let base = super::Colors::from_theme(theme);
        let rgb = |c: [u8; 3]| format!("rgb:{:02x}/{:02x}/{:02x}", c[0], c[1], c[2]);
        for (n, a, b) in [
            (10, cur.foreground, base.foreground),
            (11, cur.background, base.background),
            (12, cur.cursor, base.cursor),
        ] {
            if a != b {
                let _ = write!(self.buf, "\x1b]{n};{}\x1b\\", rgb(a));
            }
        }
        for i in 0..256 {
            if cur.palette[i] != base.palette[i] {
                let _ = write!(self.buf, "\x1b]4;{i};{}\x1b\\", rgb(cur.palette[i]));
            }
        }
    }

    /// 1 枚分の画面を書く。`scrollback` が真なら、新しいほうから `max_scrollback` 行の scrollback も。
    fn buffer(&mut self, buf: &Buffer, max_scrollback: usize, scrollback: bool) {
        let (rows, cols) = (buf.rows, buf.cols);
        if scrollback {
            let total = buf.scrollback_len();
            let n = total.min(max_scrollback);
            if n > 0 {
                let lines: Vec<&Line> = (total - n..total).map(|i| &buf.lines[i]).collect();
                self.rows(&lines, cols, rows, false);
                // 書いた行を、画面の外（scrollback）へ押し出す。
                self.set_pen(Attrs::default());
                self.cup(0, rows - 1);
                for _ in 0..n.min(rows) {
                    self.buf.push('\n');
                }
            }
        }
        let lines: Vec<&Line> = (0..rows).map(|y| buf.line(y)).collect();
        self.rows(&lines, cols, rows, true);
        self.set_pen(Attrs::default());

        // タブ位置が初めのままでなければ、置き直す。
        let default_tabs: Vec<bool> = (0..cols).map(|x| x % 8 == 0 && x != 0).collect();
        if buf.tabs != default_tabs {
            self.raw("\x1b[3g");
            for (x, _) in buf.tabs.iter().enumerate().filter(|(_, &t)| t) {
                self.cha(x);
                self.raw("\x1bH");
            }
        }
        // スクロール範囲（決めるとカーソルが左上へ行く）。
        if (buf.scroll_top, buf.scroll_bottom) != (0, rows - 1) {
            let _ = write!(
                self.buf,
                "\x1b[{};{}r",
                buf.scroll_top + 1,
                buf.scroll_bottom + 1
            );
        }
        if let Some(s) = buf.saved {
            self.place_cursor(buf, s.x, s.y, s.pending_wrap, false);
            self.set_pen(s.attrs);
            self.raw("\x1b7");
        }
        // 動いていない主画面のカーソル。動いている画面のカーソルは、最後に置き直す。
        self.place_cursor(
            buf,
            buf.cursor.x,
            buf.cursor.y,
            buf.cursor.pending_wrap,
            false,
        );
    }

    /// カーソルを置く。`origin` が真なら、スクロール範囲の上端からの行で。折り返し待ちは、
    /// 右端のセルの字を書き直して作る（字は変わらない）。
    fn place_cursor(&mut self, buf: &Buffer, x: usize, y: usize, pending: bool, origin: bool) {
        let row = if origin {
            y.saturating_sub(buf.scroll_top)
        } else {
            y
        };
        self.cup(x, row);
        if !pending {
            return;
        }
        let cells = buf.line(y).cells();
        let Some(cell) = cells.get(x) else { return };
        let (at, cell) = match cell.content {
            Content::WideTail if x > 0 => (x - 1, &cells[x - 1]),
            _ => (x, cell),
        };
        if matches!(cell.content, Content::Char(_) | Content::Cluster(_)) {
            self.cup(at, row);
            self.set_pen(cell.attrs);
            self.text(cell);
        } else if cell.content == Content::Empty {
            // 空のセルは字を書けない。空白を書いてから ECH で消す（ECH は折り返し待ちを動かさない）。
            self.set_pen(cell.attrs);
            self.raw(" \x1b[1X");
        }
    }

    fn text(&mut self, cell: &Cell) {
        match &cell.content {
            Content::Char(c) if !c.is_control() => self.buf.push(*c),
            Content::Cluster(s) => self.buf.push_str(s),
            _ => {}
        }
    }

    /// 行を上から順に書く。`absolute` が真なら各行の頭へ CUP で行く（見えている画面）。
    /// 偽なら CR LF でつなぐ（最初の行は、カーソルがいるところから。画面は `rows` 行）。
    /// 行に付いた画像の配置も置く。
    fn rows(&mut self, lines: &[&Line], cols: usize, rows: usize, absolute: bool) {
        // 前の行が折り返していて、続きの行がその続きから書けるとき、行の頭へ行かずにそのまま書く。
        let mut carry = false;
        // 折り返しでつながっている行の、最初の行。
        let mut start = 0;
        for (i, line) in lines.iter().enumerate() {
            if !carry {
                start = i;
                if absolute {
                    self.cup(0, i);
                } else if i > 0 {
                    self.set_pen(Attrs::default());
                    self.raw("\r\n");
                }
            }
            self.input_state(line.is_input());
            let cx = self.row(line.cells(), absolute);
            carry = self.carry(line, lines.get(i + 1), cx, cols, absolute);
            if !carry {
                // 配置を置くとカーソルが動き、折り返し待ちが消えるので、つながりを書き終えてから置く。
                self.line_placements(lines, start, i, absolute, rows);
            }
        }
    }

    /// 書き終えた行 `line`（カーソルは列 `cx`）の次の行 `next` を、行の頭へ行かずにそのまま
    /// 書けるか。書けるならそのための位置へカーソルを置く。
    fn carry(
        &mut self,
        line: &Line,
        next: Option<&&Line>,
        cx: usize,
        cols: usize,
        absolute: bool,
    ) -> bool {
        let Some(next) = next else { return false };
        if !line.is_wrapped() {
            return false;
        }
        let first = &next.cells()[0];
        if cx == cols && starts_text(first, absolute) {
            true
        } else if cx < cols
            && is_plain_blank(&line.cells()[cols - 1])
            && first.is_wide()
            && matches!(first.content, Content::Char(_))
        {
            // 全角の字が入りきらずに折り返した行。右端で次の全角の字を書くと、折り返す。
            self.cha(cols - 1);
            true
        } else {
            false
        }
    }

    /// 1 行のセルを書く。書き終えたときのカーソルの列を返す（行の幅と同じなら、折り返し待ち）。
    fn row(&mut self, cells: &[Cell], allow_tall: bool) -> usize {
        let cols = cells.len();
        let Some(last) = cells.iter().rposition(|c| !is_plain_blank(c)) else {
            return 0;
        };
        let mut cx = 0usize;
        let mut x = 0usize;
        while x <= last {
            let cell = &cells[x];
            match &cell.content {
                Content::Empty => {
                    let bg = cell.attrs.bg;
                    let mut e = x;
                    while e < last
                        && cells[e + 1].content == Content::Empty
                        && cells[e + 1].attrs.bg == bg
                    {
                        e += 1;
                    }
                    if bg != Color::Default {
                        if cx != x {
                            self.cha(x);
                            cx = x;
                        }
                        self.set_pen(Attrs {
                            bg,
                            ..Attrs::default()
                        });
                        let _ = write!(self.buf, "\x1b[{}X", e - x + 1);
                    }
                    x = e + 1;
                    continue;
                }
                Content::WideTail => {}
                Content::Sized {
                    block,
                    dx: 0,
                    dy: 0,
                } => {
                    if block.scale == 1 || allow_tall {
                        if cx != x {
                            self.cha(x);
                        }
                        self.set_pen(cell.attrs);
                        self.sized(block);
                        cx = x + usize::from(block.cols);
                    }
                }
                Content::Sized { .. } => {}
                Content::Char(_) | Content::Cluster(_) => {
                    if cx != x {
                        self.cha(x);
                    }
                    self.set_pen(cell.attrs);
                    self.text(cell);
                    cx = (x + usize::from(cell.width.max(1))).min(cols);
                }
            }
            x += 1;
        }
        cx
    }

    /// OSC 66。
    fn sized(&mut self, b: &SizedBlock) {
        let w = usize::from(b.cols) / usize::from(b.scale.max(1));
        let _ = write!(self.buf, "\x1b]66;s={}:w={w}", b.scale);
        if b.denominator != 0 {
            let code = |a: Align| match a {
                Align::Start => 0,
                Align::End => 1,
                Align::Center => 2,
            };
            let _ = write!(
                self.buf,
                ":n={}:d={}:v={}:h={}",
                b.numerator,
                b.denominator,
                code(b.vertical),
                code(b.horizontal)
            );
        }
        let _ = write!(self.buf, ";{}\x1b\\", b.text);
    }
}

/// 何も書かれておらず、背景色も既定のセル。
fn is_plain_blank(c: &Cell) -> bool {
    c.content == Content::Empty && c.attrs.bg == Color::Default
}

/// 折り返しの続きとして、字を書けるセルか。
fn starts_text(c: &Cell, allow_tall: bool) -> bool {
    match &c.content {
        Content::Char(_) | Content::Cluster(_) => true,
        Content::Sized {
            block,
            dx: 0,
            dy: 0,
        } => block.scale == 1 || allow_tall,
        _ => false,
    }
}
