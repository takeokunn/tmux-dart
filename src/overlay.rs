use anyhow::Result;
use unicode_width::UnicodeWidthChar;

use crate::{
    jump::{KeyPosition, overlay_anchor_for_char_index},
    tmux::{PaneState, TmuxBackend},
};

const CLEAR_SEQ: &str = "\u{1b}[2J";
const HOME_SEQ: &str = "\u{1b}[H";
const CLEAR_HOME_SEQ: &str = concat!("\u{1b}[2J", "\u{1b}[H");
const RESET_COLORS: &str = "\u{1b}[0m";
const ENTER_ALTERNATE_HOME_SEQ: &str = concat!("\u{1b}[?1049h", "\u{1b}[H");
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
    backend.write_to_tty(pane, CLEAR_HOME_SEQ)?;
    let action_result = action();

    let mut restore = String::from(RESET_COLORS);
    restore.push_str(CLEAR_SEQ);
    restore.push_str(&saved_screen.replace('\n', "\n\r"));
    restore.push_str(&format!(
        "\u{1b}[{};{}H",
        pane.cursor_y + 1,
        pane.cursor_x + 1
    ));
    restore.push_str(RESET_COLORS);
    let restore_result = backend.write_to_tty(pane, &restore);
    action_result.and_then(|v| restore_result.map(|()| v))
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

fn render_overlay(
    screen: &str,
    positions: &[usize],
    labels: &[String],
    style: &OverlayStyle,
) -> String {
    let mut rendered = String::new();
    rendered.push_str(CLEAR_HOME_SEQ);
    rendered.push_str(&style.background);
    rendered.push_str(&screen.replace('\n', "\n\r"));

    for (position, label) in positions.iter().zip(labels.iter()) {
        let display_position = overlay_anchor_for_char_index(screen, *position);
        let label_width: usize = label
            .chars()
            .map(|c| UnicodeWidthChar::width(c).unwrap_or(1))
            .sum();
        let start_column = match style.key_position {
            KeyPosition::Left => display_position.column,
            KeyPosition::Right => {
                let target_width = screen
                    .chars()
                    .nth(*position)
                    .and_then(UnicodeWidthChar::width)
                    .unwrap_or(1);
                display_position.column + target_width
            }
            KeyPosition::OffLeft => display_position.column.saturating_sub(label_width),
        };

        rendered.push_str(&format!(
            "\u{1b}[{};{}H{}{}{}{}",
            display_position.row + 1,
            start_column + 1,
            style.foreground,
            style.label_style,
            label,
            RESET_COLORS,
        ));
    }

    rendered.push_str(HOME_SEQ);
    rendered
}

#[cfg(test)]
mod tests {
    use super::{OverlayStyle, decode_tmux_color, render_overlay};
    use crate::jump::KeyPosition;

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

        assert!(rendered.starts_with("\u{1b}[2J\u{1b}[HBGab\n\rcd"));
        assert!(rendered.ends_with("\u{1b}[H"));
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
}
