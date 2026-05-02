use std::{
    fs,
    io::ErrorKind,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use tempfile::NamedTempFile;

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

pub fn current_pane_id() -> Result<String> {
    tmux_output(["display-message", "-p", "#{pane_id}"])
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
        &format!(r#"run-shell "printf '%1' > {quoted_path}""#),
    ])?;
    let previous_activity = session_activity()?;

    wait_for_char_file(&path, Some(&previous_activity), Duration::from_secs(10))
}

fn wait_for_char_file(
    path: &str,
    previous_activity: Option<&str>,
    timeout: Duration,
) -> Result<Option<char>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(ch) = read_first_char_from_file(path)? {
            return Ok(Some(ch));
        }

        if let Some(previous_activity) = previous_activity
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

fn read_first_char_from_file(path: &str) -> Result<Option<char>> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(content.chars().next()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read prompt file {path}")),
    }
}

pub fn jump_to_position(pane: &PaneState, jump_to: usize) -> Result<()> {
    tmux_success(["copy-mode", "-t", &pane.pane_id])?;
    tmux_success(["send-keys", "-X", "-t", &pane.pane_id, "start-of-line"])?;
    tmux_success(["send-keys", "-X", "-t", &pane.pane_id, "top-line"])?;
    tmux_success([
        "send-keys",
        "-X",
        "-t",
        &pane.pane_id,
        "-N",
        "200",
        "cursor-right",
    ])?;
    tmux_success(["send-keys", "-X", "-t", &pane.pane_id, "start-of-line"])?;
    tmux_success(["send-keys", "-X", "-t", &pane.pane_id, "top-line"])?;
    if pane.scroll_position > 0 {
        tmux_success([
            "send-keys",
            "-X",
            "-t",
            &pane.pane_id,
            "-N",
            &pane.scroll_position.to_string(),
            "cursor-up",
        ])?;
    }
    if jump_to > 0 {
        tmux_success([
            "send-keys",
            "-X",
            "-t",
            &pane.pane_id,
            "-N",
            &jump_to.to_string(),
            "cursor-right",
        ])?;
    }

    Ok(())
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

fn tmux_success<const N: usize>(args: [&str; N]) -> Result<()> {
    tmux_output(args).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::{shell_quote, trim_single_trailing_newline};

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
}
