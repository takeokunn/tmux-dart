# Contributing to tmux-dart

Welcome, and thanks for your interest in contributing to tmux-dart. This document covers how to set up your environment, follow the development workflow, and submit changes.

## Development Setup

### Prerequisites

Choose one of the following:

- **Nix with flakes enabled** (recommended). This gives you a fully reproducible development shell with all tools pinned.
- **Rust 1.94+** via [rustup](https://rustup.rs). The minimum supported Rust version is declared in `Cargo.toml` as `rust-version = "1.94"`. A `rust-toolchain.toml` file can be used to pin the toolchain locally.

### Setting up the environment

With Nix:

```bash
nix develop
```

This drops you into a shell with `cargo`, `clippy`, `rustfmt`, `rust-analyzer`, `bash`, and `tmux` pre-installed.

Without Nix, install Rust via rustup and ensure `cargo`, `clippy`, and `rustfmt` are available:

```bash
rustup component add clippy rustfmt
```

## Development Workflow

The following commands are the primary quality gates. Run them before pushing changes.

### Formatting

```bash
cargo fmt --check
```

Run `cargo fmt` to auto-fix formatting issues.

### Tests

```bash
cargo test
```

Unit tests are co-located with source code in `#[cfg(test)]` modules.

### Linting

```bash
cargo clippy -- -D warnings
```

The project enforces strict clippy rules (see Coding Conventions below).

### Bash syntax check

```bash
bash -n tmux-dart.tmux
```

The tmux plugin entrypoint is POSIX-compatible Bash. Syntax errors here prevent the plugin from loading.

### Full sandboxed verification

```bash
nix flake check
```

This runs `bash -n`, `cargo fmt --check`, and the package build (which includes `cargo test`) inside a Nix sandbox. This is the single command that verifies everything.

## Coding Conventions

### Rust edition

This project uses Rust edition 2024.

### Error handling

The following are **forbidden** and enforced by clippy:

- `unwrap()` (any form)
- `expect()`
- `panic!()` macros (including `todo!()`, `unreachable!()`, etc.)

Use `Result` with the `?` operator for error propagation. Use `anyhow::Result` in application code and define proper error types for library code.

### Testability: the `TmuxBackend` trait

All tmux interactions go through the `TmuxBackend` trait defined in `src/tmux.rs`. The production implementation is `RealTmux`, which calls `tmux` on the command line. Tests use a `FakeBackend` mock (defined in `src/flow.rs`) that returns canned responses.

When adding features that interact with tmux, extend `TmuxBackend` with new trait methods and implement them on both `RealTmux` and `FakeBackend`.

### Tmux plugin entrypoint

`tmux-dart.tmux` must remain POSIX-compatible Bash. Avoid bashisms like arrays, process substitution, and `[[ ]]`. The `bash -n` check in CI catches syntax errors.

## Testing

### Unit tests

Add or update tests for any behavior change. The `FakeBackend` pattern lets you test jump logic without a running tmux session. Tests are in `#[cfg(test)]` modules within each source file.

### Manual smoke tests

For changes that affect the UI (overlay drawing, label placement, cursor movement), test interactively inside tmux:

```bash
tmux -L tmux-dart-clean -f /dev/null new-session -d \; run-shell "$(pwd)/tmux-dart.tmux" \; attach-session
```

Press the configured jump key (default: `prefix + j`) and verify the behavior. Clean up afterward:

```bash
tmux -L tmux-dart-clean kill-server
```

## Pull Request Process

1. Use **conventional commit** prefixes for your commits and PR titles: `feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `ci:`, `chore:`.
2. Link any related issues in the PR description.
3. Ensure all checks pass before requesting review:
    - `cargo test`
    - `cargo fmt --check`
    - `cargo clippy -- -D warnings`
    - `bash -n tmux-dart.tmux`
4. Include tests for new behavior and note any manual smoke testing you performed.

## Reporting Issues

When reporting a bug, include:

- Your tmux version (`tmux -V`)
- Your OS and shell
- The relevant tmux configuration options (`@jump-*`)
- Steps to reproduce and expected vs. actual behavior
- Any error messages displayed by tmux or tmux-dart

Feature requests are welcome. Describe the use case and how you would expect the feature to work.

## License

tmux-dart is licensed under the MIT License. By contributing, you agree that your contributions will be licensed under the same terms. See [LICENSE](LICENSE) for details.
