use std::{fs::OpenOptions, io::Write};

use anyhow::{Context, Result};

use crate::{
    jump::KeyPosition,
    tmux::{PaneState, capture_pane_with_escapes},
};

const CLEAR_SEQ: &str = "\u{1b}[2J";
const HOME_SEQ: &str = "\u{1b}[H";
const RESET_COLORS: &str = "\u{1b}[0m";
const ENTER_ALTERNATE_SCREEN: &str = "\u{1b}[?1049h";
const RESTORE_NORMAL_SCREEN: &str = "\u{1b}[?1049l";

#[derive(Debug, Clone)]
pub struct OverlayStyle {
    pub background: String,
    pub foreground: String,
    pub key_position: KeyPosition,
}

pub fn decode_tmux_color(value: &str) -> String {
    value.replace(r#"\\e"#, "\u{1b}").replace(r#"\e"#, "\u{1b}")
}

pub fn with_recovered_screen<T, F>(pane: &PaneState, action: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    if pane.alternate_on {
        with_alternate_screen_restore(pane, action)
    } else {
        with_normal_screen_restore(pane, action)
    }
}

fn with_normal_screen_restore<T, F>(pane: &PaneState, action: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    append_to_tty(
        &pane.tty_path,
        &(ENTER_ALTERNATE_SCREEN.to_owned() + HOME_SEQ),
    )?;
    let result = action();
    append_to_tty(&pane.tty_path, RESTORE_NORMAL_SCREEN)?;
    result
}

fn with_alternate_screen_restore<T, F>(pane: &PaneState, action: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    let saved_screen = capture_pane_with_escapes(&pane.pane_id)?;
    append_to_tty(&pane.tty_path, &(CLEAR_SEQ.to_owned() + HOME_SEQ))?;
    let result = action();

    let mut restore = String::from(RESET_COLORS);
    restore.push_str(CLEAR_SEQ);
    restore.push_str(&saved_screen.replace('\n', "\n\r"));
    restore.push_str(&format!(
        "\u{1b}[{};{}H",
        pane.cursor_y + 1,
        pane.cursor_x + 1
    ));
    restore.push_str(RESET_COLORS);
    append_to_tty(&pane.tty_path, &restore)?;
    result
}

pub fn draw_overlay(
    pane: &PaneState,
    screen: &str,
    positions: &[usize],
    labels: &[String],
    style: &OverlayStyle,
) -> Result<()> {
    append_to_tty(
        &pane.tty_path,
        &render_overlay(screen, positions, labels, style),
    )
}

fn render_overlay(
    screen: &str,
    positions: &[usize],
    labels: &[String],
    style: &OverlayStyle,
) -> String {
    let mut rendered = String::new();
    rendered.push_str(CLEAR_SEQ);
    rendered.push_str(HOME_SEQ);
    rendered.push_str(&style.background);
    rendered.push_str(&screen.replace('\n', "\n\r"));

    for (position, label) in positions.iter().zip(labels.iter()) {
        let (line, column) = line_col_for_char_index(screen, *position);
        let label_width = label.chars().count();
        let start_column = match style.key_position {
            KeyPosition::Left => column,
            KeyPosition::OffLeft => column.saturating_sub(label_width),
        };

        rendered.push_str(&format!(
            "\u{1b}[{};{}H{}{}",
            line + 1,
            start_column + 1,
            style.foreground,
            label,
        ));
    }

    rendered.push_str(HOME_SEQ);
    rendered
}

fn line_col_for_char_index(screen: &str, target_index: usize) -> (usize, usize) {
    let mut line = 0usize;
    let mut column = 0usize;

    for (index, ch) in screen.chars().enumerate() {
        if index == target_index {
            return (line, column);
        }

        if ch == '\n' {
            line += 1;
            column = 0;
        } else {
            column += 1;
        }
    }

    (line, column)
}

fn append_to_tty(path: &str, content: &str) -> Result<()> {
    let mut tty = OpenOptions::new()
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open tty path {path}"))?;
    tty.write_all(content.as_bytes())
        .with_context(|| format!("failed to write overlay to tty {path}"))?;
    tty.flush()
        .with_context(|| format!("failed to flush tty {path}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{OverlayStyle, decode_tmux_color, render_overlay};
    use crate::jump::KeyPosition;

    fn style(key_position: KeyPosition) -> OverlayStyle {
        OverlayStyle {
            background: String::from("BG"),
            foreground: String::from("FG"),
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

        assert!(left.contains("\u{1b}[1;3HFGjk"));
        assert!(off_left.contains("\u{1b}[1;1HFGjk"));
    }

    #[test]
    fn decode_tmux_color_supports_single_and_double_escaped_values() {
        assert_eq!(decode_tmux_color(r#"\e[32m"#), "\u{1b}[32m");
        assert_eq!(decode_tmux_color(r#"\\e[32m"#), "\u{1b}[32m");
    }
}
