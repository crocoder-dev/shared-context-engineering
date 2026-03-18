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

        pklParityCheck =
          pkgs.runCommand "pkl-parity-check"
            {
              nativeBuildInputs = [
                pkgs.git
                pkgs.pkl
              ];
            }
            ''
              set -euo pipefail

              # Copy source files
              cp -r "${./.}" ./repo
              chmod -R u+w ./repo
              cd ./repo

              tmp_dir="$(mktemp -d)"
              cleanup() {
                rm -rf "$tmp_dir"
              }
              trap cleanup EXIT

              pkl eval -m "$tmp_dir" config/pkl/generate.pkl >/dev/null

              paths=(
                "config/.opencode/agent"
                "config/.opencode/command"
                "config/.opencode/skills"
                "config/.opencode/lib/drift-collectors.js"
                "config/automated/.opencode/agent"
                "config/automated/.opencode/command"
                "config/automated/.opencode/skills"
                "config/automated/.opencode/lib/drift-collectors.js"
                "config/.claude/agents"
                "config/.claude/commands"
                "config/.claude/skills"
                "config/.claude/lib/drift-collectors.js"
                "config/schema/sce-config.schema.json"
                "config/.mcp.json"
              )

              stale=0
              for path in "''${paths[@]}"; do
                if [ -d "$tmp_dir/$path" ] && [ -d "$path" ]; then
                  if ! git diff --no-index --exit-code -- "$tmp_dir/$path" "$path" >/dev/null 2>&1; then
                    stale=1
                    printf 'Generated output drift detected at %s\n' "$path"
                    git diff --no-index -- "$tmp_dir/$path" "$path" || true
                  fi
                fi
              done

              if [[ "$stale" -ne 0 ]]; then
                cat <<'EOF'
              Generated files are stale.

              Regenerate with:
                nix develop -c pkl eval -m . config/pkl/generate.pkl
              EOF
                exit 1
              fi

              printf 'Generated outputs are up to date.\n'
              mkdir -p "$out"
            '';

      in
      {
        checks = {
          cli-tests = cli.checks.${system}.cli-tests;
          cli-clippy = cli.checks.${system}.cli-clippy;
          cli-fmt = cli.checks.${system}.cli-fmt;
          pkl-parity = pklParityCheck;
        };

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

        devShells.default = pkgs.mkShell {
          packages =
            with pkgs;
            [
              bun
              jq
              pkl
              typescript
              nodePackages.typescript-language-server
            ]
            ++ [ rustToolchain ];

          shellHook = ''
            version_of() {
              "$1" --version 2>/dev/null | awk 'match($0, /[0-9]+(\.[0-9]+)+/) { print substr($0, RSTART, RLENGTH); exit }'
            }

            export PATH="$HOME/.cargo/bin:$PATH"

            echo "- bun: $(version_of bun)"
            echo "- pkl: $(version_of pkl)"
            echo "- tsc: $(version_of tsc)"
            echo "- tsserver-lsp: $(version_of typescript-language-server)"
            echo "- rust: $(version_of rustc)"
            echo "- sync-opencode-config: nix run .#sync-opencode-config"
            echo "- sync-opencode-config help: nix run .#sync-opencode-config -- --help"
            echo "- pkl-check-generated: nix run .#pkl-check-generated"
          '';
        };
      }
    );
}
