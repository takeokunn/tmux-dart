# tmux-dart

Rust rewrite of `tmux-jump`: an EasyMotion-like cursor jump plugin for tmux.

![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)

## Features

- tmux plugin entrypoint via `tmux-dart.tmux`
- prompts for a leading character
- scans the visible pane for word starts matching that character
- overlays EasyMotion-style labels using `jfhgkdlsa`
- supports recursive multi-key label selection
- supports configurable label keys, match mode, case sensitivity, and auto-jump
- jumps using tmux copy-mode commands
- supports tmux options for key binding, colors, label placement, and matching behavior

## Prerequisites

[Nix](https://nixos.org/download) with [flakes enabled](https://nixos.wiki/wiki/Flakes#Enable_flakes) is required. The plugin automatically builds the binary on first load via `nix build`.

## Install

Clone the repository:

```sh
git clone https://github.com/takeokunn/tmux-dart ~/.tmux/plugins/tmux-dart
```

Then add to your `tmux.conf`:

```tmux
run-shell ~/.tmux/plugins/tmux-dart/tmux-dart.tmux
```

### TPM

```tmux
set -g @plugin 'takeokunn/tmux-dart'
```

## Configuration

Optional tmux settings:

```tmux
set -g @jump-key 'j'
set -g @jump-bg-color '\e[0m\e[32m'
set -g @jump-fg-color '\e[1m\e[31m'
set -g @jump-keys-position 'left'
set -g @jump-label-keys 'jfhgkdlsa'
set -g @jump-match-mode 'word'
set -g @jump-case-sensitive 'off'
set -g @jump-auto-jump 'on'
```

`@jump-match-mode` accepts `word` (word starts), `char` (all matching
characters), and `line` (first non-blank character on each matching line).
`@jump-keys-position` accepts `left` and `off_left`. `@jump-label-keys` needs at
least two unique non-whitespace characters; otherwise tmux-dart falls back to
`jfhgkdlsa`.

## Build

```bash
nix develop
tmux-dart-check
```

`nix run .#check` runs the same Nix verification without entering the shell. Use
`nix run .#smoke` for only the tmux smoke test, and `nix flake check` for
sandboxed checks.

## License

MIT -- see [LICENSE](LICENSE) for details.
