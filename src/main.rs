use std::{env, process::ExitCode};

use anyhow::{Context, Result, bail};

use tmux_dart::{
    jump::{
        DEFAULT_LABEL_KEYS, KeyPosition, MatchMode, bounded_subset_bounds, label_keys_from_env,
        labels_for, positions_for,
    },
    overlay::{OverlayStyle, decode_tmux_color, draw_overlay, with_recovered_screen},
    tmux::{
        PaneState, cancel_copy_mode, capture_visible_pane, current_pane_id, jump_to_position,
        pane_state, prompt_for_label_char, read_initial_char_from_file,
    },
};

const JUMP_USAGE: &str =
    "usage: tmux-dart jump (--char <char> | --char-file <path>) [--pane-id <pane>]";

#[derive(Debug, Clone)]
struct JumpOptions {
    label_keys: Vec<char>,
    match_mode: MatchMode,
    case_sensitive: bool,
    auto_jump: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("tmux-dart: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("jump") => {
            let mut jump_char: Option<char> = None;
            let mut jump_char_file: Option<String> = None;
            let mut explicit_pane_id: Option<String> = None;

            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--char" => {
                        let value = args.next().context("missing value for --char")?;
                        jump_char = value.chars().next();
                    }
                    "--char-file" => {
                        jump_char_file =
                            Some(args.next().context("missing value for --char-file")?);
                    }
                    "--pane-id" => {
                        explicit_pane_id =
                            Some(args.next().context("missing value for --pane-id")?);
                    }
                    other => bail!("unsupported argument: {other}"),
                }
            }

            if jump_char.is_some() && jump_char_file.is_some() {
                bail!("--char and --char-file are mutually exclusive");
            }

            let jump_char = if let Some(jump_char) = jump_char {
                Some(jump_char)
            } else if let Some(path) = jump_char_file {
                read_initial_char_from_file(&path)?
            } else {
                bail!("{JUMP_USAGE}")
            };
            let Some(jump_char) = jump_char else {
                return Ok(());
            };
            run_jump(jump_char, explicit_pane_id)
        }
        _ => bail!("{JUMP_USAGE}"),
    }
}

fn run_jump(initial_char: char, explicit_pane_id: Option<String>) -> Result<()> {
    let pane_id = if let Some(pane_id) = explicit_pane_id {
        pane_id
    } else {
        current_pane_id()?
    };
    let pane = pane_state(&pane_id)?;
    cancel_copy_mode(&pane)?;

    let screen = capture_visible_pane(&pane)?;
    let options = jump_options_from_env();
    let positions = positions_for(
        initial_char,
        &screen,
        options.match_mode,
        options.case_sensitive,
    );
    if positions.is_empty() {
        return Ok(());
    }

    let style = OverlayStyle {
        background: decode_tmux_color(
            &env::var("JUMP_BACKGROUND_COLOR").unwrap_or_else(|_| String::from(r#"\e[0m\e[32m"#)),
        ),
        foreground: decode_tmux_color(
            &env::var("JUMP_FOREGROUND_COLOR").unwrap_or_else(|_| String::from(r#"\e[1m\e[31m"#)),
        ),
        key_position: KeyPosition::from_env(
            &env::var("JUMP_KEYS_POSITION").unwrap_or_else(|_| String::from("left")),
        ),
    };

    let selected_index = with_recovered_screen(&pane, || {
        select_position_index(&pane, &screen, &positions, &style, &options)
    })?;
    let Some(selected_index) = selected_index else {
        return Ok(());
    };

    let target = positions
        .get(selected_index)
        .copied()
        .context("selected index out of bounds")?;
    jump_to_position(&pane, target)
}

fn jump_options_from_env() -> JumpOptions {
    let label_keys = env::var("JUMP_LABEL_KEYS")
        .map(|value| label_keys_from_env(&value))
        .unwrap_or_else(|_| DEFAULT_LABEL_KEYS.to_vec());
    let match_mode =
        MatchMode::from_env(&env::var("JUMP_MATCH_MODE").unwrap_or_else(|_| String::from("word")));
    let case_sensitive = option_enabled(&env::var("JUMP_CASE_SENSITIVE").unwrap_or_default());
    let auto_jump = !option_disabled(&env::var("JUMP_AUTO_JUMP").unwrap_or_default());

    JumpOptions {
        label_keys,
        match_mode,
        case_sensitive,
        auto_jump,
    }
}

fn option_enabled(value: &str) -> bool {
    value == "1" || value == "on" || value == "true" || value == "yes"
}

fn option_disabled(value: &str) -> bool {
    value == "0" || value == "off" || value == "false" || value == "no"
}

fn select_position_index(
    pane: &PaneState,
    screen: &str,
    positions: &[usize],
    style: &OverlayStyle,
    options: &JumpOptions,
) -> Result<Option<usize>> {
    if positions.is_empty() {
        return Ok(None);
    }
    if options.auto_jump && positions.len() == 1 {
        return Ok(Some(0));
    }

    let labels = labels_for(positions.len(), &options.label_keys);
    let label_len = labels[0].chars().count();
    draw_overlay(pane, screen, positions, &labels[..positions.len()], style)?;
    let Some(input) = prompt_for_label_char("char:")? else {
        return Ok(None);
    };

    let Some(key_index) = options.label_keys.iter().position(|key| *key == input) else {
        return Ok(None);
    };

    if label_len == 1 {
        return Ok((key_index < positions.len()).then_some(key_index));
    }

    let Some(subset) = bounded_subset_bounds(
        key_index,
        label_len,
        positions.len(),
        options.label_keys.len(),
    ) else {
        return Ok(None);
    };
    let remaining = &positions[subset.clone()];
    let lower_index = select_position_index(pane, screen, remaining, style, options)?;
    Ok(lower_index.map(|value| subset.start + value))
}
