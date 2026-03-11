{
  description = "Development shell for Bun + TypeScript + Pkl";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    cli.url = "path:./cli";
    cli.inputs.nixpkgs.follows = "nixpkgs";
    cli.inputs.flake-utils.follows = "flake-utils";
    cli.inputs.rust-overlay.follows = "rust-overlay";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
      cli,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rustfmt"
            "clippy"
          ];
        };

        syncOpencodeConfigApp = pkgs.writeShellApplication {
          name = "sync-opencode-config";
          runtimeInputs = [
            pkgs.coreutils
            pkgs.diffutils
            pkgs.git
            pkgs.pkl
            pkgs.rsync
          ];
          text = builtins.readFile ./scripts/sync-opencode-config.sh;
        };

        pklCheckGeneratedApp = pkgs.writeShellApplication {
          name = "pkl-check-generated";
          runtimeInputs = [
            pkgs.git
            pkgs.nix
          ];
          text = ''
            set -euo pipefail

            repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
            if [ -z "''${repo_root}" ]; then
              repo_root="$(pwd)"
            fi

            exec nix develop "''${repo_root}" -c "''${repo_root}/config/pkl/check-generated.sh"
          '';
        };

        tokenCountWorkflowsApp = pkgs.writeShellApplication {
          name = "token-count-workflows";
          runtimeInputs = [
            pkgs.git
            pkgs.nix
          ];
          text = ''
            set -euo pipefail

            usage() {
              cat <<'EOF'
            Usage: nix run .#token-count-workflows [-- --help]

            Deterministic flake entrypoint for workflow token counting.
            Runs evals/token-count-workflows.ts through the existing evals Bun runtime.
            EOF
            }

            case "''${1:-}" in
              -h|--help)
                usage
                exit 0
                ;;
            esac

            repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
            if [ -z "''${repo_root}" ]; then
              repo_root="$(pwd)"
            fi

            evals_dir="''${repo_root}/evals"
            if [ ! -d "''${evals_dir}" ]; then
              cat >&2 <<EOF
            Could not locate evals directory at:
              ''${evals_dir}
            Run this command from the repository (or inside a git worktree rooted there).
            EOF
              exit 1
            fi

            exec nix develop "''${repo_root}" -c sh -c "cd \"''${evals_dir}\" && exec bun run token-count-workflows"
          '';
        };

        agnixLspShim = pkgs.writeShellScriptBin "agnix-lsp" ''
          set -euo pipefail

          if [ -n "''${AGNIX_LSP_BIN:-}" ] && [ -x "''${AGNIX_LSP_BIN}" ]; then
            exec "''${AGNIX_LSP_BIN}" "$@"
          fi

          if [ -x "$HOME/.cargo/bin/agnix-lsp" ]; then
            exec "$HOME/.cargo/bin/agnix-lsp" "$@"
          fi

          cat >&2 <<'EOF'
          agnix-lsp is not bundled in nixpkgs for this dev shell yet.

          Manual fallback (non-automatic):
            cargo install --locked agnix-lsp

          Then either:
            - ensure ~/.cargo/bin is on PATH, or
            - set AGNIX_LSP_BIN to the agnix-lsp binary path.
          EOF
          exit 1
        '';

      in
      {
        checks.cli-setup-command-surface = cli.checks.${system}.cli-setup-command-surface;
        checks.cli-clippy = cli.checks.${system}.cli-clippy;
        checks.sce-package = cli.packages.${system}.default;

        apps.sync-opencode-config = {
          type = "app";
          program = "${syncOpencodeConfigApp}/bin/sync-opencode-config";
          meta = {
            description = "Regenerate and sync config/.opencode outputs";
          };
        };

        apps.pkl-check-generated = {
          type = "app";
          program = "${pklCheckGeneratedApp}/bin/pkl-check-generated";
          meta = {
            description = "Run generated-output drift check in dev shell";
          };
        };

        apps.token-count-workflows = {
          type = "app";
          program = "${tokenCountWorkflowsApp}/bin/token-count-workflows";
          meta = {
            description = "Run static workflow token counting via evals Bun runtime";
          };
        };

        devShells.default = pkgs.mkShell {
          packages =
            with pkgs;
            [
              bun
              jq
              pkl
              typescript
              nodePackages.typescript-language-server
              agnixLspShim
            ]
            ++ [ rustToolchain ];

          shellHook = ''
            version_of() {
              "$1" --version 2>/dev/null | awk 'match($0, /[0-9]+(\.[0-9]+)+/) { print substr($0, RSTART, RLENGTH); exit }'
            }

            export PATH="$HOME/.cargo/bin:$PATH"

            if [ ! -x "$HOME/.cargo/bin/agnix" ]; then
              echo "- agnix: installing agnix-cli via cargo"
              cargo install --locked agnix-cli
            fi

            echo "- bun: $(version_of bun)"
            echo "- pkl: $(version_of pkl)"
            echo "- tsc: $(version_of tsc)"
            echo "- tsserver-lsp: $(version_of typescript-language-server)"
            echo "- rust: $(version_of rustc)"
            echo "- agnix: $(version_of agnix)"
            echo "- sync-opencode-config: nix run .#sync-opencode-config"
            echo "- sync-opencode-config help: nix run .#sync-opencode-config -- --help"
            echo "- pkl-check-generated: nix run .#pkl-check-generated"
          '';
        };
      }
    );
}
