use std::{env, process::ExitCode};

use anyhow::{Context, Result, bail};

use tmux_dart::{
    config::JumpConfig,
    flow::{JumpRequest, run_jump},
    tmux::{RealTmux, read_initial_char_from_file},
};

const JUMP_USAGE: &str =
    "usage: tmux-dart jump (--char <char> | --char-file <path>) [--pane-id <pane>]";

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
            let tmux = RealTmux;
            run_jump(
                &tmux,
                JumpRequest {
                    initial_char: jump_char,
                    pane_id: explicit_pane_id,
                    config: JumpConfig::from_env(),
                },
            )?;
            Ok(())
        }
        _ => bail!("{JUMP_USAGE}"),
    }
}
