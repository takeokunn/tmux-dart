use std::{
    ffi::OsStr,
    fmt::Debug,
    fs::OpenOptions,
    io::{ErrorKind, Read},
    process::Command,
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use anyhow::{Context, Result, anyhow, bail, ensure};
use tempfile::NamedTempFile;

use crate::jump::JumpTarget;

const MAX_INITIAL_CHAR_FILE_BYTES: usize = 8;
const MAX_LABEL_INPUT_FILE_BYTES: usize = 4 * 1024;
const PROMPT_FAST_POLL_WINDOW: Duration = Duration::from_millis(250);
const PROMPT_RESPONSIVE_POLL_WINDOW: Duration = Duration::from_secs(1);
const PROMPT_FAST_POLL_INTERVAL: Duration = Duration::from_millis(5);
const PROMPT_RESPONSIVE_POLL_INTERVAL: Duration = Duration::from_millis(20);
const PROMPT_IDLE_POLL_INTERVAL: Duration = Duration::from_millis(50);

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
    /// History size when this snapshot was taken. A pane that keeps producing
    /// output (`tail -f`, a busy TUI on the normal screen) pushes the captured
    /// region one line further into history per new line; comparing this
    /// against the current history size re-anchors captures and jumps to the
    /// content the user actually saw. On the alternate screen history stays 0,
    /// so the compensation is a no-op there.
    pub history_size: usize,
}

pub trait TmuxBackend {
    fn current_pane_id(&self) -> Result<String>;
    fn pane_state(&self, pane_id: &str) -> Result<PaneState>;
    fn cancel_copy_mode(&self, pane: &PaneState) -> Result<()>;
    fn capture_visible_pane(&self, pane: &PaneState) -> Result<String>;
    fn capture_pane_with_escapes(&self, pane_id: &str) -> Result<String>;
    fn write_to_tty(&self, pane: &PaneState, content: &str) -> Result<()>;
    fn prompt_for_label_input(&self, prompt: &str) -> Result<Option<String>>;
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

    fn prompt_for_label_input(&self, prompt: &str) -> Result<Option<String>> {
        prompt_for_label_input(prompt)
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
    let format = "#{pane_id};#{pane_tty};#{pane_in_mode};#{cursor_y};#{cursor_x};#{alternate_on};#{scroll_position};#{pane_height};#{history_size}";
    let output = tmux_output(["display-message", "-p", "-t", pane_id, "-F", format])?;
    let parts: Vec<&str> = output.split(';').collect();
    if parts.len() != 9 {
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
        history_size: parse_usize_or_zero(parts[8], "history_size")?,
    })
}

fn pane_history_size(pane_id: &str) -> Result<usize> {
    let output = tmux_output([
        "display-message",
        "-p",
        "-t",
        pane_id,
        "-F",
        "#{history_size}",
    ])?;
    parse_usize_or_zero(&output, "history_size")
}

/// Lines above the *current* visible top where the region snapshotted in
/// `pane` now sits. Output that arrived since the snapshot pushed that region
/// deeper into history, one line per new history line; without this the
/// capture and the jump would both land `delta` rows below the content the
/// user saw. Saturating: `clear-history` during selection cannot underflow.
fn rebased_scroll_position(pane: &PaneState, history_size_now: usize) -> usize {
    pane.scroll_position
        .saturating_add(history_size_now.saturating_sub(pane.history_size))
}

fn parse_usize_or_zero(value: &str, field: &str) -> Result<usize> {
    if value.is_empty() {
        return Ok(0);
    }

    value.parse().with_context(|| format!("invalid {field}"))
}

pub fn capture_visible_pane(pane: &PaneState) -> Result<String> {
    let scroll = rebased_scroll_position(pane, pane_history_size(&pane.pane_id)?);
    let (start, end) = capture_range(scroll, pane.pane_height)?;
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
    .map(|mut screen| {
        screen.retain(|ch| ch != '\u{fe0e}');
        screen
    })
}

fn capture_range(scroll: usize, pane_height: usize) -> Result<(isize, isize)> {
    let scroll = isize::try_from(scroll).context("tmux scroll position exceeds supported range")?;
    let pane_height =
        isize::try_from(pane_height).context("tmux pane height exceeds supported range")?;
    ensure!(pane_height > 0, "tmux pane height must be positive");
    let start = scroll
        .checked_neg()
        .context("tmux scroll position exceeds supported range")?;
    let end = start
        .checked_add(pane_height - 1)
        .context("tmux capture range exceeds supported range")?;
    Ok((start, end))
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

fn wait_for_char_file(
    path: &str,
    previous_activity: Option<&str>,
    timeout: Duration,
) -> Result<Option<char>> {
    let started_at = Instant::now();
    let deadline = started_at + timeout;
    let activity_change_grace_period = Duration::from_millis(250);
    let mut next_activity_check = started_at + activity_change_grace_period;
    let mut file_was_observed = false;

    loop {
        if let Some(content) = read_bounded_utf8_file(path, MAX_INITIAL_CHAR_FILE_BYTES)? {
            file_was_observed = true;
            if let Some(ch) = parse_single_char_content(&content)? {
                return Ok(Some(ch));
            }
        }

        if session_activity_changed(previous_activity, &mut next_activity_check)? {
            return Ok(None);
        }

        if Instant::now() >= deadline {
            ensure!(file_was_observed, "prompt file was not found: {path}");
            return Ok(None);
        }

        sleep_until_next_prompt_poll(started_at, deadline);
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
    ensure!(
        !first.is_control(),
        "prompt file does not accept control characters"
    );
    Ok(Some(first))
}

pub fn prompt_for_label_input(prompt: &str) -> Result<Option<String>> {
    let temp = NamedTempFile::new().context("failed to create temporary prompt file")?;
    let path = temp.path().to_string_lossy().into_owned();
    let quoted_path = shell_quote(&path);

    tmux_success([
        "command-prompt",
        "-1",
        "-p",
        prompt,
        &prompt_for_label_input_command(&quoted_path),
    ])?;
    let previous_activity = session_activity()?;

    wait_for_prompt_file(&path, Some(&previous_activity), Duration::from_secs(10))
}

fn wait_for_prompt_file(
    path: &str,
    previous_activity: Option<&str>,
    timeout: Duration,
) -> Result<Option<String>> {
    let started_at = Instant::now();
    let deadline = started_at + timeout;
    let activity_change_grace_period = Duration::from_millis(250);
    let mut next_activity_check = started_at + activity_change_grace_period;

    loop {
        if let Some(input) = read_prompt_input_from_file(path)? {
            return Ok(Some(input));
        }

        if session_activity_changed(previous_activity, &mut next_activity_check)? {
            return Ok(None);
        }

        if Instant::now() >= deadline {
            return Ok(None);
        }

        sleep_until_next_prompt_poll(started_at, deadline);
    }
}

fn prompt_poll_interval(elapsed: Duration) -> Duration {
    if elapsed < PROMPT_FAST_POLL_WINDOW {
        PROMPT_FAST_POLL_INTERVAL
    } else if elapsed < PROMPT_RESPONSIVE_POLL_WINDOW {
        PROMPT_RESPONSIVE_POLL_INTERVAL
    } else {
        PROMPT_IDLE_POLL_INTERVAL
    }
}

fn sleep_until_next_prompt_poll(started_at: Instant, deadline: Instant) {
    let now = Instant::now();
    let interval = prompt_poll_interval(now.saturating_duration_since(started_at));
    thread::sleep(interval.min(deadline.saturating_duration_since(now)));
}

fn session_activity_changed(
    previous_activity: Option<&str>,
    next_check: &mut Instant,
) -> Result<bool> {
    let Some(previous_activity) = previous_activity else {
        return Ok(false);
    };
    let now = Instant::now();
    if now < *next_check {
        return Ok(false);
    }

    *next_check = now + Duration::from_millis(250);
    Ok(session_activity()? != previous_activity)
}

fn read_prompt_input_from_file(path: &str) -> Result<Option<String>> {
    let Some(content) = read_bounded_utf8_file(path, MAX_LABEL_INPUT_FILE_BYTES)? else {
        return Ok(None);
    };
    parse_prompt_content(&content)
}

fn read_bounded_utf8_file(path: &str, limit: usize) -> Result<Option<String>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NONBLOCK);

    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to open prompt file {path}"));
        }
    };
    ensure!(
        file.metadata()
            .with_context(|| format!("failed to inspect prompt file {path}"))?
            .is_file(),
        "prompt path is not a regular file: {path}"
    );
    let read_limit = limit
        .checked_add(1)
        .context("prompt file byte limit exceeds supported range")?;
    let read_limit_u64 =
        u64::try_from(read_limit).context("prompt file byte limit exceeds supported range")?;
    let mut bytes = Vec::with_capacity(read_limit);
    file.by_ref()
        .take(read_limit_u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read prompt file {path}"))?;
    ensure!(
        bytes.len() <= limit,
        "prompt file exceeds {limit} byte limit"
    );
    String::from_utf8(bytes)
        .context("prompt file must contain valid UTF-8")
        .map(Some)
}

fn parse_prompt_content(content: &str) -> Result<Option<String>> {
    let content = strip_optional_line_ending(content);
    if content.is_empty() {
        return Ok(None);
    }

    Ok(Some(content.to_owned()))
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
    // Enter copy mode first: it pins the view, so the scroll compensation read
    // right after stays valid even while the pane keeps producing output.
    tmux_success(["copy-mode", "-t", &pane.pane_id])?;
    let scroll_up = rebased_scroll_position(pane, pane_history_size(&pane.pane_id)?);
    let commands = copy_mode_movement_commands(scroll_up, jump_to);
    send_copy_mode_jump_commands(pane, &commands)?;

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyModeJumpCommand {
    StartOfLine,
    TopLine,
    CursorUp(usize),
    CursorDown(usize),
    CursorRight(usize),
}

/// Movements to run after copy mode is entered. `scroll_up` is the (already
/// history-compensated) number of rows between the current visible top and the
/// top of the captured region.
fn copy_mode_movement_commands(scroll_up: usize, jump_to: JumpTarget) -> Vec<CopyModeJumpCommand> {
    let mut commands = vec![
        CopyModeJumpCommand::StartOfLine,
        CopyModeJumpCommand::TopLine,
    ];

    if scroll_up > 0 {
        commands.push(CopyModeJumpCommand::CursorUp(scroll_up));
    }
    if jump_to.row > 0 {
        commands.push(CopyModeJumpCommand::CursorDown(jump_to.row));
    }
    if jump_to.column > 0 {
        commands.push(CopyModeJumpCommand::CursorRight(jump_to.column));
    }

    commands
}

fn send_copy_mode_jump_commands(pane: &PaneState, commands: &[CopyModeJumpCommand]) -> Result<()> {
    let args = copy_mode_movement_args(&pane.pane_id, commands);
    if args.is_empty() {
        return Ok(());
    }
    tmux_success_args(&args)
}

fn copy_mode_movement_args(pane_id: &str, commands: &[CopyModeJumpCommand]) -> Vec<String> {
    let mut args = Vec::with_capacity(commands.len().saturating_mul(8));

    for (index, command) in commands.iter().enumerate() {
        if index > 0 {
            args.push(String::from(";"));
        }
        args.extend([
            String::from("send-keys"),
            String::from("-X"),
            String::from("-t"),
            pane_id.to_owned(),
        ]);

        match command {
            CopyModeJumpCommand::StartOfLine => args.push(String::from("start-of-line")),
            CopyModeJumpCommand::TopLine => args.push(String::from("top-line")),
            CopyModeJumpCommand::CursorUp(count) => {
                args.extend([
                    String::from("-N"),
                    count.to_string(),
                    String::from("cursor-up"),
                ]);
            }
            CopyModeJumpCommand::CursorDown(count) => {
                args.extend([
                    String::from("-N"),
                    count.to_string(),
                    String::from("cursor-down"),
                ]);
            }
            CopyModeJumpCommand::CursorRight(count) => {
                args.extend([
                    String::from("-N"),
                    count.to_string(),
                    String::from("cursor-right"),
                ]);
            }
        }
    }

    args
}

fn session_activity() -> Result<String> {
    tmux_output(["display-message", "-p", "#{session_activity}"])
}

fn tmux_stdout<const N: usize>(args: [&str; N]) -> Result<Vec<u8>> {
    tmux_stdout_args(&args)
}

fn tmux_stdout_args<T>(args: &[T]) -> Result<Vec<u8>>
where
    T: AsRef<OsStr> + Debug,
{
    let output = Command::new("tmux")
        .args(args.iter().map(AsRef::as_ref))
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

fn tmux_success_args<T>(args: &[T]) -> Result<()>
where
    T: AsRef<OsStr> + Debug,
{
    tmux_stdout_args(args).map(|_| ())
}

fn prompt_for_label_input_command(quoted_path: &str) -> String {
    format!(r#"run-shell "printf '%s' #{{q:%%%}} > {quoted_path}""#)
}

#[cfg(test)]
mod tests {
    use std::{io::Write, time::Duration};

    #[cfg(unix)]
    use std::{process::Command, time::Instant};

    use anyhow::Result;
    use tempfile::NamedTempFile;

    use super::{
        CopyModeJumpCommand, MAX_INITIAL_CHAR_FILE_BYTES, PROMPT_FAST_POLL_INTERVAL,
        PROMPT_FAST_POLL_WINDOW, PROMPT_IDLE_POLL_INTERVAL, PROMPT_RESPONSIVE_POLL_INTERVAL,
        PROMPT_RESPONSIVE_POLL_WINDOW, capture_range, copy_mode_movement_args,
        copy_mode_movement_commands, parse_prompt_content, parse_single_char_content,
        prompt_for_label_input_command, prompt_poll_interval, read_bounded_utf8_file,
        rebased_scroll_position, shell_quote, trim_single_trailing_newline, wait_for_char_file,
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
    fn prompt_for_label_input_command_escapes_prompt_input() {
        assert_eq!(
            prompt_for_label_input_command("' /tmp/path '"),
            r#"run-shell "printf '%s' #{q:%%%} > ' /tmp/path '""#
        );
    }

    #[test]
    fn parse_prompt_content_accepts_single_value_with_newline() {
        assert!(matches!(parse_prompt_content("Z\n"), Ok(Some(value)) if value == "Z"));
    }

    #[test]
    fn parse_prompt_content_accepts_multi_character_input_with_newline() {
        assert!(matches!(parse_prompt_content("ab\n"), Ok(Some(value)) if value == "ab"));
    }

    #[test]
    fn parse_prompt_content_returns_none_for_empty_input() {
        assert!(matches!(parse_prompt_content(""), Ok(None)));
    }

    #[test]
    fn parse_prompt_content_preserves_single_character_values() {
        assert!(matches!(parse_prompt_content("Z"), Ok(Some(value)) if value == "Z"));
    }

    #[test]
    fn prompt_polling_is_fast_for_immediate_input_then_backs_off() {
        assert_eq!(
            prompt_poll_interval(Duration::ZERO),
            PROMPT_FAST_POLL_INTERVAL
        );
        assert_eq!(
            prompt_poll_interval(PROMPT_FAST_POLL_WINDOW - Duration::from_nanos(1)),
            PROMPT_FAST_POLL_INTERVAL
        );
        assert_eq!(
            prompt_poll_interval(PROMPT_FAST_POLL_WINDOW),
            PROMPT_RESPONSIVE_POLL_INTERVAL
        );
        assert_eq!(
            prompt_poll_interval(PROMPT_RESPONSIVE_POLL_WINDOW - Duration::from_nanos(1)),
            PROMPT_RESPONSIVE_POLL_INTERVAL
        );
        assert_eq!(
            prompt_poll_interval(PROMPT_RESPONSIVE_POLL_WINDOW),
            PROMPT_IDLE_POLL_INTERVAL
        );
    }

    #[test]
    fn bounded_prompt_read_rejects_oversized_files() -> Result<()> {
        let mut temp = NamedTempFile::new()?;
        temp.write_all(&[b'x'; MAX_INITIAL_CHAR_FILE_BYTES + 1])?;

        let result =
            read_bounded_utf8_file(&temp.path().to_string_lossy(), MAX_INITIAL_CHAR_FILE_BYTES);

        assert!(matches!(result, Err(error) if error.to_string().contains("byte limit")));
        Ok(())
    }

    #[test]
    fn bounded_prompt_read_rejects_invalid_utf8() -> Result<()> {
        let mut temp = NamedTempFile::new()?;
        temp.write_all(&[0xff])?;

        let result =
            read_bounded_utf8_file(&temp.path().to_string_lossy(), MAX_INITIAL_CHAR_FILE_BYTES);

        assert!(matches!(result, Err(error) if error.to_string().contains("valid UTF-8")));
        Ok(())
    }

    #[test]
    fn bounded_prompt_read_rejects_non_regular_files() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let result = read_bounded_utf8_file(
            &temp_dir.path().to_string_lossy(),
            MAX_INITIAL_CHAR_FILE_BYTES,
        );

        assert!(matches!(result, Err(error) if error.to_string().contains("not a regular file")));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn bounded_prompt_read_rejects_fifo_without_blocking() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let fifo = temp_dir.path().join("prompt-fifo");
        let status = Command::new("mkfifo").arg(&fifo).status()?;
        assert!(status.success());

        let started_at = Instant::now();
        let result = read_bounded_utf8_file(&fifo.to_string_lossy(), MAX_INITIAL_CHAR_FILE_BYTES);

        assert!(started_at.elapsed() < Duration::from_secs(1));
        assert!(matches!(result, Err(error) if error.to_string().contains("not a regular file")));
        Ok(())
    }

    #[test]
    fn initial_char_parser_rejects_control_characters() {
        for content in ["\t", "\u{1b}", "\u{7f}"] {
            assert!(parse_single_char_content(content).is_err());
        }
    }

    #[test]
    fn initial_char_file_accepts_unicode_with_line_ending() -> Result<()> {
        let mut temp = NamedTempFile::new()?;
        temp.write_all("界\r\n".as_bytes())?;

        assert_eq!(
            wait_for_char_file(&temp.path().to_string_lossy(), None, Duration::ZERO)?,
            Some('界')
        );
        Ok(())
    }

    #[test]
    fn missing_initial_char_file_is_an_error_after_timeout() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let missing = temp_dir.path().join("missing-char");

        let result = wait_for_char_file(&missing.to_string_lossy(), None, Duration::ZERO);

        assert!(matches!(result, Err(error) if error.to_string().contains("was not found")));
        Ok(())
    }

    #[test]
    fn copy_mode_movements_use_scroll_rows_and_columns() {
        assert_eq!(
            copy_mode_movement_commands(3, JumpTarget { row: 2, column: 5 }),
            vec![
                CopyModeJumpCommand::StartOfLine,
                CopyModeJumpCommand::TopLine,
                CopyModeJumpCommand::CursorUp(3),
                CopyModeJumpCommand::CursorDown(2),
                CopyModeJumpCommand::CursorRight(5),
            ]
        );
    }

    #[test]
    fn copy_mode_movements_omit_zero_distance_moves() {
        assert_eq!(
            copy_mode_movement_commands(0, JumpTarget { row: 0, column: 0 }),
            vec![
                CopyModeJumpCommand::StartOfLine,
                CopyModeJumpCommand::TopLine,
            ]
        );
    }

    #[test]
    fn copy_mode_movements_form_one_tmux_command_list() {
        let commands = copy_mode_movement_commands(3, JumpTarget { row: 2, column: 5 });

        assert_eq!(
            copy_mode_movement_args("%17", &commands),
            [
                "send-keys",
                "-X",
                "-t",
                "%17",
                "start-of-line",
                ";",
                "send-keys",
                "-X",
                "-t",
                "%17",
                "top-line",
                ";",
                "send-keys",
                "-X",
                "-t",
                "%17",
                "-N",
                "3",
                "cursor-up",
                ";",
                "send-keys",
                "-X",
                "-t",
                "%17",
                "-N",
                "2",
                "cursor-down",
                ";",
                "send-keys",
                "-X",
                "-t",
                "%17",
                "-N",
                "5",
                "cursor-right",
            ]
        );
    }

    #[test]
    fn rebased_scroll_position_follows_new_output_into_history() {
        let pane = pane_with_scroll_and_history(4, 100);
        // No new output: the original scroll offset still holds.
        assert_eq!(rebased_scroll_position(&pane, 100), 4);
        // Seven lines arrived while the user was picking a label: the captured
        // region is now seven lines further up.
        assert_eq!(rebased_scroll_position(&pane, 107), 11);
        // History shrank (clear-history): never underflow.
        assert_eq!(rebased_scroll_position(&pane, 0), 4);
    }

    #[test]
    fn rebased_scroll_position_saturates_instead_of_overflowing() {
        let pane = pane_with_scroll_and_history(usize::MAX, 0);
        assert_eq!(rebased_scroll_position(&pane, 1), usize::MAX);
    }

    #[test]
    fn capture_range_rejects_invalid_tmux_dimensions() {
        assert!(capture_range(0, 0).is_err());
        assert!(capture_range(usize::MAX, 1).is_err());
        assert!(capture_range(0, usize::MAX).is_err());
    }

    #[test]
    fn capture_range_includes_exactly_the_visible_rows() -> Result<()> {
        assert_eq!(capture_range(3, 5)?, (-3, 1));
        Ok(())
    }

    fn pane_with_scroll_and_history(
        scroll_position: usize,
        history_size: usize,
    ) -> super::PaneState {
        super::PaneState {
            pane_id: String::from("%0"),
            tty_path: String::from("/dev/null"),
            in_copy_mode: false,
            cursor_y: 0,
            cursor_x: 0,
            alternate_on: false,
            scroll_position,
            pane_height: 24,
            history_size,
        }
    }
}
