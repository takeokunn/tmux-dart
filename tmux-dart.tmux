#!/usr/bin/env bash

set -eu

CURRENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

get_tmux_option() {
  local option="$1"
  local default_value="$2"
  local option_value

  option_value="$(tmux show-option -gqv "$option")"
  if [ -z "$option_value" ]; then
    printf '%s' "$default_value"
  else
    printf '%s' "$option_value"
  fi
}

build_binary() {
  local bin="$CURRENT_DIR/result/bin/tmux-dart"

  if ! command -v nix >/dev/null 2>&1; then
    tmux display-message "tmux-dart: nix is required to build the plugin"
    exit 1
  fi

  nix build --out-link "$CURRENT_DIR/result" "path:$CURRENT_DIR#default" >/dev/null
  if [ ! -x "$bin" ]; then
    tmux display-message "tmux-dart: failed to build plugin binary"
    exit 1
  fi
}

ensure_binary() {
  if [ -n "${TMUX_DART_BINARY:-}" ]; then
    printf '%s' "$TMUX_DART_BINARY"
    return
  fi
  local bin="$CURRENT_DIR/result/bin/tmux-dart"
  if [ ! -x "$bin" ]; then
    build_binary
  fi
  printf '%s' "$bin"
}

export_jump_env() {
  export JUMP_BACKGROUND_COLOR="$(get_tmux_option '@jump-bg-color' '\e[0m\e[32m')"
  export JUMP_FOREGROUND_COLOR="$(get_tmux_option '@jump-fg-color' '\e[1m\e[31m')"
  export JUMP_THEME="$(get_tmux_option '@jump-theme' 'classic')"
  export JUMP_KEYS_POSITION="$(get_tmux_option '@jump-keys-position' 'left')"
  export JUMP_LABEL_KEYS="$(get_tmux_option '@jump-label-keys' 'jfhgkdlsa')"
  export JUMP_MATCH_MODE="$(get_tmux_option '@jump-match-mode' 'word')"
  export JUMP_CASE_SENSITIVE="$(get_tmux_option '@jump-case-sensitive' 'off')"
  export JUMP_AUTO_JUMP="$(get_tmux_option '@jump-auto-jump' 'on')"
}

run_tmux_dart_jump() {
  local bin

  export_jump_env
  bin="$(ensure_binary)"
  "$bin" jump "$@"
}

run_jump_with_char() {
  if [ "$#" -ne 1 ]; then
    tmux display-message "tmux-dart: missing jump character"
    exit 1
  fi
  if [ -z "$1" ]; then
    exit 0
  fi

  run_tmux_dart_jump --char "$1"
}

run_jump_with_char_file() {
  if [ "$#" -ne 1 ] || [ -z "$1" ]; then
    tmux display-message "tmux-dart: missing jump character file"
    exit 1
  fi

  run_tmux_dart_jump --char-file "$1"
  rm -f -- "$1"
}

char_file_path() {
  local dir="${TMPDIR:-/tmp}/tmux-dart-$(id -u)"
  local server_pid

  mkdir -p -m 700 "$dir"
  chmod 700 "$dir"
  # Scope the file to this tmux server: concurrent servers (or test runs)
  # sharing one path would overwrite and delete each other's characters.
  server_pid="$(tmux display-message -p '#{pid}')"
  printf '%s' "$dir/jump-char-$server_pid"
}

install_key_binding() {
  local jump_key
  local char_file

  jump_key="$(get_tmux_option "@jump-key" "j")"
  char_file="$(char_file_path)"
  # The typed character travels through tmux buffers, never through a shell or
  # a format expansion: `%%%` backslash-escapes `"` `\` `$` `;` `~` for the
  # double-quoted re-parse, set-buffer/save-buffer write the raw byte(s) to the
  # file, and only then does run-shell start this script with the (safe) file
  # path. Substituting the response directly into a shell command broke on
  # quotes, semicolons, and hashes.
  tmux bind-key -N "Jump to pane location in copy mode" "$jump_key" \
    command-prompt -1 -p 'char:' \
    "set-buffer -b tmux-dart-char -- \"%%%\" ; save-buffer -b tmux-dart-char '$char_file' ; delete-buffer -b tmux-dart-char ; run-shell -b \"'$CURRENT_DIR/tmux-dart.tmux' --char-file '$char_file'\""
}

case "${1:-}" in
  --char)
    run_jump_with_char "${2:-}"
    ;;
  --char-file)
    run_jump_with_char_file "${2:-}"
    ;;
  "")
    if [ -z "${TMUX_DART_BINARY:-}" ]; then
      build_binary
    fi
    install_key_binding
    ;;
  *)
    tmux display-message "tmux-dart: unexpected arguments"
    exit 1
    ;;
esac
