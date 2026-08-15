# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.9] - 2026-08-15

### Fixed

- The key binding delivers the typed character again. The prompt template substituted the response through `#{q:%%%}`, which `run-shell` format-expanded to an empty string, so pressing the jump key silently did nothing. The response now travels through a tmux buffer (`set-buffer`/`save-buffer`) and reaches the plugin via `--char-file`, which also makes shell-hostile characters (`'`, `"`, `;`, `#`, `$`, `\`, space) jumpable.
- Label selection works again for multi-match jumps. The label prompt used the same broken `run-shell` substitution, so every multi-match jump hung for the full prompt timeout and then cancelled; it now uses the same buffer-based delivery.
- The label prompt no longer splits in two: `command-prompt -p` treats commas as a prompt-list separator, so the previous "jump key (N matches, depth D):" prompt swallowed one extra keypress per selection. The prompt now avoids commas.
- Answering the label prompt no longer races the session-activity cancellation check: after an activity change the prompt file is re-read for a short grace period, so a valid selection arriving milliseconds after the keypress is not discarded as a dismissal.
- The label prompt client is spawned instead of awaited, restoring the selection timeout: a client-less `command-prompt` blocks until answered, which previously kept the overlay up forever when the prompt was abandoned and then jumped against a long-stale capture on the next keypress.
- Submitting an empty prompt response with Enter (which tmux delivers as a lone carriage return) is treated as a cancellation instead of a control-character error.
- Each tmux server gets its own jump-character file, so concurrent servers no longer overwrite or delete each other's prompt input.
- Messages shown via `display-message` escape `#`, so a jump character like `#` cannot be format-expanded into garbage.
- When a jump starts from copy mode, the cursor position is re-read after leaving copy mode, so the alternate-screen restore parks the cursor where the application left it instead of on the copy-mode cursor.

### Added

- Real-tmux end-to-end coverage for the full key-binding flow (including shell-hostile characters) and for label selection and prompt dismissal, driven through a genuinely attached client.

## [0.1.8] - 2026-07-26

### Changed

- Label selection is recursive and accepts one key per prompt, restoring compatibility with tmux-jump muscle memory while retaining deterministic labels for large match sets.
- Copy-mode navigation is sent as one tmux command list instead of starting a subprocess for every movement, reducing movement process launches from as many as seven to three.
- Overlay rendering now reuses precomputed cells and reserved buffers, nearly doubling throughput in the mixed-Unicode render benchmark.
- Prompt polling reacts every 5 ms during the initial input window, then backs off progressively to avoid unnecessary idle wakeups.

### Fixed

- Prompt-file reads are bounded, reject non-regular files and invalid UTF-8, and do not block on FIFOs.
- Arithmetic around pane history, cursor movement, label generation, and overlay placement now handles extreme values without wrapping or panicking.
- Real-tmux regression coverage now verifies the recursive selection flow and command-list navigation.

## [0.1.7] - 2026-07-09

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

[Unreleased]: https://github.com/takeokunn/tmux-dart/compare/v0.1.8...HEAD
[0.1.8]: https://github.com/takeokunn/tmux-dart/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/takeokunn/tmux-dart/compare/v0.1.0...v0.1.7
[0.1.0]: https://github.com/takeokunn/tmux-dart/releases/tag/v0.1.0
