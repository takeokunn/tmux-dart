# tmux-dart

Rust rewrite of `tmux-jump`: an EasyMotion-like cursor jump plugin for tmux.

## Features

- tmux plugin entrypoint via `tmux-dart.tmux`
- prompts for a leading character
- scans the visible pane for word starts matching that character
- overlays EasyMotion-style labels using `jfhgkdlsa`
- supports recursive multi-key label selection
- supports configurable label keys, match mode, case sensitivity, and auto-jump
- jumps using tmux copy-mode commands
- supports tmux options for key binding, colors, label placement, and matching behavior

## Development

```bash
nix develop
tmux-dart-check
```

`nix run .#check` runs the same Nix verification without entering the shell. Use
`nix run .#smoke` for only the tmux smoke test, and `nix flake check` for
sandboxed checks.

## tmux setup

```tmux
run-shell /path/to/tmux-dart/tmux-dart.tmux
```

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
