use std::{env, process::ExitCode};

use anyhow::{Context, Result, bail, ensure};

use tmux_dart::{
    config::JumpConfig,
    flow::{JumpRequest, run_jump},
    tmux::{RealTmux, read_initial_char_from_file},
};

const JUMP_USAGE: &str =
    "usage: tmux-dart jump (--char <char> | --char-file <path>) [--pane-id <pane>]";
const TMUX_DART_HELP: &str = "\
tmux-dart

Usage:
    tmux-dart jump (--char <char> | --char-file <path>) [--pane-id <pane>]

Options:
    --char <char>       Set the requested character. Exactly one Unicode scalar.
    --char-file <path>  Read the requested character from a file. One character only.
    --pane-id <pane>    Target a specific tmux pane instead of the current pane.
    -h, --help          Show this help text.

Examples:
    tmux-dart jump --char x
    tmux-dart jump --char-file /tmp/jump-char
    tmux-dart jump --char x --pane-id %12
";

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
    run_with_args(env::args().skip(1))
}

fn run_with_args<I>(args: I) -> Result<()>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    match args.next().as_deref() {
        Some("--help") | Some("-h") => {
            print_help();
            Ok(())
        }
        Some("help") => {
            print_help();
            Ok(())
        }
        Some("jump") => {
            let mut jump_char: Option<char> = None;
            let mut jump_char_file: Option<String> = None;
            let mut explicit_pane_id: Option<String> = None;

            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--help" | "-h" => {
                        print_help();
                        return Ok(());
                    }
                    "help" => {
                        print_help();
                        return Ok(());
                    }
                    "--char" => {
                        let value = args.next().context("missing value for --char")?;
                        jump_char = Some(parse_jump_char(value)?);
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

fn print_help() {
    println!("{TMUX_DART_HELP}");
}

fn parse_jump_char(value: String) -> Result<char> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        bail!("--char requires exactly one character");
    };
    ensure!(
        chars.next().is_none(),
        "--char requires exactly one character"
    );
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::{parse_jump_char, run_with_args};

    #[test]
    fn parse_jump_char_accepts_single_character() {
        assert!(matches!(parse_jump_char("Z".to_owned()), Ok('Z')));
    }

    #[test]
    fn parse_jump_char_accepts_single_unicode_scalar() {
        assert!(matches!(parse_jump_char("界".to_owned()), Ok('界')));
    }

    #[test]
    fn parse_jump_char_rejects_empty_input() {
        assert!(parse_jump_char(String::new()).is_err());
    }

    #[test]
    fn parse_jump_char_rejects_multiple_characters() {
        assert!(parse_jump_char("ab".to_owned()).is_err());
    }

    #[test]
    fn parse_jump_char_allows_whitespace_characters() {
        assert!(matches!(parse_jump_char(" ".to_owned()), Ok(' ')));
    }

    #[test]
    fn run_with_args_prints_usage_for_help() {
        let result = run_with_args(["--help".to_owned()]);
        assert!(result.is_ok());
    }

    #[test]
    fn run_with_args_accepts_jump_help() {
        let result = run_with_args(["jump".to_owned(), "--help".to_owned()]);
        assert!(result.is_ok());
    }

    #[test]
    fn run_with_args_accepts_top_level_help_subcommand() {
        let result = run_with_args(["help".to_owned()]);
        assert!(result.is_ok());
    }
}
