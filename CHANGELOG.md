# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- Jumps no longer drift inside TUIs (vim splits, htop, anything with a fixed header/footer). Such apps narrow the terminal scroll region (`DECSTBM`), which `ESC[2J` does not clear; painting the multi-match overlay and restoring the screen then scrolled inside that stale region and shifted every row. The overlay and restore now reset the scroll region (`ESC[r`) before repainting, and the restore homes the cursor explicitly instead of relying on the overlay having left it there.
- Jumps now land on the correct cell when a line contains wide (East Asian) characters: copy-mode `cursor-right` is driven by a grid-cell count, while overlay labels keep using display columns.
- Jumps now land on the correct cell when a line contains multi-character grid cells: NFD combining marks (macOS `ls` prints "ぎ" as き + ゙), ZWJ emoji sequences (👨‍👩‍👧), variation selectors (☝️), flag pairs (🇯🇵), and skin-tone modifiers (👍🏻). tmux stores each such grapheme cluster in a single cell, so counting characters overshot the line end and wrapped the cursor onto a following row. Columns are now measured in grapheme-cluster cells for navigation and in whole-cluster display width for overlay labels (including right-positioned labels, whose width can come from a variation selector rather than the base character alone).
- Jumps no longer drift downward when the pane keeps producing output (`tail -f`, busy TUIs on the normal screen): new lines push the captured region into history between capture and navigation, so the jump now measures the history growth and compensates the copy-mode scroll distance accordingly. The capture itself is re-anchored the same way.
- In default `word` mode, NFD combining marks no longer surface a bogus word start after every mark: zero-width characters continue their base character's word.
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
