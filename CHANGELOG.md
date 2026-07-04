# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- Jumps now land on the correct cell when a line contains wide (East Asian) characters: copy-mode `cursor-right` is driven by a character count, while overlay labels keep using display columns.
- In default `word` mode, non-word targets (punctuation, symbols, whitespace such as `/`) now match every occurrence instead of reporting no match.
- `--char` now rejects empty or multi-character values instead of truncating them silently.
- `--char-file` now requires a single character and tolerates a trailing newline instead of truncating multi-character content.

### Added

- Overlay theme presets via `@jump-theme` for `classic`, `contrast`, and `soft`
- `@jump-keys-position` support for `left`, `right`, and `off_left`
- Prompt text that reflects the number of matches and label depth before selection begins

## [0.1.0] - 2026-05-18

### Added

- EasyMotion-like cursor jump plugin for tmux
- Implemented in Rust for performance
- Nix build and development shell with `nix develop` and `nix flake check`
- Three matching modes: `word`, `char`/`anywhere`, and `line`/`line_start`
- Recursive multi-key label selection for many search matches
- Auto-jump for single matches (configurable via `@jump-auto-jump`)
- Customizable label keys (`@jump-label-keys`), colors (`@jump-bg-color`, `@jump-fg-color`), and key positions (`@jump-keys-position`)
- `TMUX_DART_BINARY` environment variable support for using a pre-built binary
- Case-sensitive matching option (`@jump-case-sensitive`)
- Unit test coverage with `FakeBackend` mock implementing the `TmuxBackend` trait

[Unreleased]: https://github.com/takeokunn/tmux-dart/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/takeokunn/tmux-dart/releases/tag/v0.1.0
