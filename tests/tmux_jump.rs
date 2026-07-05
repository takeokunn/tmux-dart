//! End-to-end tests that drive a real tmux server and assert the copy-mode
//! cursor lands on the intended cell.
//!
//! These exercise the parts that unit tests cannot: the actual copy-mode
//! `cursor-right`/`cursor-down` navigation, multi-row jumps, wrapped lines,
//! and capture of colored output. They are skipped automatically when tmux is
//! unavailable (for example the sandboxed `nix flake check` package build) and
//! run for real in CI via the `nix develop` shell, which provides tmux.
//!
//! Each test uses a private tmux server on a throwaway socket, so it never
//! touches the developer's real sessions.

use std::{process::Command, thread::sleep, time::Duration};

use anyhow::{Context, Result, bail};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_tmux-dart");

fn tmux_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// A private tmux server bound to a socket inside a temp dir. Killed on drop.
struct Server {
    dir: TempDir,
}

impl Server {
    /// Start a detached session of the given size whose pane prints `content`
    /// (interpreted with `printf %b`, so `\n` and `\033` are honored) and then
    /// stays alive. Returns the server and its pane id.
    fn start(content: &str, cols: u16, rows: u16) -> Result<(Self, String)> {
        let dir = tempfile::tempdir().context("create temp dir for tmux socket")?;
        let server = Server { dir };
        let pane_command = format!("printf %b {}; exec sleep 3600", shell_single_quote(content));
        server.tmux(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-x",
            &cols.to_string(),
            "-y",
            &rows.to_string(),
            &pane_command,
        ])?;
        let pane = server.tmux(&["display-message", "-p", "-F", "#{pane_id}"])?;

        // Wait until the shell has actually rendered the content.
        for _ in 0..40 {
            let captured = server.tmux(&["capture-pane", "-p", "-t", &pane])?;
            if captured.chars().any(|ch| !ch.is_whitespace()) {
                break;
            }
            sleep(Duration::from_millis(50));
        }

        Ok((server, pane))
    }

    fn socket(&self) -> Result<String> {
        self.dir
            .path()
            .join("sock")
            .to_str()
            .context("non-utf8 tmux socket path")
            .map(str::to_owned)
    }

    fn tmux(&self, args: &[&str]) -> Result<String> {
        let socket = self.socket()?;
        let output = Command::new("tmux")
            .arg("-S")
            .arg(&socket)
            .args(args)
            .output()
            .context("spawn tmux")?;
        if !output.status.success() {
            bail!(
                "tmux {:?} failed: {}",
                args,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end_matches('\n')
            .to_owned())
    }

    /// Run the built binary against `pane`, pointing `$TMUX` at this server so
    /// the binary's bare `tmux` calls hit it — exactly as when the plugin is
    /// spawned inside tmux via run-shell.
    fn run_jump(&self, pane: &str, ch: &str, mode: &str) -> Result<()> {
        let socket = self.socket()?;
        let pid = self.tmux(&["display-message", "-p", "-F", "#{pid}"])?;
        Command::new(BIN)
            .args(["jump", "--char", ch, "--pane-id", pane])
            .env("TMUX", format!("{socket},{pid},0"))
            .env("JUMP_MATCH_MODE", mode)
            .env("JUMP_AUTO_JUMP", "on")
            .env("JUMP_CASE_SENSITIVE", "off")
            .status()
            .context("spawn tmux-dart binary")?;
        // The binary reports its own errors; the no-match path exits 0 anyway.
        Ok(())
    }

    fn in_copy_mode(&self, pane: &str) -> Result<bool> {
        Ok(self.tmux(&["display-message", "-p", "-t", pane, "-F", "#{pane_in_mode}"])? == "1")
    }

    fn copy_cursor(&self, pane: &str) -> Result<(i64, i64)> {
        let x = self.tmux(&[
            "display-message",
            "-p",
            "-t",
            pane,
            "-F",
            "#{copy_cursor_x}",
        ])?;
        let y = self.tmux(&[
            "display-message",
            "-p",
            "-t",
            pane,
            "-F",
            "#{copy_cursor_y}",
        ])?;
        Ok((parse_i64(&x)?, parse_i64(&y)?))
    }

    /// Capture the visible grid as plain text (no escapes), one line per row.
    fn capture_plain(&self, pane: &str, rows: u16) -> Result<String> {
        self.tmux(&[
            "capture-pane",
            "-p",
            "-t",
            pane,
            "-S",
            "0",
            "-E",
            &(rows - 1).to_string(),
        ])
    }

    /// Capture the visible grid with SGR escapes retained — the exact input the
    /// overlay restore repaints from.
    fn capture_escaped(&self, pane: &str) -> Result<String> {
        self.tmux(&["capture-pane", "-e", "-p", "-t", pane])
    }

    /// Write raw bytes to the pane's slave tty, exactly as the overlay code does
    /// via `write_to_tty` — tmux processes them and updates the pane grid.
    fn write_pane_tty(&self, pane: &str, bytes: &str) -> Result<()> {
        use std::io::Write;
        let tty = self.tmux(&["display-message", "-p", "-t", pane, "-F", "#{pane_tty}"])?;
        let mut handle = std::fs::OpenOptions::new()
            .append(true)
            .open(&tty)
            .with_context(|| format!("open pane tty {tty}"))?;
        handle
            .write_all(bytes.as_bytes())
            .context("write pane tty")?;
        handle.flush().context("flush pane tty")?;
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Ok(socket) = self.socket() {
            let _ = Command::new("tmux")
                .arg("-S")
                .arg(&socket)
                .arg("kill-server")
                .output();
        }
    }
}

fn parse_i64(value: &str) -> Result<i64> {
    value
        .trim()
        .parse::<i64>()
        .with_context(|| format!("parse integer from tmux output {value:?}"))
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// A single-match auto-jump must leave the pane in copy mode with the cursor on
/// the expected grid cell.
fn assert_jump(
    content: &str,
    cols: u16,
    rows: u16,
    ch: &str,
    mode: &str,
    expect: (i64, i64),
) -> Result<()> {
    let (server, pane) = Server::start(content, cols, rows)?;
    server.run_jump(&pane, ch, mode)?;
    assert!(
        server.in_copy_mode(&pane)?,
        "jumping to {ch:?} in {content:?} should enter copy mode"
    );
    assert_eq!(
        server.copy_cursor(&pane)?,
        expect,
        "jump to {ch:?} in {content:?} landed on the wrong cell"
    );
    Ok(())
}

/// Grid column of the copy cursor after pressing `cursor-right` `presses`
/// times from the start of `content`'s first line, on a raw tmux server with
/// no tmux-dart involved. Probes how this environment's tmux lays `content`
/// out in grid cells, guarding cell-layout assertions against older tmux or
/// non-UTF-8 locales.
fn cursor_x_after_presses(content: &str, presses: usize) -> Result<i64> {
    let (server, pane) = Server::start(content, 80, 24)?;
    server.tmux(&["copy-mode", "-t", &pane])?;
    server.tmux(&["send-keys", "-X", "-t", &pane, "start-of-line"])?;
    server.tmux(&["send-keys", "-X", "-t", &pane, "top-line"])?;
    server.tmux(&[
        "send-keys",
        "-X",
        "-t",
        &pane,
        "-N",
        &presses.to_string(),
        "cursor-right",
    ])?;
    let x = server.tmux(&[
        "display-message",
        "-p",
        "-t",
        &pane,
        "-F",
        "#{copy_cursor_x}",
    ])?;
    parse_i64(&x)
}

/// Probe whether this environment's tmux treats CJK characters as double-width.
/// One `cursor-right` from column 0 of "あa" lands on column 2 when wide-aware
/// (the padding cell is skipped) and column 1 otherwise.
fn tmux_is_wide_aware() -> Result<bool> {
    Ok(cursor_x_after_presses("あa", 1)? == 2)
}

macro_rules! require_tmux {
    () => {
        if !tmux_available() {
            eprintln!("skipping tmux e2e: tmux is not available on PATH");
            return Ok(());
        }
    };
}

#[test]
fn jumps_to_single_ascii_match() -> Result<()> {
    require_tmux!();
    // 'w' is the only word-start match in "hello world" -> grid column 6.
    assert_jump("hello world", 80, 24, "w", "word", (6, 0))
}

#[test]
fn jumps_across_rows_with_cursor_down() -> Result<()> {
    require_tmux!();
    // 'w' only appears on the second line -> row 1, column 6.
    assert_jump("alpha beta\\nhello world", 80, 24, "w", "word", (6, 1))
}

#[test]
fn jumps_to_non_word_target_in_word_mode() -> Result<()> {
    require_tmux!();
    // '/' is never a word start, but word mode falls back to matching it.
    assert_jump("a/b", 80, 24, "/", "word", (1, 0))
}

#[test]
fn jumps_to_mid_word_char_in_char_mode() -> Result<()> {
    require_tmux!();
    // 'r' in "world" is not a word start; char mode still finds the lone match.
    assert_jump("world", 80, 24, "r", "char", (2, 0))
}

#[test]
fn jumps_into_a_wrapped_line() -> Result<()> {
    require_tmux!();
    // 15 chars in a 10-column pane wrap; 'm' displays on the 2nd row, column 2.
    assert_jump("abcdefghijklmno", 10, 24, "m", "char", (2, 1))
}

#[test]
fn matches_against_captured_text_ignoring_color_escapes() -> Result<()> {
    require_tmux!();
    // Colored output: capture (without -e) yields plain text, so 'P' (word start
    // of "PASS" on the 2nd line) is found at row 1, column 0.
    assert_jump(
        "\\033[31mERROR\\033[0m ok\\n\\033[32mPASS\\033[0m done",
        80,
        24,
        "P",
        "word",
        (0, 1),
    )
}

#[test]
fn jumps_to_the_bottom_row() -> Result<()> {
    require_tmux!();
    // Target on the last row of a short pane.
    assert_jump("l1\\nl2\\nl3\\nl4\\nzebra", 80, 5, "z", "word", (0, 4))
}

#[test]
fn reports_no_match_without_entering_copy_mode() -> Result<()> {
    require_tmux!();
    let (server, pane) = Server::start("hello world", 80, 24)?;
    server.run_jump(&pane, "Q", "word")?;
    assert!(
        !server.in_copy_mode(&pane)?,
        "a jump with no matches must not enter copy mode"
    );
    Ok(())
}

#[test]
fn jumps_to_the_correct_cell_after_wide_characters() -> Result<()> {
    require_tmux!();
    if !tmux_is_wide_aware()? {
        eprintln!("skipping wide-char e2e: tmux is not double-width aware in this environment");
        return Ok(());
    }
    // "あいう z": 'z' is reached with 4 character-count presses and sits on grid
    // column 7 (each wide char occupies two cells). Counting display cells would
    // overshoot — this is the regression guard driven against real tmux.
    assert_jump("あいう z", 80, 24, "z", "word", (7, 0))?;
    // Same, one row down and with a shorter wide run: grid column 5, row 1.
    assert_jump("top\\nかき z", 80, 24, "z", "word", (5, 1))
}

#[test]
fn jumps_to_the_correct_cell_after_combining_marks() -> Result<()> {
    require_tmux!();
    // macOS `ls` emits NFD: each か+゙ pair shares one grid cell, so 'z' is
    // three presses away on grid column 5. Counting the combining marks as
    // presses would run past the line end and wrap onto the next row.
    if cursor_x_after_presses("か\u{3099}き\u{3099} z", 3)? != 5 {
        eprintln!("skipping NFD e2e: tmux does not combine NFD marks in this environment");
        return Ok(());
    }
    assert_jump("か\u{3099}き\u{3099} z", 80, 24, "z", "word", (5, 0))
}

#[test]
fn jumps_to_the_correct_cell_after_emoji_clusters() -> Result<()> {
    require_tmux!();
    // Each cluster is one grid cell of width 2 in modern tmux: 'z' is two
    // presses away on grid column 3. The raw-tmux probe skips a case when this
    // environment's tmux does not combine that cluster kind.
    let cases = [
        ("👨\u{200d}👩\u{200d}👧 z", "ZWJ family emoji"),
        ("☝\u{fe0f} z", "VS16 emoji presentation"),
        ("🇯🇵 z", "regional indicator pair"),
        ("👍🏻 z", "skin-tone modifier"),
    ];
    for (content, kind) in cases {
        if cursor_x_after_presses(content, 2)? != 3 {
            eprintln!("skipping {kind} e2e: tmux does not combine it into one cell here");
            continue;
        }
        assert_jump(content, 80, 24, "z", "word", (3, 0))?;
    }
    Ok(())
}

#[test]
fn jumps_on_the_alternate_screen() -> Result<()> {
    require_tmux!();
    // A TUI-style pane: rows painted on the alternate screen, where there is
    // no history and no scroll offset.
    let content = "\\033[?1049h\\033[Halpha beta\\r\\ngamma zebra";
    let (server, pane) = Server::start(content, 80, 24)?;
    assert_eq!(
        server.tmux(&[
            "display-message",
            "-p",
            "-t",
            &pane,
            "-F",
            "#{alternate_on}"
        ])?,
        "1",
        "the fixture must be on the alternate screen"
    );

    server.run_jump(&pane, "z", "word")?;
    assert!(server.in_copy_mode(&pane)?);
    assert_eq!(server.copy_cursor(&pane)?, (6, 1));
    Ok(())
}

#[test]
fn jumps_within_scrolled_history() -> Result<()> {
    require_tmux!();
    // The user scrolled 15 lines up in copy mode before triggering the jump:
    // the flow must capture the *scrolled* view, and the jump must land there
    // (cursor-up over the scroll offset), not on the bottom of history.
    let mut lines: Vec<String> = (1..=40)
        .map(|i| {
            if i == 20 {
                String::from("Qmarker")
            } else {
                format!("line {i:02}")
            }
        })
        .collect();
    lines.push(String::from("bottom"));
    let content = lines.join("\\n");

    let (server, pane) = Server::start(&content, 80, 10)?;
    server.tmux(&["copy-mode", "-t", &pane])?;
    server.tmux(&["send-keys", "-X", "-t", &pane, "-N", "15", "scroll-up"])?;

    server.run_jump(&pane, "Q", "word")?;
    assert!(server.in_copy_mode(&pane)?);
    // "Qmarker" (history line 20) is the 4th visible row of the scrolled view.
    assert_eq!(server.copy_cursor(&pane)?, (0, 3));
    assert_eq!(
        server.tmux(&[
            "display-message",
            "-p",
            "-t",
            &pane,
            "-F",
            "#{scroll_position}"
        ])?,
        "15",
        "the jump must stay within the view the user had scrolled to"
    );
    Ok(())
}

#[test]
fn overlay_restore_does_not_drift_under_a_scroll_region() -> Result<()> {
    require_tmux!();
    // Regression guard for the reported TUI drift: a full-screen app (vim split,
    // htop, anything with a fixed header/footer) narrows the scroll region with
    // DECSTBM (`ESC[top;bottom r`). The multi-match overlay path repaints the
    // saved screen line by line; if it does not reset that region first, every
    // `\n` scrolls *inside* the region and shifts the whole grid, so the jump —
    // computed against the pre-overlay capture — lands rows off. The restore
    // sequence must round-trip the grid unchanged.
    let cols = 40u16;
    let rows = 10u16;
    // Enter the alternate screen, draw distinct rows, then reserve a header
    // (rows 1-2) and footer (row `rows`) by setting the scroll region to
    // rows 3..rows-1, and park the cursor inside it — just like a real TUI.
    let body = (0..rows)
        .map(|r| format!("ROW{r:02} zebra"))
        .collect::<Vec<_>>()
        .join("\\r\\n");
    let content = format!("\\033[?1049h\\033[H{body}\\033[3;{}r\\033[5;1H", rows - 1);
    let (server, pane) = Server::start(&content, cols, rows)?;

    assert_eq!(
        server.tmux(&[
            "display-message",
            "-p",
            "-t",
            &pane,
            "-F",
            "#{alternate_on}"
        ])?,
        "1",
        "the fixture must be on the alternate screen"
    );

    let before = server.capture_plain(&pane, rows)?;
    let saved = server.capture_escaped(&pane)?;
    // Drive the exact production restore sequence against the live pane.
    let restore = tmux_dart::overlay::alternate_screen_restore_sequence(&saved, 0, 0);
    server.write_pane_tty(&pane, &restore)?;
    sleep(Duration::from_millis(100));
    let after = server.capture_plain(&pane, rows)?;

    assert_eq!(
        trim_trailing_ws(&before),
        trim_trailing_ws(&after),
        "restoring the screen under a scroll region shifted the grid (TUI drift)"
    );
    Ok(())
}

/// Compare grids ignoring per-line trailing spaces, which `capture-pane` may or
/// may not emit depending on where content ends.
fn trim_trailing_ws(screen: &str) -> String {
    screen
        .lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}
