{
  description = "Development shell for Bun + TypeScript + Pkl";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
      crane,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        rustVersion = "1.93.1";

        rustToolchain = pkgs.rust-bin.stable.${rustVersion}.default.override {
          extensions = [
            "rustfmt"
            "clippy"
          ];
        };

        craneLib = (crane.mkLib pkgs).overrideToolchain (_: rustToolchain);

        workspaceRoot = ./.;
        workspaceSrc = pkgs.lib.fileset.toSource {
          root = workspaceRoot;
          fileset = pkgs.lib.fileset.unions [
            (craneLib.fileset.commonCargoSources workspaceRoot)
            (pkgs.lib.fileset.maybeMissing ./config/.opencode)
            (pkgs.lib.fileset.maybeMissing ./config/.claude)
            (pkgs.lib.fileset.maybeMissing ./config/schema/sce-config.schema.json)
            (pkgs.lib.fileset.maybeMissing ./cli/assets/hooks)
          ];
        };

        version = pkgs.lib.strings.trim (builtins.readFile ./.version);
        gitCommit =
          if self ? rev then
            self.rev
          else if self ? dirtyRev then
            self.dirtyRev
          else
            "unknown";
        shortGitCommit = builtins.substring 0 12 gitCommit;

        commonCargoArgs = {
          pname = "sce";
          inherit version;
          src = workspaceSrc;
          cargoToml = ./cli/Cargo.toml;
          cargoLock = ./cli/Cargo.lock;
          strictDeps = true;
          doCheck = false;
          SCE_GIT_COMMIT = shortGitCommit;

          nativeBuildInputs = [
            rustToolchain
          ];

          postUnpack = ''
            cd "$sourceRoot/cli"
            sourceRoot="."
          '';
        };

        cargoArtifacts = craneLib.buildDepsOnly (
          commonCargoArgs
          // {
            pname = "sce-deps";
          }
        );

        scePackage = craneLib.buildPackage (
          commonCargoArgs
          // {
            inherit cargoArtifacts;
          }
        );

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
              )

              stale=0
              for path in "''${paths[@]}"; do
                if [ -e "$tmp_dir/$path" ] || [ -e "$path" ]; then
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
        packages = {
          sce = scePackage;
          default = scePackage;
        };

        checks = {
          cli-tests = craneLib.cargoTest (
            commonCargoArgs
            // {
              pname = "sce-cli-tests";
              inherit cargoArtifacts;
              nativeCheckInputs = [ pkgs.git ];
            }
          );

          cli-clippy = craneLib.cargoClippy (
            commonCargoArgs
            // {
              pname = "sce-cli-clippy";
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets --all-features";
            }
          );

          cli-fmt = craneLib.cargoFmt (
            commonCargoArgs
            // {
              pname = "sce-cli-fmt";
            }
          );

          pkl-parity = pklParityCheck;
        };

        apps.sce = {
          type = "app";
          program = "${scePackage}/bin/sce";
          meta = {
            description = "Run the packaged sce CLI";
          };
        };

        apps.sync-opencode-config = {
          type = "app";
          program = "${syncOpencodeConfigApp}/bin/sync-opencode-config";
          meta = {
            description = "Regenerate config and sync root .opencode";
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
              scePackage
            ]
            ++ [ rustToolchain ];

          shellHook = ''
            version_of() {
              "$1" --version 2>/dev/null | awk 'match($0, /[0-9]+(\.[0-9]+)+/) { print substr($0, RSTART, RLENGTH); exit }'
            }

            echo "- bun: $(version_of bun)"
            echo "- pkl: $(version_of pkl)"
            echo "- tsc: $(version_of tsc)"
            echo "- tsserver-lsp: $(version_of typescript-language-server)"
            echo "- rust: $(version_of rustc)"
            echo "- sce: $(version_of sce)"
            echo "- sync-opencode-config: nix run .#sync-opencode-config"
            echo "- sync-opencode-config help: nix run .#sync-opencode-config -- --help"
            echo "- pkl-check-generated: nix run .#pkl-check-generated"
          '';
        };
      }
    );
}
