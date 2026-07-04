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
      forAllSystemsWith = f: forAllSystems (system: f nixpkgs.legacyPackages.${system});
      packageFor =
        pkgs:
        pkgs.rustPlatform.buildRustPackage {
          pname = "tmux-dart";
          version = "0.1.0";
          src = self;
          cargoLock = {
            lockFile = ./Cargo.lock;
          };
        };
    in
    {
      formatter = forAllSystemsWith (pkgs:
        pkgs.writeShellApplication {
          name = "fmt";
          runtimeInputs = with pkgs; [
            cargo
            nixfmt
            rustfmt
          ];
          text = ''
            nixfmt "$@"
            cargo fmt
          '';
        });

      checks = forAllSystemsWith (pkgs: {
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
        clippy =
          (packageFor pkgs).overrideAttrs (_old: {
            pname = "tmux-dart-clippy";
            nativeBuildInputs = (_old.nativeBuildInputs or [ ]) ++ [ pkgs.clippy ];
            doCheck = false;
            buildPhase = ''
              cargo clippy --all-targets -- -D warnings
            '';
            installPhase = ''
              touch $out
            '';
          });
        package = packageFor pkgs;
      });

      devShells = forAllSystemsWith (pkgs: {
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
          ];
          shellHook = ''
            cat <<'USAGE_EOF'

            === tmux-dart Development Shell ===

            Quick verification:
              nix flake check  # bash syntax + fmt + clippy + package build/tests

            Plugin testing:
              # No pre-build needed; tmux-dart.tmux runs nix build on plugin load.
              tmux -L tmux-dart-clean -f /dev/null new-session -d \; run-shell "$(pwd)/tmux-dart.tmux" \; attach-session
              # Press prefix + j inside tmux. Cleanup from another shell when done:
              tmux -L tmux-dart-clean kill-server

            USAGE_EOF
          '';
        };
      });

      packages = forAllSystemsWith (pkgs: {
        default = packageFor pkgs;
      });

      apps = forAllSystemsWith (pkgs: {
        default = {
          type = "app";
          program = "${packageFor pkgs}/bin/tmux-dart";
          meta.description = "Run tmux-dart";
        };
      });
    };
}
