use super::*;

fn row_text(s: &Screen, y: usize) -> String {
    s.row(y)
        .cells()
        .iter()
        .map(|c| match &c.content {
            Content::Empty => " ".to_string(),
            _ => c.text(),
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn scrollback_text(s: &Screen, i: usize) -> String {
    s.scrollback_row(i)
        .cells()
        .iter()
        .map(|c| {
            if c.is_empty() {
                " ".to_string()
            } else {
                c.text()
            }
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[test]
fn writes_chars_and_advances_cursor() {
    let mut s = Screen::new(3, 10);
    s.print_str("abc");
    assert_eq!(row_text(&s, 0), "abc");
    let c = s.cursor();
    assert_eq!((c.x, c.y, c.pending_wrap), (3, 0, false));
}

#[test]
fn written_cells_carry_pen_attrs() {
    let mut s = Screen::new(3, 10);
    s.pen.fg = Color::Rgb(1, 2, 3);
    s.pen.flags.insert(cell::AttrFlags::BOLD);
    s.pen.underline = Underline {
        style: UnderlineStyle::Curly,
        color: Color::Palette(9),
    };
    s.print('x');
    let cell = &s.row(0).cells()[0];
    assert_eq!(cell.attrs, s.pen);
    assert!(cell.attrs.flags.contains(cell::AttrFlags::BOLD));
}

#[test]
fn wide_char_uses_two_cells() {
    let mut s = Screen::new(3, 10);
    s.print('あ');
    s.print('b');
    let cells = s.row(0).cells();
    assert_eq!(cells[0].width, 2);
    assert_eq!(cells[0].text(), "あ");
    assert!(cells[1].is_wide_tail());
    assert_eq!(cells[2].text(), "b");
    assert_eq!(s.cursor().x, 3);
}

#[test]
fn overwriting_half_of_wide_char_clears_the_other_half() {
    let mut s = Screen::new(3, 10);
    s.print('あ');
    s.set_cursor(1, 0);
    s.print('x');
    let cells = s.row(0).cells();
    assert!(cells[0].is_empty());
    assert_eq!(cells[1].text(), "x");

    let mut s = Screen::new(3, 10);
    s.print('あ');
    s.set_cursor(0, 0);
    s.print('x');
    let cells = s.row(0).cells();
    assert_eq!(cells[0].text(), "x");
    assert!(cells[1].is_empty());
}

#[test]
fn combining_char_attaches_to_previous_cell() {
    let mut s = Screen::new(3, 10);
    s.print_str("e\u{0301}x");
    let cells = s.row(0).cells();
    assert_eq!(cells[0].text(), "e\u{0301}");
    assert_eq!(cells[0].width, 1);
    assert_eq!(cells[1].text(), "x");
    assert_eq!(s.cursor().x, 2);
}

#[test]
fn combining_char_after_wide_char_and_at_line_end() {
    let mut s = Screen::new(3, 4);
    s.print_str("あ\u{3099}");
    assert_eq!(s.row(0).cells()[0].text(), "あ\u{3099}");
    assert_eq!(s.cursor().x, 2);

    let mut s = Screen::new(3, 3);
    s.print_str("abc\u{0301}");
    assert_eq!(s.row(0).cells()[2].text(), "c\u{0301}");
    assert!(s.cursor().pending_wrap);
}

#[test]
fn emoji_variation_selector_and_zwj_widen_cell() {
    let mut s = Screen::new(3, 10);
    s.print_str("\u{2764}\u{FE0F}x");
    let cells = s.row(0).cells();
    assert_eq!(cells[0].width, 2);
    assert!(cells[1].is_wide_tail());
    assert_eq!(cells[2].text(), "x");

    let mut s = Screen::new(3, 10);
    s.print_str("\u{1F468}\u{200D}\u{1F469}x");
    let cells = s.row(0).cells();
    assert_eq!(cells[0].text(), "\u{1F468}\u{200D}\u{1F469}");
    assert_eq!(cells[2].text(), "x");
}

#[test]
fn wraps_at_end_of_line() {
    let mut s = Screen::new(3, 4);
    s.print_str("abcd");
    let c = s.cursor();
    assert_eq!((c.x, c.y, c.pending_wrap), (3, 0, true));
    assert!(!s.row(0).is_wrapped());
    s.print('e');
    assert_eq!(row_text(&s, 0), "abcd");
    assert_eq!(row_text(&s, 1), "e");
    assert!(s.row(0).is_wrapped());
    assert!(!s.row(1).is_wrapped());
    assert_eq!(s.cursor().y, 1);
}

#[test]
fn no_wraparound_overwrites_last_cell() {
    let mut s = Screen::new(3, 4);
    s.wraparound = false;
    s.print_str("abcdef");
    assert_eq!(row_text(&s, 0), "abcf");
    assert_eq!(s.cursor().y, 0);
    assert!(!s.row(0).is_wrapped());
}

#[test]
fn wide_char_that_does_not_fit_moves_to_next_line() {
    let mut s = Screen::new(3, 4);
    s.print_str("abc");
    s.print('あ');
    assert_eq!(row_text(&s, 0), "abc");
    assert!(s.row(0).is_wrapped());
    assert_eq!(s.row(1).cells()[0].text(), "あ");
    assert_eq!(s.cursor().x, 2);
    assert_eq!(s.cursor().y, 1);
}

#[test]
fn wide_char_that_does_not_fit_without_wraparound_backs_up() {
    let mut s = Screen::new(3, 4);
    s.wraparound = false;
    s.print_str("abc");
    s.print('あ');
    assert_eq!(row_text(&s, 0), "abあ");
    assert_eq!(s.cursor().y, 0);
}

#[test]
fn line_feed_at_bottom_scrolls_into_scrollback() {
    let mut s = Screen::new(3, 5);
    for (i, w) in ["one", "two", "three", "four"].iter().enumerate() {
        if i > 0 {
            s.index();
            s.carriage_return();
        }
        s.print_str(w);
    }
    assert_eq!(s.scrollback_len(), 1);
    assert_eq!(scrollback_text(&s, 0), "one");
    assert_eq!(row_text(&s, 0), "two");
    assert_eq!(row_text(&s, 2), "four");
}

#[test]
fn scrollback_is_limited() {
    let mut s = Screen::with_scrollback(2, 5, 3);
    for i in 0..10 {
        s.print_str(&i.to_string());
        s.index();
        s.carriage_return();
    }
    assert_eq!(s.scrollback_len(), 3);
    assert_eq!(scrollback_text(&s, 0), "6");
    assert_eq!(scrollback_text(&s, 2), "8");
}

#[test]
fn default_scrollback_is_5000() {
    let mut s = Screen::new(2, 3);
    for _ in 0..6000 {
        s.index();
    }
    assert_eq!(s.scrollback_len(), 5000);
}

#[test]
fn scroll_inside_region_does_not_touch_outside_or_scrollback() {
    let mut s = Screen::new(5, 5);
    for (y, w) in ["a", "b", "c", "d", "e"].iter().enumerate() {
        s.set_cursor(0, y);
        s.print_str(w);
    }
    assert!(s.set_scroll_region(1, 3));
    assert_eq!(s.scroll_region(), (1, 3));
    s.set_cursor(0, 3);
    s.index();
    assert_eq!(s.cursor().y, 3);
    assert_eq!(
        (0..5).map(|y| row_text(&s, y)).collect::<Vec<_>>(),
        ["a", "c", "d", "", "e"]
    );
    assert_eq!(s.scrollback_len(), 0);

    s.set_cursor(0, 1);
    s.reverse_index();
    assert_eq!(
        (0..5).map(|y| row_text(&s, y)).collect::<Vec<_>>(),
        ["a", "", "c", "d", "e"]
    );
}

#[test]
fn region_starting_at_top_sends_lines_to_scrollback() {
    let mut s = Screen::new(4, 5);
    for (y, w) in ["a", "b", "c", "d"].iter().enumerate() {
        s.set_cursor(0, y);
        s.print_str(w);
    }
    s.set_scroll_region(0, 2);
    s.scroll_up(1);
    assert_eq!(scrollback_text(&s, 0), "a");
    assert_eq!(
        (0..4).map(|y| row_text(&s, y)).collect::<Vec<_>>(),
        ["b", "c", "", "d"]
    );
}

#[test]
fn scroll_down_and_invalid_region() {
    let mut s = Screen::new(3, 5);
    for (y, w) in ["a", "b", "c"].iter().enumerate() {
        s.set_cursor(0, y);
        s.print_str(w);
    }
    s.scroll_down(1);
    assert_eq!(
        (0..3).map(|y| row_text(&s, y)).collect::<Vec<_>>(),
        ["", "a", "b"]
    );
    assert!(!s.set_scroll_region(2, 1));
    assert!(!s.set_scroll_region(0, 3));
    assert_eq!(s.scroll_region(), (0, 2));
}

#[test]
fn alt_screen_has_no_scrollback_and_restores_primary() {
    let mut s = Screen::new(2, 5);
    s.print_str("main");
    s.enter_alt_screen();
    assert!(s.is_alt_screen());
    assert_eq!(row_text(&s, 0), "");
    s.print_str("alt");
    for _ in 0..5 {
        s.index();
    }
    assert_eq!(s.scrollback_len(), 0);
    s.leave_alt_screen();
    assert!(!s.is_alt_screen());
    assert_eq!(row_text(&s, 0), "main");
    assert_eq!(s.cursor().x, 4);

    s.enter_alt_screen();
    assert_eq!(row_text(&s, 0), "");
}

#[test]
fn primary_scrollback_survives_alt_screen() {
    let mut s = Screen::new(2, 5);
    s.print_str("a");
    s.index();
    s.index();
    assert_eq!(s.scrollback_len(), 1);
    s.enter_alt_screen();
    assert_eq!(s.scrollback_len(), 0);
    s.leave_alt_screen();
    assert_eq!(s.scrollback_len(), 1);
}

#[test]
fn dirty_rows_track_changes() {
    let mut s = Screen::new(4, 5);
    assert_eq!(s.take_dirty_rows(), vec![0, 1, 2, 3]);
    assert!(s.take_dirty_rows().is_empty());
    s.set_cursor(0, 2);
    s.print('x');
    assert_eq!(s.take_dirty_rows(), vec![2]);
    s.set_scroll_region(1, 2);
    s.scroll_up(1);
    assert_eq!(s.take_dirty_rows(), vec![1, 2]);
    s.enter_alt_screen();
    assert_eq!(s.take_dirty_rows(), vec![0, 1, 2, 3]);
    s.leave_alt_screen();
    assert_eq!(s.take_dirty_rows(), vec![0, 1, 2, 3]);
}

#[test]
fn erase_uses_pen_background_and_fixes_wide_halves() {
    let mut s = Screen::new(2, 6);
    s.print_str("aあb");
    s.pen.bg = Color::Palette(4);
    s.erase_cells(0, 2, 6);
    let cells = s.row(0).cells();
    assert_eq!(cells[0].text(), "a");
    assert!(cells[1].is_empty());
    assert!(cells[2].is_empty());
    assert_eq!(cells[2].attrs.bg, Color::Palette(4));
    assert_eq!(cells[5].attrs.bg, Color::Palette(4));
    assert_eq!(cells[1].attrs.bg, Color::Default);
}

#[test]
fn tabs() {
    let mut s = Screen::new(2, 20);
    assert_eq!(s.tab_stops(), vec![8, 16]);
    s.tab();
    assert_eq!(s.cursor().x, 8);
    s.tab();
    s.tab();
    assert_eq!(s.cursor().x, 19);
    s.back_tab();
    assert_eq!(s.cursor().x, 16);
    s.set_cursor(4, 0);
    s.set_tab_stop();
    assert_eq!(s.tab_stops(), vec![4, 8, 16]);
    s.clear_all_tab_stops();
    assert!(s.tab_stops().is_empty());
}

#[test]
fn save_and_restore_cursor() {
    let mut s = Screen::new(3, 5);
    s.pen.fg = Color::Palette(2);
    s.set_cursor(3, 1);
    s.save_cursor();
    s.pen = Attrs::default();
    s.set_cursor(0, 0);
    s.restore_cursor();
    assert_eq!((s.cursor().x, s.cursor().y), (3, 1));
    assert_eq!(s.pen.fg, Color::Palette(2));
}

#[test]
fn cursor_appearance() {
    let mut s = Screen::new(3, 5);
    s.cursor_shape = CursorShape::Bar;
    s.cursor_blinking = true;
    let c = s.cursor();
    assert_eq!(c.shape, CursorShape::Bar);
    assert!(c.blinking && c.visible);
}

#[test]
fn resize_truncates_on_alt_screen() {
    let mut s = Screen::new(3, 6);
    s.enter_alt_screen();
    s.print_str("abcdef");
    s.resize(3, 3);
    assert_eq!(row_text(&s, 0), "abc");
    assert_eq!(s.cols(), 3);
    assert_eq!(s.cursor().x, 2);
    s.resize(3, 6);
    assert_eq!(row_text(&s, 0), "abc");
    assert_eq!(s.row(0).cells().len(), 6);
}

#[test]
fn resize_cutting_wide_char_on_alt_screen_clears_it() {
    let mut s = Screen::new(2, 4);
    s.enter_alt_screen();
    s.print_str("abあ");
    s.resize(2, 3);
    assert_eq!(row_text(&s, 0), "ab");
}

#[test]
fn resize_rows() {
    let mut s = Screen::new(4, 5);
    for (y, w) in ["a", "b", "c", "d"].iter().enumerate() {
        s.set_cursor(0, y);
        s.print_str(w);
    }
    s.set_cursor(0, 1);
    s.resize(2, 5);
    assert_eq!(row_text(&s, 0), "a");
    assert_eq!(row_text(&s, 1), "b");
    assert_eq!(s.scrollback_len(), 0);

    s.set_cursor(0, 1);
    s.resize(1, 5);
    assert_eq!(s.scrollback_len(), 1);
    assert_eq!(scrollback_text(&s, 0), "a");
    assert_eq!(row_text(&s, 0), "b");
    assert_eq!(s.cursor().y, 0);

    s.resize(3, 5);
    assert_eq!(s.rows(), 3);
    assert_eq!(s.scroll_region(), (0, 2));
}
