{
  description = "tmux-dart development shell and package";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      pkgsFor = system: nixpkgs.legacyPackages.${system};
      packageFor =
        system:
        let
          pkgs = pkgsFor system;
        in
        pkgs.rustPlatform.buildRustPackage {
          pname = "tmux-dart";
          version = "0.1.0";
          src = self;
          cargoLock = {
            lockFile = ./Cargo.lock;
          };
        };
      smokeFor =
        system:
        let
          pkgs = pkgsFor system;
          tmuxDart = packageFor system;
        in
        pkgs.writeShellApplication {
          name = "tmux-dart-smoke";
          runtimeInputs = [
            pkgs.coreutils
            pkgs.tmux
          ];
          text = ''
            set -eu

            session="tmux_dart_qa_$$"
            cleanup() {
              tmux kill-session -t "$session" >/dev/null 2>&1 || true
            }
            trap cleanup EXIT

            tmux new-session -d -s "$session" "sh -lc 'printf \"qzxvword\\n\"; exec cat'"
            pane_id="$(tmux display-message -p -t "$session":0.0 '#{pane_id}')"

            content=""
            for _ in $(seq 1 40); do
              content="$(tmux capture-pane -p -t "$pane_id")"
              case "$content" in
                *qzxvword*) break ;;
              esac
              sleep 0.05
            done

            jump_char_file="$(mktemp)"
            trap 'rm -f "$jump_char_file"; cleanup' EXIT
            printf 'q' > "$jump_char_file"

            JUMP_BACKGROUND_COLOR='\e[0m\e[32m' \
            JUMP_FOREGROUND_COLOR='\e[1m\e[31m' \
            JUMP_KEYS_POSITION='left' \
              "${tmuxDart}/bin/tmux-dart" jump --char-file "$jump_char_file" --pane-id "$pane_id"

            state="$(tmux display-message -p -t "$pane_id" '#{pane_in_mode};#{cursor_x};#{cursor_y};#{scroll_position}')"
            content="$(tmux capture-pane -p -t "$pane_id")"

            printf 'STATE<<%s>>\nAFTER<<%s>>\n' "$state" "$content"

            if [ "$state" != '1;0;1;0' ]; then
              printf 'tmux-dart smoke failed: unexpected pane state\n' >&2
              exit 1
            fi

            case "$content" in
              *qzxvword*) ;;
              *)
                printf 'tmux-dart smoke failed: pane content missing qzxvword\n' >&2
                exit 1
                ;;
            esac
          '';
        };
      checkFor =
        system:
        let
          pkgs = pkgsFor system;
        in
        pkgs.writeShellApplication {
          name = "tmux-dart-check";
          runtimeInputs = [ pkgs.nix ];
          text = ''
            set -eu

            repo="''${1:-$PWD}"
            nix flake check "path:$repo"
            nix run "path:$repo#smoke"
          '';
        };
    in
    {
      formatter = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        pkgs.nixfmt
      );

      checks = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          default =
            pkgs.runCommand "tmux-dart-check"
              {
                nativeBuildInputs = [
                  pkgs.bash
                  pkgs.cargo
                  pkgs.rustfmt
                ];
                src = self;
              }
              ''
                cp -r $src/. .
                chmod -R u+w .
                bash -n tmux-dart.tmux
                cargo fmt --check
                touch $out
              '';
          package = packageFor system;
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              bash
              cargo
              clippy
              nixfmt
              pkg-config
              rust-analyzer
              rustc
              rustfmt
              tmux
              (checkFor system)
              (smokeFor system)
            ];
            shellHook = ''
                            cat <<'USAGE_EOF'

              === tmux-dart Development Shell ===

              Quick verification:
                tmux-dart-check  # nix flake check + tmux smoke
                tmux-dart-smoke  # tmux smoke test using the Nix-built binary
                nix flake check  # sandboxed shell syntax + fmt + package build/tests
                nix run .#check  # same as tmux-dart-check from outside nix develop
                nix run .#smoke  # same as tmux-dart-smoke from outside nix develop

              Plugin testing:
                # No pre-build needed; tmux-dart.tmux runs nix build on plugin load.
                tmux -L tmux-dart-clean -f /dev/null new-session -d \; run-shell "$(pwd)/tmux-dart.tmux" \; attach-session
                # Press prefix + j inside tmux. Cleanup from another shell when done:
                tmux -L tmux-dart-clean kill-server

              USAGE_EOF
            '';
          };
        }
      );

      packages = forAllSystems (system: {
        default = packageFor system;
        check = checkFor system;
        smoke = smokeFor system;
      });

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/tmux-dart";
          meta.description = "Run tmux-dart";
        };
        check = {
          type = "app";
          program = "${self.packages.${system}.check}/bin/tmux-dart-check";
          meta.description = "Run tmux-dart Nix verification";
        };
        smoke = {
          type = "app";
          program = "${self.packages.${system}.smoke}/bin/tmux-dart-smoke";
          meta.description = "Run tmux-dart tmux smoke test";
        };
      });
    };
}
