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
          text = ''
            set -euo pipefail

            usage() {
              cat <<'EOF'
            Usage: nix run .#sync-opencode-config [-- --help]

            Deterministic flake entrypoint for opencode config sync workflow.

            Current scope:
            - Regenerate generated config outputs in a staging workspace.
            - Replace repository config/ only after successful staged regeneration.
            - Replace repository-root .opencode/ from staged config/.opencode/.
            - Exclude runtime artifacts during root sync (for example node_modules/).
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

            live_config="''${repo_root}/config"
            live_opencode="''${repo_root}/.opencode"
            generator_path="''${live_config}/pkl/generate.pkl"

            if [ ! -d "''${live_config}" ]; then
              cat >&2 <<EOF
            Could not locate live config directory at:
              ''${live_config}
            EOF
              exit 1
            fi

            if [ ! -f "''${generator_path}" ]; then
              cat >&2 <<EOF
            Could not locate config generator at:
              ''${generator_path}
            Run this command from the repository (or inside a git worktree rooted there).
            EOF
              exit 1
            fi

            stage_root="$(mktemp -d "''${TMPDIR:-/tmp}/sync-opencode-config.XXXXXX")"
            stage_config="''${stage_root}/config"
            stage_opencode="''${stage_config}/.opencode"
            stage_generator_path="''${stage_config}/pkl/generate.pkl"
            backup_config="''${repo_root}/.config-pre-sync-backup"
            backup_opencode="''${repo_root}/.opencode-pre-sync-backup"
            config_swap_complete=0
            opencode_swap_complete=0

            exclude_runtime_artifacts=(
              node_modules
            )

            cleanup() {
              if [ "''${opencode_swap_complete}" -ne 1 ] && [ -d "''${backup_opencode}" ]; then
                rm -rf "''${live_opencode}"
                mv "''${backup_opencode}" "''${live_opencode}"
              fi
              if [ "''${config_swap_complete}" -ne 1 ] && [ -d "''${backup_config}" ]; then
                rm -rf "''${live_config}"
                mv "''${backup_config}" "''${live_config}"
              fi
              rm -rf "''${stage_root}" "''${backup_config}" "''${backup_opencode}"
            }
            trap cleanup EXIT

            echo "==> Preparing staged config workspace"
            cp -R "''${live_config}" "''${stage_config}"

            echo "==> Regenerating generated-owned config outputs in staging"
            pkl eval -m "''${stage_root}" "''${stage_generator_path}"

            if [ ! -d "''${stage_config}/.opencode" ] || [ ! -d "''${stage_config}/.claude" ]; then
              cat >&2 <<EOF
            Staged regeneration is incomplete; refusing to replace live config/.
            Missing expected generated directories under:
              ''${stage_config}
            EOF
              exit 1
            fi

            if [ ! -d "''${stage_opencode}" ]; then
              cat >&2 <<EOF
            Staged regeneration is missing config/.opencode; refusing to replace root .opencode/.
            Missing directory:
              ''${stage_opencode}
            EOF
              exit 1
            fi

            if [ -e "''${backup_config}" ]; then
              rm -rf "''${backup_config}"
            fi

            echo "==> Replacing live config/ from staged output"
            mv "''${live_config}" "''${backup_config}"
            cp -R "''${stage_config}" "''${live_config}"
            rm -rf "''${backup_config}"
            config_swap_complete=1

            if [ -e "''${backup_opencode}" ]; then
              rm -rf "''${backup_opencode}"
            fi

            if [ -e "''${live_opencode}" ]; then
              echo "==> Replacing repository-root .opencode/ from staged config/.opencode/"
              mv "''${live_opencode}" "''${backup_opencode}"
            else
              echo "==> Creating repository-root .opencode/ from staged config/.opencode/"
            fi

            rm -rf "''${live_opencode}"
            mkdir -p "''${live_opencode}"

            rsync_excludes=()
            diff_excludes=()
            for entry in "''${exclude_runtime_artifacts[@]}"; do
              rsync_excludes+=(--exclude "''${entry}/")
              diff_excludes+=(-x "''${entry}")
            done

            rsync -a "''${rsync_excludes[@]}" "''${stage_opencode}/" "''${live_opencode}/"

            if ! diff -rq "''${diff_excludes[@]}" "''${stage_opencode}" "''${live_opencode}" >/dev/null; then
              cat >&2 <<EOF
            Root .opencode replacement verification failed.
            Source and target trees differ after copy.
            EOF
              exit 1
            fi

            rm -rf "''${backup_opencode}"
            opencode_swap_complete=1

            cat <<'EOF'
            Done: repository config/ has been regenerated in staging and replaced.
            Done: repository-root .opencode/ has been replaced from staged config/.opencode/.
            EOF
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

        apps.sync-opencode-config = {
          type = "app";
          program = "${syncOpencodeConfigApp}/bin/sync-opencode-config";
          meta = {
            description = "Regenerate and sync config/.opencode outputs";
          };
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            bun
            jq
            pkl
            typescript
            nodePackages.typescript-language-server
            agnixLspShim
            cargo
            rustc
          ];

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
          '';
        };
      }
    );
}
