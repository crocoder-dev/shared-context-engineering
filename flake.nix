{
  description = "Shared Context Engineering repository and sce CLI flake";

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
            (pkgs.lib.fileset.maybeMissing ./.version)
            (pkgs.lib.fileset.maybeMissing ./cli/src/services/default_paths.rs)
            ./config
            (pkgs.lib.fileset.maybeMissing ./cli/assets/hooks)
            (pkgs.lib.fileset.maybeMissing ./scripts/prepare-cli-generated-assets.sh)
          ];
        };

        npmSrc = pkgs.lib.fileset.toSource {
          root = workspaceRoot;
          fileset = pkgs.lib.fileset.unions [
            ./npm/README.md
            ./npm/package.json
            ./npm/bin/sce.js
            ./npm/lib/install.js
            ./npm/lib/platform.js
            ./npm/lib/release-manifest-public-key.pem
            ./npm/test/install.test.js
            ./npm/test/platform.test.js
            ./scripts/lib/release-manifest-signing.mjs
          ];
        };

        configLibBashPolicySrc = pkgs.lib.fileset.toSource {
          root = ./config/lib/bash-policy-plugin;
          fileset = pkgs.lib.fileset.unions [
            ./config/lib/bash-policy-plugin/package.json
            ./config/lib/bash-policy-plugin/bun.lock
            ./config/lib/bash-policy-plugin/bash-policy/runtime.ts
            ./config/lib/bash-policy-plugin/bash-policy-runtime.test.ts
            ./config/lib/bash-policy-plugin/opencode-bash-policy-plugin.ts
          ];
        };

        integrationsInstallSrc = pkgs.lib.fileset.toSource {
          root = workspaceRoot;
          fileset = pkgs.lib.fileset.unions [
            ./integrations/install/Cargo.toml
            ./integrations/install/Cargo.lock
            ./integrations/install/src
          ];
        };

        # Fixed-output derivation to fetch Bun dependencies
        # The output hash must be updated when package.json or bun.lock changes
        configLibBashPolicyDeps = pkgs.stdenv.mkDerivation {
          pname = "config-lib-bash-policy-deps";
          version = "0.1.0";
          src = pkgs.lib.fileset.toSource {
            root = ./config/lib/bash-policy-plugin;
            fileset = pkgs.lib.fileset.unions [
              ./config/lib/bash-policy-plugin/package.json
              ./config/lib/bash-policy-plugin/bun.lock
            ];
          };
          nativeBuildInputs = [ pkgs.bun ];
          dontBuild = true;
          installPhase = ''
            bun install --frozen-lockfile --no-progress
            # Remove Bun's cache symlinks that point to build directory
            rm -rf node_modules/.cache
            mkdir -p $out
            cp -r node_modules $out/
          '';
          outputHashMode = "recursive";
          outputHashAlgo = "sha256";
          outputHash = "sha256-4OPHPfR0vHIRsAflr/EFddsMkR5ZnRaN9MLrdJDqTJY=";
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
            mkdir -p "$sourceRoot/cli/assets/generated/config"
            cp -R ${./config/.opencode} "$sourceRoot/cli/assets/generated/config/opencode"
            cp -R ${./config/.claude} "$sourceRoot/cli/assets/generated/config/claude"
            mkdir -p "$sourceRoot/cli/assets/generated/config/schema"
            cp ${./config/schema/sce-config.schema.json} "$sourceRoot/cli/assets/generated/config/schema/sce-config.schema.json"

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

        integrationsInstallCargoArgs = {
          pname = "sce-install-channel-integration-tests";
          version = "0.1.0";
          src = integrationsInstallSrc;
          cargoToml = ./integrations/install/Cargo.toml;
          cargoLock = ./integrations/install/Cargo.lock;
          strictDeps = true;
          doCheck = false;

          nativeBuildInputs = [
            rustToolchain
          ];

          postUnpack = ''
            cd "$sourceRoot/integrations/install"
            sourceRoot="."
          '';
        };

        integrationsInstallCargoArtifacts = craneLib.buildDepsOnly (
          integrationsInstallCargoArgs
          // {
            pname = "sce-install-channel-integration-tests-deps";
          }
        );

        integrationsInstallPackage = craneLib.buildPackage (
          integrationsInstallCargoArgs
          // {
            cargoArtifacts = integrationsInstallCargoArtifacts;
            meta = {
              mainProgram = "install-channel-integration-tests";
              description = "Opt-in install-channel integration runner for sce";
            };
          }
        );

        scePackage = craneLib.buildPackage (
          commonCargoArgs
          // {
            inherit cargoArtifacts;
            meta = {
              mainProgram = "sce";
              description = "Shared Context Engineering CLI";
            };
          }
        );

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

        releaseArtifactsApp = pkgs.writeShellApplication {
          name = "release-artifacts";
          runtimeInputs = [
            pkgs.coreutils
            pkgs.gnutar
            pkgs.gzip
            pkgs.jq
            pkgs.nix
            pkgs.nodejs
          ];
          text = ''
            set -euo pipefail

            usage() {
              cat <<'EOF'
            Usage: release-artifacts --version <semver> --out-dir <path>

            Builds the canonical current-platform `sce` release archive via Nix and writes
            the archive, checksum, and metadata manifest fragment into the output directory.
            EOF
            }

            version=""
            out_dir=""

            while [[ $# -gt 0 ]]; do
              case "$1" in
                --version)
                  version="''${2:-}"
                  shift 2
                  ;;
                --out-dir)
                  out_dir="''${2:-}"
                  shift 2
                  ;;
                --help|-h)
                  usage
                  exit 0
                  ;;
                *)
                  printf 'Unknown argument: %s\n\n' "$1" >&2
                  usage >&2
                  exit 1
                  ;;
              esac
            done

            if [[ -z "$version" || -z "$out_dir" ]]; then
              usage >&2
              exit 1
            fi

            if [[ ! -f "flake.nix" ]]; then
              printf 'release-artifacts must be run from the repository root that contains flake.nix\n' >&2
              exit 1
            fi

            checked_in_version="$(tr -d '\n' < .version)"
            if [[ "$version" != "$checked_in_version" ]]; then
              printf 'Requested release version %s does not match checked-in .version %s\n' "$version" "$checked_in_version" >&2
              exit 1
            fi

            cargo_version="$(sed -n 's/^version = "\([^"]*\)"$/\1/p' cli/Cargo.toml | head -n 1)"
            npm_version="$(${pkgs.nodejs}/bin/node -p "JSON.parse(require('fs').readFileSync('npm/package.json', 'utf8')).version")"

            if [[ "$cargo_version" != "$version" ]]; then
              printf 'cli/Cargo.toml version %s does not match release version %s\n' "$cargo_version" "$version" >&2
              exit 1
            fi

            if [[ "$npm_version" != "$version" ]]; then
              printf 'npm/package.json version %s does not match release version %s\n' "$npm_version" "$version" >&2
              exit 1
            fi

            normalize_arch() {
              case "$1" in
                x86_64|amd64)
                  printf 'x86_64'
                  ;;
                arm64|aarch64)
                  printf 'aarch64'
                  ;;
                *)
                  printf '%s' "$1"
                  ;;
              esac
            }

            detect_target_triple() {
              local os="$1"
              local arch="$2"

              case "$os:$arch" in
                Linux:x86_64)
                  printf 'x86_64-unknown-linux-gnu'
                  ;;
                Linux:aarch64)
                  printf 'aarch64-unknown-linux-gnu'
                  ;;
                Darwin:aarch64)
                  printf 'aarch64-apple-darwin'
                  ;;
                *)
                  printf 'Unsupported release target for os=%s arch=%s\n' "$os" "$arch" >&2
                  exit 1
                  ;;
              esac
            }

            os_name="$(uname -s)"
            arch_name="$(normalize_arch "$(uname -m)")"
            target_triple="$(detect_target_triple "$os_name" "$arch_name")"
            archive_name="sce-v''${version}-''${target_triple}.tar.gz"
            checksum_name="''${archive_name}.sha256"
            manifest_name="sce-v''${version}-''${target_triple}.json"
            archive_root="sce-v''${version}-''${target_triple}"

            mkdir -p "$out_dir"

            nix build .#default --out-link result

            binary_path="result/bin/sce"
            if [[ ! -x "$binary_path" ]]; then
              printf 'Expected built CLI binary at %s\n' "$binary_path" >&2
              exit 1
            fi

            binary_version="$($binary_path version --format json | ${pkgs.jq}/bin/jq -r '.version')"
            if [[ "$binary_version" != "$version" ]]; then
              printf 'Built CLI version %s does not match release version %s\n' "$binary_version" "$version" >&2
              exit 1
            fi

            tmp_dir="$(mktemp -d)"
            cleanup() {
              rm -rf "$tmp_dir"
            }
            trap cleanup EXIT

            mkdir -p "$tmp_dir/$archive_root/bin"
            cp "$binary_path" "$tmp_dir/$archive_root/bin/sce"
            cp "LICENSE" "$tmp_dir/$archive_root/LICENSE"
            cp "README.md" "$tmp_dir/$archive_root/README.md"

            archive_path="$out_dir/$archive_name"
            checksum_path="$out_dir/$checksum_name"
            manifest_path="$out_dir/$manifest_name"

            tar \
              --sort=name \
              --mtime='UTC 1970-01-01' \
              --owner=0 \
              --group=0 \
              --numeric-owner \
              -C "$tmp_dir" \
              -cf - "$archive_root" | gzip -n > "$archive_path"

            checksum="$(sha256sum "$archive_path" | cut -d ' ' -f 1)"
            printf '%s  %s\n' "$checksum" "$archive_name" > "$checksum_path"

            jq \
              --null-input \
              --arg version "$version" \
              --arg archive "$archive_name" \
              --arg checksum_file "$checksum_name" \
              --arg checksum "$checksum" \
              --arg target_triple "$target_triple" \
              --arg os "''${os_name,,}" \
              --arg arch "$arch_name" \
              '{
                version: $version,
                binary: "sce",
                archive: $archive,
                checksum_file: $checksum_file,
                checksum_sha256: $checksum,
                target_triple: $target_triple,
                os: $os,
                arch: $arch
              }' > "$manifest_path"

            printf 'Built release artifacts:\n'
            printf '  %s\n' "$archive_path"
            printf '  %s\n' "$checksum_path"
            printf '  %s\n' "$manifest_path"
          '';
        };

        releaseManifestApp = pkgs.writeShellApplication {
          name = "release-manifest";
          runtimeInputs = [
            pkgs.coreutils
            pkgs.jq
            pkgs.nodejs
          ];
          text = ''
            set -euo pipefail

            usage() {
              cat <<'EOF'
            Usage: release-manifest --version <semver> --artifacts-dir <path> --out-dir <path> [--signing-key-file <path>]

            Merges per-platform `sce` release metadata fragments into a stable release
            manifest and combined SHA256SUMS file, then signs the manifest using
            `SCE_RELEASE_MANIFEST_SIGNING_KEY` or an explicit private key file.
            EOF
            }

            version=""
            artifacts_dir=""
            out_dir=""
            signing_key_file=""

            while [[ $# -gt 0 ]]; do
              case "$1" in
                --version)
                  version="''${2:-}"
                  shift 2
                  ;;
                --artifacts-dir)
                  artifacts_dir="''${2:-}"
                  shift 2
                  ;;
                --out-dir)
                  out_dir="''${2:-}"
                  shift 2
                  ;;
                --signing-key-file)
                  signing_key_file="''${2:-}"
                  shift 2
                  ;;
                --help|-h)
                  usage
                  exit 0
                  ;;
                *)
                  printf 'Unknown argument: %s\n\n' "$1" >&2
                  usage >&2
                  exit 1
                  ;;
              esac
            done

            if [[ -z "$version" || -z "$artifacts_dir" || -z "$out_dir" ]]; then
              usage >&2
              exit 1
            fi

            mkdir -p "$out_dir"

            shopt -s nullglob
            manifest_inputs=("$artifacts_dir"/sce-v"$version"-*.json)
            checksum_inputs=("$artifacts_dir"/sce-v"$version"-*.tar.gz.sha256)
            shopt -u nullglob

            if [[ ''${#manifest_inputs[@]} -eq 0 ]]; then
              printf 'No release manifest fragments found in %s\n' "$artifacts_dir" >&2
              exit 1
            fi

            manifest_path="$out_dir/sce-v''${version}-release-manifest.json"
            signature_path="$manifest_path.sig"
            checksums_path="$out_dir/sce-v''${version}-SHA256SUMS"

            : > "$checksums_path"
            mapfile -t checksum_inputs_sorted < <(printf '%s\n' "''${checksum_inputs[@]}" | sort)
            for checksum_file in "''${checksum_inputs_sorted[@]}"; do
              while IFS= read -r line; do
                printf '%s\n' "$line" >> "$checksums_path"
              done < "$checksum_file"
            done

            mapfile -t manifest_inputs_sorted < <(printf '%s\n' "''${manifest_inputs[@]}" | sort)

            jq \
              --slurp \
              --arg version "$version" \
              '{
                version: $version,
                binary: "sce",
                artifacts: (sort_by(.target_triple))
               }' "''${manifest_inputs_sorted[@]}" > "$manifest_path"

            sign_args=(
              ./scripts/sign-release-manifest.mjs
              --manifest "$manifest_path"
              --signature-output "$signature_path"
            )

            if [[ -n "$signing_key_file" ]]; then
              sign_args+=(--private-key-file "$signing_key_file")
            fi

            node "''${sign_args[@]}"

            printf 'Assembled release metadata:\n'
            printf '  %s\n' "$manifest_path"
            printf '  %s\n' "$signature_path"
            printf '  %s\n' "$checksums_path"
          '';
        };

        releaseNpmPackageApp = pkgs.writeShellApplication {
          name = "release-npm-package";
          runtimeInputs = [
            pkgs.coreutils
            pkgs.jq
            pkgs.nodejs
          ];
          text = ''
            set -euo pipefail

            usage() {
              cat <<'EOF'
            Usage: release-npm-package --version <semver> --out-dir <path>

            Builds the canonical npm package tarball for `sce` using the checked-in npm
            launcher package and writes the packed tarball plus package metadata into the
            output directory.
            EOF
            }

            version=""
            out_dir=""

            while [[ $# -gt 0 ]]; do
              case "$1" in
                --version)
                  version="''${2:-}"
                  shift 2
                  ;;
                --out-dir)
                  out_dir="''${2:-}"
                  shift 2
                  ;;
                --help|-h)
                  usage
                  exit 0
                  ;;
                *)
                  printf 'Unknown argument: %s\n\n' "$1" >&2
                  usage >&2
                  exit 1
                  ;;
              esac
            done

            if [[ -z "$version" || -z "$out_dir" ]]; then
              usage >&2
              exit 1
            fi

            if [[ ! -f "flake.nix" ]]; then
              printf 'release-npm-package must be run from the repository root that contains flake.nix\n' >&2
              exit 1
            fi

            checked_in_version="$(tr -d '\n' < .version)"
            if [[ "$version" != "$checked_in_version" ]]; then
              printf 'Requested release version %s does not match checked-in .version %s\n' "$version" "$checked_in_version" >&2
              exit 1
            fi

            cargo_version="$(sed -n 's/^version = "\([^"]*\)"$/\1/p' cli/Cargo.toml | head -n 1)"
            npm_version="$(${pkgs.nodejs}/bin/node -p "JSON.parse(require('fs').readFileSync('npm/package.json', 'utf8')).version")"

            if [[ "$cargo_version" != "$version" ]]; then
              printf 'cli/Cargo.toml version %s does not match release version %s\n' "$cargo_version" "$version" >&2
              exit 1
            fi

            if [[ "$npm_version" != "$version" ]]; then
              printf 'npm/package.json version %s does not match release version %s\n' "$npm_version" "$version" >&2
              exit 1
            fi

            mkdir -p "$out_dir"

            tmp_dir="$(mktemp -d)"
            cleanup() {
              rm -rf "$tmp_dir"
            }
            trap cleanup EXIT

            stage_dir="$tmp_dir/npm-package"
            mkdir -p "$stage_dir"
            cp -R npm/. "$stage_dir/"

            packed_name="$(npm pack --silent "$stage_dir")"
            package_name="sce-v''${version}-npm.tgz"
            metadata_name="sce-v''${version}-npm.json"

            mv "$packed_name" "$out_dir/$package_name"

            npm_pkg_name="$(${pkgs.nodejs}/bin/node -p "JSON.parse(require('fs').readFileSync('npm/package.json', 'utf8')).name")"

            jq \
              --null-input \
              --arg version "$version" \
              --arg package "$package_name" \
              --arg npm_pkg_name "$npm_pkg_name" \
              '{
                version: $version,
                package_name: $npm_pkg_name,
                package_file: $package,
                install_command: "npm install -g \($npm_pkg_name)"
              }' > "$out_dir/$metadata_name"

            printf 'Built npm release assets:\n'
            printf '  %s\n' "$out_dir/$package_name"
            printf '  %s\n' "$out_dir/$metadata_name"
          '';
        };

        installChannelIntegrationTestsApp = pkgs.writeShellApplication {
          name = "install-channel-integration-tests";
          runtimeInputs = [
            pkgs.nodejs
            pkgs.bun
            rustToolchain
          ];
          text = ''
            set -euo pipefail

            export SCE_INSTALL_CHANNEL_SCE_BIN="${scePackage}/bin/sce"
            exec "${integrationsInstallPackage}/bin/install-channel-integration-tests" "$@"
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

              export GIT_PAGER=cat

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

        configLibBunTests =
          pkgs.runCommand "config-lib-bun-tests"
            {
              nativeBuildInputs = [ pkgs.bun ];
            }
            ''
              set -euo pipefail

              # Copy source files
              cp -r "${configLibBashPolicySrc}" ./bash-policy
              chmod -R u+w ./bash-policy
              cd ./bash-policy

              # Use pre-fetched dependencies from FOD
              cp -r "${configLibBashPolicyDeps}/node_modules" ./

              # Run tests
              bun test

              mkdir -p "$out"
            '';

        configLibBiomeCheck =
          pkgs.runCommand "config-lib-biome-check"
            {
              nativeBuildInputs = [ pkgs.biome ];
            }
            ''
              set -euo pipefail

              cp -r "${configLibBashPolicySrc}" ./bash-policy
              chmod -R u+w ./bash-policy
              cd ./bash-policy

              biome check --formatter-enabled=false .

              mkdir -p "$out"
            '';

        configLibBiomeFormat =
          pkgs.runCommand "config-lib-biome-format"
            {
              nativeBuildInputs = [ pkgs.biome ];
            }
            ''
              set -euo pipefail

              cp -r "${configLibBashPolicySrc}" ./bash-policy
              chmod -R u+w ./bash-policy
              cd ./bash-policy

              biome check --linter-enabled=false .

              mkdir -p "$out"
            '';

        npmTests =
          pkgs.runCommand "npm-bun-tests"
            {
              nativeBuildInputs = [ pkgs.bun ];
            }
            ''
              set -euo pipefail

              cp -r "${npmSrc}" ./repo
              chmod -R u+w ./repo
              cd ./repo/npm

              bun test ./test/*.test.js

              mkdir -p "$out"
            '';

        npmBiomeCheck =
          pkgs.runCommand "npm-biome-check"
            {
              nativeBuildInputs = [ pkgs.biome ];
            }
            ''
              set -euo pipefail

              cp -r "${npmSrc}" ./repo
              chmod -R u+w ./repo
              cd ./repo/npm

              biome check --formatter-enabled=false .

              mkdir -p "$out"
            '';

        npmBiomeFormat =
          pkgs.runCommand "npm-biome-format"
            {
              nativeBuildInputs = [ pkgs.biome ];
            }
            ''
              set -euo pipefail

              cp -r "${npmSrc}" ./repo
              chmod -R u+w ./repo
              cd ./repo/npm

              biome check --linter-enabled=false .

              mkdir -p "$out"
            '';

        sceApp = {
          type = "app";
          program = "${scePackage}/bin/sce";
          meta = {
            description = "Run the packaged sce CLI";
          };
        };
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

          integrations-install-tests = craneLib.cargoTest (
            integrationsInstallCargoArgs
            // {
              pname = "sce-integrations-install-tests";
              cargoArtifacts = integrationsInstallCargoArtifacts;
            }
          );

          integrations-install-clippy = craneLib.cargoClippy (
            integrationsInstallCargoArgs
            // {
              pname = "sce-integrations-install-clippy";
              cargoArtifacts = integrationsInstallCargoArtifacts;
              cargoClippyExtraArgs = "--all-targets --all-features";
            }
          );

          integrations-install-fmt = craneLib.cargoFmt (
            integrationsInstallCargoArgs
            // {
              pname = "sce-integrations-install-fmt";
            }
          );

          pkl-parity = pklParityCheck;

          npm-bun-tests = npmTests;
          npm-biome-check = npmBiomeCheck;
          npm-biome-format = npmBiomeFormat;

          config-lib-bun-tests = configLibBunTests;
          config-lib-biome-check = configLibBiomeCheck;
          config-lib-biome-format = configLibBiomeFormat;
        };

        apps = {
          sce = sceApp;
          default = sceApp;

          pkl-check-generated = {
            type = "app";
            program = "${pklCheckGeneratedApp}/bin/pkl-check-generated";
            meta = {
              description = "Run generated-output drift check in dev shell";
            };
          };

          release-artifacts = {
            type = "app";
            program = "${releaseArtifactsApp}/bin/release-artifacts";
            meta = {
              description = "Build current-platform sce release artifacts";
            };
          };

          release-manifest = {
            type = "app";
            program = "${releaseManifestApp}/bin/release-manifest";
            meta = {
              description = "Assemble sce release manifest";
            };
          };

          release-npm-package = {
            type = "app";
            program = "${releaseNpmPackageApp}/bin/release-npm-package";
            meta = {
              description = "Build sce npm package tarball";
            };
          };

          install-channel-integration-tests = {
            type = "app";
            program = "${installChannelIntegrationTestsApp}/bin/install-channel-integration-tests";
            meta = {
              description = "Run opt-in install-channel integration entrypoint";
            };
          };
        };

        devShells.default = pkgs.mkShell {
          packages =
            with pkgs;
[
               biome
               bun
               jq
               pkl
               typescript
               nodePackages.typescript-language-server
               rust-analyzer
               scePackage
            ]
            ++ [ rustToolchain ];

          shellHook = ''
            version_of() {
              "$1" --version 2>/dev/null | awk 'match($0, /[0-9]+(\.[0-9]+)+/) { print substr($0, RSTART, RLENGTH); exit }'
            }

            echo "- bun: $(version_of bun)"
            echo "- biome: $(version_of biome)"
            echo "- pkl: $(version_of pkl)"
            echo "- tsc: $(version_of tsc)"
            echo "- tsserver-lsp: $(version_of typescript-language-server)"
            echo "- rust: $(version_of rustc)"
            echo "- sce: $(version_of sce)"
            echo "- pkl-check-generated: nix run .#pkl-check-generated"
            echo "- release-artifacts: nix run .#release-artifacts -- --help"
            echo "- release-manifest: nix run .#release-manifest -- --help"
            echo "- release-npm-package: nix run .#release-npm-package -- --help"
          '';
        };
      }
    );
}
