use std::fmt::Write;

use anyhow::Result;
use unicode_width::UnicodeWidthChar;

use crate::{
    jump::{KeyPosition, OverlayCell, overlay_cells_for_char_indices},
    tmux::{PaneState, TmuxBackend},
};

const CLEAR_SEQ: &str = "\u{1b}[2J";
const HOME_SEQ: &str = "\u{1b}[H";
/// Reset the scroll region (DECSTBM) to the full screen. A TUI (vim splits,
/// anything with a fixed header/footer) narrows the scroll region with
/// `ESC[top;bottom r`; `ESC[2J` does *not* clear it. Without this reset, the
/// line feeds we emit while painting the overlay or restoring the screen scroll
/// *within* the stale region and shift every row — the "drift" that only shows
/// up inside TUIs. Emitting it before any multi-line write makes line feeds
/// behave normally again.
const RESET_SCROLL_REGION: &str = "\u{1b}[r";
/// Reset the scroll region, clear, and home — the standard prelude before
/// repainting a whole screen's worth of lines.
const RESET_CLEAR_HOME_SEQ: &str = concat!("\u{1b}[r", "\u{1b}[2J", "\u{1b}[H");
const RESET_COLORS: &str = "\u{1b}[0m";
const ENTER_ALTERNATE_HOME_SEQ: &str = concat!("\u{1b}[?1049h", "\u{1b}[r", "\u{1b}[H");
const RESTORE_NORMAL_SCREEN: &str = "\u{1b}[?1049l";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayTheme {
    Classic,
    Contrast,
    Soft,
}

impl OverlayTheme {
    pub fn from_env(value: &str) -> Self {
        match value {
            "contrast" | "high_contrast" | "high-contrast" => Self::Contrast,
            "soft" | "muted" => Self::Soft,
            _ => Self::Classic,
        }
    }

    pub fn defaults(self) -> OverlayStyle {
        match self {
            Self::Classic => OverlayStyle {
                background: String::from("\u{1b}[0m\u{1b}[32m"),
                foreground: String::from("\u{1b}[1m\u{1b}[31m"),
                label_style: String::from("\u{1b}[1m"),
                key_position: KeyPosition::Left,
            },
            Self::Contrast => OverlayStyle {
                background: String::from("\u{1b}[0m\u{1b}[37m"),
                foreground: String::from("\u{1b}[1m\u{1b}[30m"),
                label_style: String::from("\u{1b}[1m\u{1b}[7m"),
                key_position: KeyPosition::Left,
            },
            Self::Soft => OverlayStyle {
                background: String::from("\u{1b}[0m\u{1b}[2m\u{1b}[36m"),
                foreground: String::from("\u{1b}[1m\u{1b}[33m"),
                label_style: String::from("\u{1b}[1m"),
                key_position: KeyPosition::Left,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayStyle {
    pub background: String,
    pub foreground: String,
    pub label_style: String,
    pub key_position: KeyPosition,
}

pub fn decode_tmux_color(value: &str) -> String {
    let decoded = value.replace(r#"\\e"#, "\u{1b}").replace(r#"\e"#, "\u{1b}");
    retain_sgr_sequences(&decoded)
}

fn retain_sgr_sequences(value: &str) -> String {
    let mut retained = String::new();
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' || chars.next_if_eq(&'[').is_none() {
            continue;
        }

        let mut params = String::new();
        while let Some(&param) = chars.peek() {
            if param == 'm' {
                chars.next();
                retained.push('\u{1b}');
                retained.push('[');
                retained.push_str(&params);
                retained.push('m');
                break;
            }

            if param.is_ascii_digit() || param == ';' || param == ':' {
                params.push(param);
                chars.next();
            } else {
                break;
            }
        }
    }

    retained
}

pub fn with_recovered_screen<B, T, F>(backend: &B, pane: &PaneState, action: F) -> Result<T>
where
    B: TmuxBackend,
    F: FnOnce() -> Result<T>,
{
    if pane.alternate_on {
        with_alternate_screen_restore(backend, pane, action)
    } else {
        with_normal_screen_restore(backend, pane, action)
    }
}

fn with_normal_screen_restore<B, T, F>(backend: &B, pane: &PaneState, action: F) -> Result<T>
where
    B: TmuxBackend,
    F: FnOnce() -> Result<T>,
{
    backend.write_to_tty(pane, ENTER_ALTERNATE_HOME_SEQ)?;
    let action_result = action();
    let restore_result = backend.write_to_tty(pane, RESTORE_NORMAL_SCREEN);
    action_result.and_then(|v| restore_result.map(|()| v))
}

fn with_alternate_screen_restore<B, T, F>(backend: &B, pane: &PaneState, action: F) -> Result<T>
where
    B: TmuxBackend,
    F: FnOnce() -> Result<T>,
{
    let saved_screen = backend.capture_pane_with_escapes(&pane.pane_id)?;
    backend.write_to_tty(pane, RESET_CLEAR_HOME_SEQ)?;
    let action_result = action();

    let restore = alternate_screen_restore_sequence(&saved_screen, pane.cursor_y, pane.cursor_x);
    let restore_result = backend.write_to_tty(pane, &restore);
    action_result.and_then(|v| restore_result.map(|()| v))
}

/// Build the byte sequence that repaints `saved_screen` (captured with `-e`, so
/// it still carries its SGR colors) and parks the cursor back where the TUI left
/// it. Resets the scroll region first: `saved_screen` is written line by line
/// with `\n\r`, and a narrowed scroll region would scroll those line feeds
/// instead of advancing rows, shifting the whole grid — see
/// [`RESET_SCROLL_REGION`].
pub fn alternate_screen_restore_sequence(
    saved_screen: &str,
    cursor_y: usize,
    cursor_x: usize,
) -> String {
    let line_break_count = saved_screen
        .as_bytes()
        .iter()
        .filter(|&&byte| byte == b'\n')
        .count();
    let mut restore = String::with_capacity(
        saved_screen
            .len()
            .saturating_add(line_break_count)
            .saturating_add(64),
    );
    restore.push_str(RESET_COLORS);
    restore.push_str(RESET_SCROLL_REGION);
    restore.push_str(CLEAR_SEQ);
    // Home explicitly rather than assuming the caller left the cursor there: the
    // saved rows below are written sequentially, so they must start at row 1.
    restore.push_str(HOME_SEQ);
    for ch in saved_screen.chars() {
        restore.push(ch);
        if ch == '\n' {
            restore.push('\r');
        }
    }
    assert!(
        write!(
            restore,
            "\u{1b}[{};{}H",
            cursor_y.saturating_add(1),
            cursor_x.saturating_add(1)
        )
        .is_ok(),
        "writing to a String cannot fail"
    );
    restore.push_str(RESET_COLORS);
    restore
}

pub fn draw_overlay<B: TmuxBackend + ?Sized>(
    backend: &B,
    pane: &PaneState,
    screen: &str,
    positions: &[usize],
    labels: &[String],
    style: &OverlayStyle,
) -> Result<()> {
    backend.write_to_tty(pane, &render_overlay(screen, positions, labels, style))
}

pub(crate) fn draw_overlay_cells<B: TmuxBackend + ?Sized>(
    backend: &B,
    pane: &PaneState,
    screen: &str,
    overlay_cells: &[OverlayCell],
    labels: &[String],
    style: &OverlayStyle,
) -> Result<()> {
    backend.write_to_tty(
        pane,
        &render_overlay_cells(screen, overlay_cells, labels, style),
    )
}

fn render_overlay(
    screen: &str,
    positions: &[usize],
    labels: &[String],
    style: &OverlayStyle,
) -> String {
    let overlay_cells = overlay_cells_for_char_indices(screen, positions);
    render_overlay_cells(screen, &overlay_cells, labels, style)
}

fn render_overlay_cells(
    screen: &str,
    overlay_cells: &[OverlayCell],
    labels: &[String],
    style: &OverlayStyle,
) -> String {
    let line_break_count = screen
        .as_bytes()
        .iter()
        .filter(|&&byte| byte == b'\n')
        .count();
    let per_label_capacity = style
        .foreground
        .len()
        .saturating_add(style.label_style.len())
        .saturating_add(RESET_COLORS.len())
        .saturating_add(32);
    let capacity = RESET_CLEAR_HOME_SEQ
        .len()
        .saturating_add(style.background.len())
        .saturating_add(screen.len())
        .saturating_add(line_break_count)
        .saturating_add(labels.len().saturating_mul(per_label_capacity))
        .saturating_add(
            labels
                .iter()
                .fold(0usize, |total, label| total.saturating_add(label.len())),
        )
        .saturating_add(HOME_SEQ.len());
    let mut rendered = String::with_capacity(capacity);
    rendered.push_str(RESET_CLEAR_HOME_SEQ);
    rendered.push_str(&style.background);
    for ch in screen.chars() {
        rendered.push(ch);
        if ch == '\n' {
            rendered.push('\r');
        }
    }

    for (cell, label) in overlay_cells.iter().zip(labels.iter()) {
        let display_position = cell.anchor;
        let label_width = label.chars().fold(0usize, |width, ch| {
            width.saturating_add(UnicodeWidthChar::width(ch).unwrap_or(1))
        });
        let start_column = match style.key_position {
            KeyPosition::Left => display_position.column,
            KeyPosition::Right => {
                // Whole-cell width, not the width of the char alone: for a
                // cluster like ☝️ the wideness comes from the variation
                // selector, and the label must clear the entire cell.
                display_position.column.saturating_add(cell.width)
            }
            KeyPosition::OffLeft => display_position.column.saturating_sub(label_width),
        };

        assert!(
            write!(
                rendered,
                "\u{1b}[{};{}H{}{}{}{}",
                display_position.row.saturating_add(1),
                start_column.saturating_add(1),
                style.foreground,
                style.label_style,
                label,
                RESET_COLORS,
            )
            .is_ok(),
            "writing to a String cannot fail"
        );
    }

    rendered.push_str(HOME_SEQ);
    rendered
}

#[cfg(test)]
mod tests {
    use std::{hint::black_box, time::Instant};

    use super::{
        OverlayStyle, alternate_screen_restore_sequence, decode_tmux_color, render_overlay,
        render_overlay_cells,
    };
    use crate::jump::{KeyPosition, overlay_cells_for_char_indices};

    fn style(key_position: KeyPosition) -> OverlayStyle {
        OverlayStyle {
            background: String::from("BG"),
            foreground: String::from("FG"),
            label_style: String::from("LB"),
            key_position,
        }
    }

    #[test]
    fn render_overlay_starts_from_a_clean_home_position() {
        let positions = [0usize];
        let labels = [String::from("j")];
        let rendered = render_overlay(
            "ab\ncd",
            &positions[..],
            &labels[..],
            &style(KeyPosition::Left),
        );

        // The scroll region is reset (`ESC[r`) before the clear+home so the
        // `\n\r` line feeds below never scroll inside a TUI's stale region.
        assert!(rendered.starts_with("\u{1b}[r\u{1b}[2J\u{1b}[HBGab\n\rcd"));
        assert!(rendered.ends_with("\u{1b}[H"));
    }

    #[test]
    fn render_overlay_resets_scroll_region_before_any_line_feed() {
        // Regression guard for the TUI drift bug: the scroll-region reset must
        // come before the first `\n\r`, otherwise painting the overlay scrolls
        // the pane and every jump target lands a row (or more) off.
        let positions = [0usize];
        let labels = [String::from("j")];
        let rendered = render_overlay(
            "one\ntwo\nthree",
            &positions[..],
            &labels[..],
            &style(KeyPosition::Left),
        );

        let reset = rendered.find("\u{1b}[r");
        let first_line_feed = rendered.find("\n\r");
        assert!(
            matches!((reset, first_line_feed), (Some(r), Some(lf)) if r < lf),
            "scroll-region reset must precede the first line feed"
        );
    }

    #[test]
    fn alternate_screen_restore_resets_scroll_region_before_repaint() {
        // The restore repaints the saved TUI screen with `\n\r` per row, so it
        // must also neutralize a narrowed scroll region first — the same drift
        // source as the overlay, on the way back out.
        let restore = alternate_screen_restore_sequence("row-a\nrow-b\nrow-c", 2, 4);

        let reset = restore.find("\u{1b}[r");
        let first_line_feed = restore.find("\n\r");
        assert!(
            matches!((reset, first_line_feed), (Some(r), Some(lf)) if r < lf),
            "scroll-region reset must precede the first repainted line feed"
        );
        // Cursor is parked back at the TUI's 1-based position (row 3, col 5).
        assert!(restore.contains("\u{1b}[3;5H"));
    }

    #[test]
    fn alternate_screen_restore_saturates_untrusted_cursor_coordinates() {
        let restore = alternate_screen_restore_sequence("screen", usize::MAX, usize::MAX);

        assert!(restore.contains(&format!("\u{1b}[{};{}H", usize::MAX, usize::MAX)));
    }

    #[test]
    fn render_overlay_honors_label_position_modes() {
        let positions = [2usize];
        let labels = [String::from("jk")];
        let left = render_overlay(
            "alpha",
            &positions[..],
            &labels[..],
            &style(KeyPosition::Left),
        );
        let off_left = render_overlay(
            "alpha",
            &positions[..],
            &labels[..],
            &style(KeyPosition::OffLeft),
        );
        let right = render_overlay(
            "alpha",
            &positions[..],
            &labels[..],
            &style(KeyPosition::Right),
        );

        assert!(left.contains("\u{1b}[1;3HFGLBjk\u{1b}[0m"));
        assert!(off_left.contains("\u{1b}[1;1HFGLBjk\u{1b}[0m"));
        assert!(right.contains("\u{1b}[1;4HFGLBjk\u{1b}[0m"));
    }

    #[test]
    fn render_overlay_off_left_saturates_at_the_first_column() {
        // A target in column 0 with a two-char label must not underflow: the
        // label clamps to column 1 (1-based) instead of wrapping around.
        let positions = [0usize];
        let labels = [String::from("jk")];
        let rendered = render_overlay(
            "alpha",
            &positions[..],
            &labels[..],
            &style(KeyPosition::OffLeft),
        );

        assert!(rendered.contains("\u{1b}[1;1HFGLBjk\u{1b}[0m"));
    }

    #[test]
    fn render_overlay_places_label_at_display_column_after_wide_characters() {
        let positions = [4usize];
        let labels = [String::from("j")];
        let rendered = render_overlay(
            "あいう x",
            &positions[..],
            &labels[..],
            &style(KeyPosition::Left),
        );

        assert!(rendered.contains("\u{1b}[1;8HFGLBj\u{1b}[0m"));
    }

    #[test]
    fn render_overlay_places_label_to_the_right_of_wide_characters() {
        let positions = [4usize];
        let labels = [String::from("j")];
        let rendered = render_overlay(
            "あいう x",
            &positions[..],
            &labels[..],
            &style(KeyPosition::Right),
        );

        assert!(rendered.contains("\u{1b}[1;9HFGLBj\u{1b}[0m"));
    }

    #[test]
    fn render_overlay_right_position_clears_multi_char_clusters() {
        // ☝ + VS16 is one grid cell of width 2; a right-positioned label must
        // land after the whole cell (1-based column 3), not after the base
        // char's own width.
        let positions = [0usize];
        let labels = [String::from("j")];
        let rendered = render_overlay(
            "☝\u{fe0f} z",
            &positions[..],
            &labels[..],
            &style(KeyPosition::Right),
        );

        assert!(rendered.contains("\u{1b}[1;3HFGLBj\u{1b}[0m"));
    }

    #[test]
    fn cached_overlay_cells_preserve_mixed_unicode_rendering() {
        let screen = "あ☝\u{fe0f} e\u{301}\nalpha";
        let positions = [0usize, 1, 2, 4, 7];
        let labels = ["j", "f", "h", "g", "k"].map(String::from);
        let overlay_style = style(KeyPosition::Right);
        let cells = overlay_cells_for_char_indices(screen, &positions);

        assert_eq!(
            render_overlay_cells(screen, &cells, &labels, &overlay_style),
            render_overlay(screen, &positions, &labels, &overlay_style)
        );
    }

    #[test]
    fn decode_tmux_color_supports_single_and_double_escaped_values() {
        assert_eq!(decode_tmux_color(r#"\e[32m"#), "\u{1b}[32m");
        assert_eq!(decode_tmux_color(r#"\\e[32m"#), "\u{1b}[32m");
    }

    #[test]
    fn decode_tmux_color_supports_extended_sgr_values() {
        assert_eq!(
            decode_tmux_color(r#"\e[38;2;1;2;3m\e[48:2:4:5:6m"#),
            "\u{1b}[38;2;1;2;3m\u{1b}[48:2:4:5:6m"
        );
    }

    #[test]
    fn decode_tmux_color_discards_non_sgr_sequences() {
        assert_eq!(
            decode_tmux_color(r#"\e]52;c;SGVsbG8=\a\e[31mplain\e[2J"#),
            "\u{1b}[31m"
        );
    }

    #[test]
    #[ignore = "manual release-mode performance measurement"]
    fn benchmark_render_overlay_mixed_unicode_screen() {
        const SAMPLE_COUNT: usize = 7;
        const ITERATIONS_PER_SAMPLE: usize = 400;

        let screen = "alpha あいう ☝\u{fe0f} e\u{301} omega\n".repeat(160);
        let positions = (0..300).map(|index| index * 2).collect::<Vec<_>>();
        let labels = (0..positions.len())
            .map(|index| format!("{:02}", index % 81))
            .collect::<Vec<_>>();
        let overlay_style = style(KeyPosition::Right);
        let mut samples = Vec::with_capacity(SAMPLE_COUNT);

        for _ in 0..SAMPLE_COUNT {
            let started = Instant::now();
            for _ in 0..ITERATIONS_PER_SAMPLE {
                black_box(render_overlay(
                    black_box(&screen),
                    black_box(&positions),
                    black_box(&labels),
                    black_box(&overlay_style),
                ));
            }
            samples.push(started.elapsed());
        }

        samples.sort_unstable();
        eprintln!(
            "render_overlay: median {:?} for {ITERATIONS_PER_SAMPLE} iterations ({SAMPLE_COUNT} samples)",
            samples[SAMPLE_COUNT / 2]
        );
    }
}
