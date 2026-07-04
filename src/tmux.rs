use std::{
    fs,
    io::ErrorKind,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use tempfile::NamedTempFile;

use crate::jump::JumpTarget;

#[derive(Debug, Clone)]
pub struct PaneState {
    pub pane_id: String,
    pub tty_path: String,
    pub in_copy_mode: bool,
    pub cursor_y: usize,
    pub cursor_x: usize,
    pub alternate_on: bool,
    pub scroll_position: usize,
    pub pane_height: usize,
}

pub trait TmuxBackend {
    fn current_pane_id(&self) -> Result<String>;
    fn pane_state(&self, pane_id: &str) -> Result<PaneState>;
    fn cancel_copy_mode(&self, pane: &PaneState) -> Result<()>;
    fn capture_visible_pane(&self, pane: &PaneState) -> Result<String>;
    fn capture_pane_with_escapes(&self, pane_id: &str) -> Result<String>;
    fn write_to_tty(&self, pane: &PaneState, content: &str) -> Result<()>;
    fn prompt_for_label_char(&self, prompt: &str) -> Result<Option<char>>;
    fn display_message(&self, message: &str) -> Result<()>;
    fn jump_to_position(&self, pane: &PaneState, jump_to: JumpTarget) -> Result<()>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RealTmux;

impl TmuxBackend for RealTmux {
    fn current_pane_id(&self) -> Result<String> {
        current_pane_id()
    }

    fn pane_state(&self, pane_id: &str) -> Result<PaneState> {
        pane_state(pane_id)
    }

    fn cancel_copy_mode(&self, pane: &PaneState) -> Result<()> {
        cancel_copy_mode(pane)
    }

    fn capture_visible_pane(&self, pane: &PaneState) -> Result<String> {
        capture_visible_pane(pane)
    }

    fn capture_pane_with_escapes(&self, pane_id: &str) -> Result<String> {
        capture_pane_with_escapes(pane_id)
    }

    fn write_to_tty(&self, pane: &PaneState, content: &str) -> Result<()> {
        write_to_tty(&pane.tty_path, content)
    }

    fn prompt_for_label_char(&self, prompt: &str) -> Result<Option<char>> {
        prompt_for_label_char(prompt)
    }

    fn display_message(&self, message: &str) -> Result<()> {
        display_message(message)
    }

    fn jump_to_position(&self, pane: &PaneState, jump_to: JumpTarget) -> Result<()> {
        jump_to_position(pane, jump_to)
    }
}

pub fn current_pane_id() -> Result<String> {
    tmux_output(["display-message", "-p", "#{pane_id}"])
}

pub fn display_message(message: &str) -> Result<()> {
    tmux_success(["display-message", message])
}

pub fn pane_state(pane_id: &str) -> Result<PaneState> {
    let format = "#{pane_id};#{pane_tty};#{pane_in_mode};#{cursor_y};#{cursor_x};#{alternate_on};#{scroll_position};#{pane_height}";
    let output = tmux_output(["display-message", "-p", "-t", pane_id, "-F", format])?;
    let parts: Vec<&str> = output.split(';').collect();
    if parts.len() != 8 {
        bail!("unexpected tmux pane format output: {output}");
    }

    Ok(PaneState {
        pane_id: parts[0].to_owned(),
        tty_path: parts[1].to_owned(),
        in_copy_mode: parts[2] == "1",
        cursor_y: parse_usize_or_zero(parts[3], "cursor_y")?,
        cursor_x: parse_usize_or_zero(parts[4], "cursor_x")?,
        alternate_on: parts[5] == "1",
        scroll_position: parse_usize_or_zero(parts[6], "scroll_position")?,
        pane_height: parse_usize_or_zero(parts[7], "pane_height")?,
    })
}

fn parse_usize_or_zero(value: &str, field: &str) -> Result<usize> {
    if value.is_empty() {
        return Ok(0);
    }

    value.parse().with_context(|| format!("invalid {field}"))
}

pub fn capture_visible_pane(pane: &PaneState) -> Result<String> {
    let start = -(pane.scroll_position as isize);
    let end = start + pane.pane_height as isize - 1;
    tmux_capture_output([
        "capture-pane",
        "-p",
        "-t",
        &pane.pane_id,
        "-S",
        &start.to_string(),
        "-E",
        &end.to_string(),
    ])
    .map(|screen| screen.replace('\u{fe0e}', ""))
}

pub fn capture_pane_with_escapes(pane_id: &str) -> Result<String> {
    tmux_capture_output(["capture-pane", "-e", "-p", "-t", pane_id])
}

pub fn cancel_copy_mode(pane: &PaneState) -> Result<()> {
    if pane.in_copy_mode {
        tmux_success(["send-keys", "-X", "-t", &pane.pane_id, "cancel"])?;
    }
    Ok(())
}

pub fn read_initial_char_from_file(path: &str) -> Result<Option<char>> {
    wait_for_char_file(path, None, Duration::from_secs(10))
}

pub fn prompt_for_label_char(prompt: &str) -> Result<Option<char>> {
    let temp = NamedTempFile::new().context("failed to create temporary prompt file")?;
    let path = temp.path().to_string_lossy().into_owned();
    let quoted_path = shell_quote(&path);

    tmux_success([
        "command-prompt",
        "-1",
        "-p",
        prompt,
        &prompt_for_label_char_command(&quoted_path),
    ])?;
    let previous_activity = session_activity()?;

    wait_for_char_file(&path, Some(&previous_activity), Duration::from_secs(10))
}

fn wait_for_char_file(
    path: &str,
    previous_activity: Option<&str>,
    timeout: Duration,
) -> Result<Option<char>> {
    let started_at = Instant::now();
    let deadline = started_at + timeout;
    let activity_change_grace_period = Duration::from_millis(250);

    loop {
        if let Some(ch) = read_single_char_from_file(path)? {
            return Ok(Some(ch));
        }

        if started_at.elapsed() >= activity_change_grace_period
            && let Some(previous_activity) = previous_activity
            && session_activity()? != previous_activity
        {
            return Ok(None);
        }

        if Instant::now() >= deadline {
            return Ok(None);
        }

        thread::sleep(Duration::from_millis(50));
    }
}

fn read_single_char_from_file(path: &str) -> Result<Option<char>> {
    match fs::read_to_string(path) {
        Ok(content) => parse_single_char_content(&content),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read prompt file {path}")),
    }
}

fn parse_single_char_content(content: &str) -> Result<Option<char>> {
    let content = strip_optional_line_ending(content);
    if content.is_empty() {
        return Ok(None);
    }

    let mut chars = content.chars();
    let Some(first) = chars.next() else {
        return Ok(None);
    };
    ensure!(
        chars.next().is_none(),
        "prompt file must contain exactly one character"
    );
    Ok(Some(first))
}

fn strip_optional_line_ending(content: &str) -> &str {
    if let Some(stripped) = content.strip_suffix("\r\n") {
        stripped
    } else if let Some(stripped) = content.strip_suffix('\n') {
        stripped
    } else {
        content
    }
}

pub fn jump_to_position(pane: &PaneState, jump_to: JumpTarget) -> Result<()> {
    for command in copy_mode_jump_commands(pane, jump_to) {
        send_copy_mode_jump_command(pane, command)?;
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyModeJumpCommand {
    CopyMode,
    StartOfLine,
    TopLine,
    CursorUp(usize),
    CursorDown(usize),
    CursorRight(usize),
}

fn copy_mode_jump_commands(pane: &PaneState, jump_to: JumpTarget) -> Vec<CopyModeJumpCommand> {
    let mut commands = vec![
        CopyModeJumpCommand::CopyMode,
        CopyModeJumpCommand::StartOfLine,
        CopyModeJumpCommand::TopLine,
    ];

    if pane.scroll_position > 0 {
        commands.push(CopyModeJumpCommand::CursorUp(pane.scroll_position));
    }
    if jump_to.row > 0 {
        commands.push(CopyModeJumpCommand::CursorDown(jump_to.row));
    }
    if jump_to.column > 0 {
        commands.push(CopyModeJumpCommand::CursorRight(jump_to.column));
    }

    commands
}

fn send_copy_mode_jump_command(pane: &PaneState, command: CopyModeJumpCommand) -> Result<()> {
    match command {
        CopyModeJumpCommand::CopyMode => tmux_success(["copy-mode", "-t", &pane.pane_id]),
        CopyModeJumpCommand::StartOfLine => {
            tmux_success(["send-keys", "-X", "-t", &pane.pane_id, "start-of-line"])
        }
        CopyModeJumpCommand::TopLine => {
            tmux_success(["send-keys", "-X", "-t", &pane.pane_id, "top-line"])
        }
        CopyModeJumpCommand::CursorUp(count) => {
            let count = count.to_string();
            tmux_success([
                "send-keys",
                "-X",
                "-t",
                &pane.pane_id,
                "-N",
                &count,
                "cursor-up",
            ])
        }
        CopyModeJumpCommand::CursorDown(count) => {
            let count = count.to_string();
            tmux_success([
                "send-keys",
                "-X",
                "-t",
                &pane.pane_id,
                "-N",
                &count,
                "cursor-down",
            ])
        }
        CopyModeJumpCommand::CursorRight(count) => {
            let count = count.to_string();
            tmux_success([
                "send-keys",
                "-X",
                "-t",
                &pane.pane_id,
                "-N",
                &count,
                "cursor-right",
            ])
        }
    }
}

fn session_activity() -> Result<String> {
    tmux_output(["display-message", "-p", "#{session_activity}"])
}

fn tmux_stdout<const N: usize>(args: [&str; N]) -> Result<Vec<u8>> {
    let output = Command::new("tmux")
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn tmux with args {:?}", args))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(anyhow!("tmux command failed: {:?}: {stderr}", args));
    }

    Ok(output.stdout)
}

fn tmux_output<const N: usize>(args: [&str; N]) -> Result<String> {
    Ok(String::from_utf8_lossy(&tmux_stdout(args)?)
        .trim_end_matches('\n')
        .to_owned())
}

fn tmux_capture_output<const N: usize>(args: [&str; N]) -> Result<String> {
    Ok(trim_single_trailing_newline(&tmux_stdout(args)?))
}

fn trim_single_trailing_newline(output: &[u8]) -> String {
    let mut output = String::from_utf8_lossy(output).into_owned();
    if output.ends_with('\n') {
        output.pop();
    }
    output
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r#"'\''"#))
}

pub fn write_to_tty(path: &str, content: &str) -> Result<()> {
    use std::{fs::OpenOptions, io::Write};

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

fn tmux_success<const N: usize>(args: [&str; N]) -> Result<()> {
    tmux_output(args).map(|_| ())
}

fn prompt_for_label_char_command(quoted_path: &str) -> String {
    format!(r#"run-shell "printf '%s' '%%%' > {quoted_path}""#)
}

#[cfg(test)]
mod tests {
    use super::{
        CopyModeJumpCommand, copy_mode_jump_commands, parse_single_char_content,
        prompt_for_label_char_command, shell_quote, trim_single_trailing_newline,
    };

    use crate::jump::JumpTarget;

    #[test]
    fn capture_output_only_trims_tmuxs_final_newline() {
        assert_eq!(trim_single_trailing_newline(b"alpha\n"), "alpha");
        assert_eq!(trim_single_trailing_newline(b"alpha\n\n"), "alpha\n");
        assert_eq!(trim_single_trailing_newline(b"alpha"), "alpha");
    }

    #[test]
    fn shell_quote_handles_spaces_and_quotes() {
        assert_eq!(shell_quote("/tmp/tmux dart"), "'/tmp/tmux dart'");
        assert_eq!(shell_quote("/tmp/a'b"), "'/tmp/a'\\''b'");
    }

    #[test]
    fn prompt_for_label_char_command_escapes_prompt_input() {
        assert_eq!(
            prompt_for_label_char_command("' /tmp/path '"),
            r#"run-shell "printf '%s' '%%%' > ' /tmp/path '""#
        );
    }

    #[test]
    fn parse_single_char_content_accepts_single_character_with_newline() {
        assert!(matches!(parse_single_char_content("Z\n"), Ok(Some('Z'))));
    }

    #[test]
    fn parse_single_char_content_accepts_single_character_with_crlf() {
        assert!(matches!(parse_single_char_content("Z\r\n"), Ok(Some('Z'))));
    }

    #[test]
    fn parse_single_char_content_rejects_multiple_characters() {
        assert!(parse_single_char_content("ab").is_err());
    }

    #[test]
    fn parse_single_char_content_rejects_multiple_characters_with_newline() {
        assert!(parse_single_char_content("ab\n").is_err());
    }

    #[test]
    fn parse_single_char_content_rejects_extra_trailing_newlines() {
        assert!(parse_single_char_content("Z\n\n").is_err());
    }

    #[test]
    fn parse_single_char_content_returns_none_for_empty_input() {
        assert!(matches!(parse_single_char_content(""), Ok(None)));
    }

    #[test]
    fn copy_mode_jump_commands_use_display_rows_and_columns() {
        let pane = pane_with_scroll_position(3);

        assert_eq!(
            copy_mode_jump_commands(&pane, JumpTarget { row: 2, column: 5 }),
            vec![
                CopyModeJumpCommand::CopyMode,
                CopyModeJumpCommand::StartOfLine,
                CopyModeJumpCommand::TopLine,
                CopyModeJumpCommand::CursorUp(3),
                CopyModeJumpCommand::CursorDown(2),
                CopyModeJumpCommand::CursorRight(5),
            ]
        );
    }

    #[test]
    fn copy_mode_jump_commands_omit_zero_distance_moves() {
        let pane = pane_with_scroll_position(0);

        assert_eq!(
            copy_mode_jump_commands(&pane, JumpTarget { row: 0, column: 0 }),
            vec![
                CopyModeJumpCommand::CopyMode,
                CopyModeJumpCommand::StartOfLine,
                CopyModeJumpCommand::TopLine,
            ]
        );
    }

    fn pane_with_scroll_position(scroll_position: usize) -> super::PaneState {
        super::PaneState {
            pane_id: String::from("%0"),
            tty_path: String::from("/dev/null"),
            in_copy_mode: false,
            cursor_y: 0,
            cursor_x: 0,
            alternate_on: false,
            scroll_position,
            pane_height: 24,
        }
    }
}
