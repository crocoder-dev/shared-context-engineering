{
  description = "Shared Context Engineering repository and sce CLI flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    crane.url = "github:ipetkov/crane";
    opencode.url = "github:anomalyco/opencode/dev";
    opencode-nixpkgs.follows = "opencode/nixpkgs";
    turso.url = "github:tursodatabase/turso/1ebe80ff228f3a56cb521d44b12dc9a7bd04b027";
    turso.inputs.nixpkgs.follows = "nixpkgs";
    turso.inputs.flake-utils.follows = "flake-utils";
    turso.inputs.crane.follows = "crane";
    turso.inputs.rust-overlay.follows = "rust-overlay";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
      crane,
      opencode,
      opencode-nixpkgs,
      turso,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
          config.allowUnfreePredicate =
            pkg: builtins.elem (pkgs.lib.getName pkg) [ "claude-code" ];
        };

        opencodePkgs = import opencode-nixpkgs {
          inherit system;
        };

        bunVersion = "1.3.14";
        bunSource =
          {
            x86_64-linux = {
              platform = "linux-x64";
              hash = "sha256-lR7iruhV8IWVruxiJSJqKY0/6oOj3NZGXAnLzN9+hI8=";
            };
            aarch64-linux = {
              platform = "linux-aarch64";
              hash = "sha256-on/7Y6gxA3WDbg1vZorhf6jY0YuIw3yCHGUzGXOhmjs=";
            };
            x86_64-darwin = {
              platform = "darwin-x64";
              hash = "sha256-QYPfM3RiPlurMVxUfPoJdFM81FfYa3O2OfeoeXTNZjM=";
            };
            aarch64-darwin = {
              platform = "darwin-aarch64";
              hash = "sha256-2LliIYKK1vl6x6wKt+lYcjQa92MAHogD6CZ2UsJlJiA=";
            };
          }
          .${system};
        bunPackage = pkgs.bun.overrideAttrs (_oldAttrs: {
          version = bunVersion;
          src = pkgs.fetchurl {
            url = "https://github.com/oven-sh/bun/releases/download/bun-v${bunVersion}/bun-${bunSource.platform}.zip";
            hash = bunSource.hash;
          };
        });

        rustVersion = "1.95.0";

        rustToolchain = pkgs.rust-bin.stable.${rustVersion}.default.override {
          extensions = [
            "rustfmt"
            "clippy"
            "rust-src"
          ];
        };

        tursoToolchain = (pkgs.rust-bin.fromRustupToolchainFile "${turso}/rust-toolchain.toml").override {
          targets = [ "wasm32-unknown-unknown" ];
        };

        craneLib = (crane.mkLib pkgs).overrideToolchain (_: rustToolchain);

        tursoCraneLib = (crane.mkLib pkgs).overrideToolchain (_: tursoToolchain);

        workspaceRoot = ./.;
        workspaceSrc = pkgs.lib.fileset.toSource {
          root = workspaceRoot;
          fileset = pkgs.lib.fileset.unions [
            (craneLib.fileset.commonCargoSources workspaceRoot)
            (pkgs.lib.fileset.maybeMissing ./.version)
            (pkgs.lib.fileset.maybeMissing ./cli/src/services/default_paths.rs)
            (pkgs.lib.fileset.maybeMissing ./cli/src/services/agent_trace/fixtures)
            (pkgs.lib.fileset.maybeMissing ./cli/src/services/patch/fixtures)
            (pkgs.lib.fileset.maybeMissing ./cli/src/services/structured_patch/fixtures)
            (pkgs.lib.fileset.maybeMissing ./cli/migrations)
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
          root = workspaceRoot;
          fileset = pkgs.lib.fileset.unions [
            ./config/lib/package.json
            ./config/lib/bun.lock
            ./config/lib/tsconfig.json
            ./config/lib/agent-trace-plugin
./config/lib/bash-policy-plugin/bash-policy-runtime.test.ts
            ./config/lib/bash-policy-plugin/opencode-bash-policy-plugin.ts
            (pkgs.lib.fileset.maybeMissing ./cli/src/services/structured_patch/fixtures)
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

        flatpakPackagingSrc = pkgs.lib.fileset.toSource {
          root = workspaceRoot;
          fileset = pkgs.lib.fileset.unions [
            ./packaging/flatpak/dev.crocoder.sce.yml
            ./packaging/flatpak/dev.crocoder.sce.metainfo.xml
            ./packaging/flatpak/git-host-bridge
            ./packaging/flatpak/cargo-sources.json
            ./packaging/flatpak/sce-flatpak.sh
          ];
        };

        # Fixed-output derivation to fetch Bun dependencies
        # The output hash must be updated when package.json or bun.lock changes
        configLibBashPolicyDeps = pkgs.stdenv.mkDerivation {
          pname = "config-lib-bash-policy-deps";
          version = "0.1.0";
          src = pkgs.lib.fileset.toSource {
            root = ./config/lib;
            fileset = pkgs.lib.fileset.unions [
              ./config/lib/package.json
              ./config/lib/bun.lock
            ];
          };
          nativeBuildInputs = [ bunPackage ];
          dontBuild = true;
          installPhase = ''
            bun install --frozen-lockfile --no-progress
            # Remove Bun's cache symlinks that point to build directory
            rm -rf node_modules/.cache
            rm -f node_modules/.bin/download-msgpackr-prebuilds
            mkdir -p $out
            cp -r node_modules $out/
          '';
          outputHashMode = "recursive";
          outputHashAlgo = "sha256";
          outputHash = if pkgs.stdenv.isLinux
            then "sha256-yDKVHH46EzzyiCwBSISEXnJJbqZ2ihvS2H0SGgITaPY="
            else "sha256-KpUXn9+gHy5whrKWXBt9KZI9RwSpa7DLNfRLL/bMT4Q=";
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

        cargoDepsArgs = {
          pname = "sce-deps";
          inherit version;
          src = craneLib.cleanCargoSource ./cli;
          cargoToml = ./cli/Cargo.toml;
          cargoLock = ./cli/Cargo.lock;
          strictDeps = true;
          doCheck = false;

          nativeBuildInputs = [
            rustToolchain
          ];
        };

        cargoArtifacts = craneLib.buildDepsOnly cargoDepsArgs;

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

        opencodePackage = opencode.packages.${system}.opencode.overrideAttrs (oldAttrs: {
          postPatch = (oldAttrs.postPatch or "") + ''
            substituteInPlace package.json \
              --replace-fail '"packageManager": "bun@1.3.14"' \
              '"packageManager": "bun@${opencodePkgs.bun.version}"'
          '';
        });

        claudeCodePackage = pkgs.claude-code;

        tursoCargoArgs = {
          pname = "turso";
          version = "0.7.0-pre.10";
          src = turso;
          strictDeps = true;

          nativeBuildInputs = with pkgs; [
            pkg-config
            python3
          ];

          buildInputs = [ pkgs.openssl ];
        };

        tursoCargoArtifacts = tursoCraneLib.buildDepsOnly tursoCargoArgs;

        tursoUpstreamPackage = tursoCraneLib.buildPackage (
          tursoCargoArgs
          // {
            cargoArtifacts = tursoCargoArtifacts;
            cargoExtraArgs = "--package turso_cli";
            doCheck = false;
          }
        );

        tursoPackage = pkgs.symlinkJoin {
          name = "turso-${tursoUpstreamPackage.version}";
          paths = [ tursoUpstreamPackage ];
          postBuild = ''
            ln -s "$out/bin/tursodb" "$out/bin/turso"
          '';
          meta = tursoUpstreamPackage.meta // {
            mainProgram = "turso";
          };
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

        pklGenerateApp = pkgs.writeShellApplication {
          name = "pkl-generate";
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

            exec nix develop "''${repo_root}" -c pkl eval -m "''${repo_root}" "''${repo_root}/config/pkl/generate.pkl"
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
            bunPackage
            rustToolchain
          ];
          text = ''
            set -euo pipefail

            export SCE_INSTALL_CHANNEL_SCE_BIN="${scePackage}/bin/sce"
            exec "${integrationsInstallPackage}/bin/install-channel-integration-tests" "$@"
          '';
        };

        flatpakToolRuntimeInputs =
          [
            pkgs.coreutils
            pkgs.git
            pkgs.gnutar
            pkgs.gzip
            pkgs.python3
          ]
          ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
            pkgs.appstream
            pkgs.flatpak
            pkgs.flatpak-builder
          ];

        flatpakToolApp = pkgs.writeShellApplication {
          name = "sce-flatpak";
          runtimeInputs = flatpakToolRuntimeInputs;
          text = ''
            exec ${pkgs.bash}/bin/bash ${./packaging/flatpak/sce-flatpak.sh} "$@"
          '';
        };

        flatpakValidateApp = pkgs.writeShellApplication {
          name = "flatpak-validate";
          runtimeInputs = [ flatpakToolApp ];
          text = ''
            exec sce-flatpak validate "$@"
          '';
        };

        flatpakLocalManifestApp = pkgs.writeShellApplication {
          name = "flatpak-local-manifest";
          runtimeInputs = [ flatpakToolApp ];
          text = ''
            exec sce-flatpak prepare-local-manifest "$@"
          '';
        };

        flatpakBuildApp = pkgs.writeShellApplication {
          name = "flatpak-build";
          runtimeInputs = [ flatpakToolApp ];
          text = ''
            exec sce-flatpak build "$@"
          '';
        };

        releaseFlatpakPackageApp = pkgs.writeShellApplication {
          name = "release-flatpak-package";
          runtimeInputs = [ flatpakToolApp ];
          text = ''
            exec sce-flatpak release-package "$@"
          '';
        };

        releaseFlatpakBundleApp = pkgs.writeShellApplication {
          name = "release-flatpak-bundle";
          runtimeInputs = [ flatpakToolApp ];
          text = ''
            exec sce-flatpak release-bundle "$@"
          '';
        };

        flatpakStaticValidationCheck = pkgs.runCommand "flatpak-static-validation"
          {
            nativeBuildInputs = [ flatpakToolApp ];
          }
          ''
            set -euo pipefail

            cp -r "${flatpakPackagingSrc}" ./repo
            chmod -R u+w ./repo
            cd ./repo

            sce-flatpak validate --repo-root "$PWD" --skip-optional-lint

            mkdir -p "$out"
          '';

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
              cp -r "${configLibBashPolicySrc}" ./repo
              chmod -R u+w ./repo
              cd ./repo/config/lib

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

              cp -r "${configLibBashPolicySrc}" ./repo
              chmod -R u+w ./repo
              cd ./repo/config/lib

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

              cp -r "${configLibBashPolicySrc}" ./repo
              chmod -R u+w ./repo
              cd ./repo/config/lib

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

        workflowActionlintCheck =
          pkgs.runCommand "workflow-actionlint"
            {
              nativeBuildInputs = [ pkgs.actionlint ];
            }
            ''
              set -euo pipefail

              cp -r "${./.github/workflows}" ./workflows
              chmod -R u+w ./workflows

              actionlint ./workflows/*.yml

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
          bun = bunPackage;
          opencode = opencodePackage;
          claude-code = claudeCodePackage;
          turso = tursoPackage;
          default = scePackage;
        };

        checks =
          {
            cli-tests = craneLib.cargoTest (
              commonCargoArgs
              // {
                pname = "sce-cli-tests";
                inherit cargoArtifacts;
                doCheck = true;
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

            workflow-actionlint = workflowActionlintCheck;
          }
          // pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
            flatpak-static-validation = flatpakStaticValidationCheck;
          };

        apps =
          {
            sce = sceApp;
            default = sceApp;

            pkl-check-generated = {
              type = "app";
              program = "${pklCheckGeneratedApp}/bin/pkl-check-generated";
              meta = {
                description = "Run generated-output drift check in dev shell";
              };
            };

            pkl-generate = {
              type = "app";
              program = "${pklGenerateApp}/bin/pkl-generate";
              meta = {
                description = "Generate config outputs from Pkl sources";
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
          }
          // pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
            flatpak-validate = {
              type = "app";
              program = "${flatpakValidateApp}/bin/flatpak-validate";
              meta = {
                description = "Validate Flatpak packaging metadata and local-source manifest generation";
              };
            };

            flatpak-local-manifest = {
              type = "app";
              program = "${flatpakLocalManifestApp}/bin/flatpak-local-manifest";
              meta = {
                description = "Generate a Flatpak manifest that builds from the current checkout";
              };
            };

            flatpak-build = {
              type = "app";
              program = "${flatpakBuildApp}/bin/flatpak-build";
              meta = {
                description = "Build the sce Flatpak from the current checkout with flatpak-builder";
              };
            };

            release-flatpak-package = {
              type = "app";
              program = "${releaseFlatpakPackageApp}/bin/release-flatpak-package";
              meta = {
                description = "Build Flatpak source-manifest GitHub Release assets";
              };
            };

            release-flatpak-bundle = {
              type = "app";
              program = "${releaseFlatpakBundleApp}/bin/release-flatpak-bundle";
              meta = {
                description = "Build Flatpak bundle GitHub Release assets";
              };
            };
          };

        devShells.default = pkgs.mkShell {
          packages =
            with pkgs;
            [
              biome
              bunPackage
              jq
              pkl
              pkl-lsp
              typescript
              typescript-language-server
              vscode-json-languageserver
              opencodePackage
              claudeCodePackage
              rust-analyzer
              scePackage
              tursoPackage
            ]
            ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
              appstream
              flatpak
              flatpak-builder
            ]
            ++ [ rustToolchain ];

          shellHook = ''
            version_of() {
              "$1" --version 2>/dev/null | awk 'match($0, /[0-9]+(\.[0-9]+)+/) { print substr($0, RSTART, RLENGTH); exit }'
            }

            alias claude="${claudeCodePackage}/bin/claude"
            alias cc="${claudeCodePackage}/bin/claude"

            echo "- bun: $(version_of bun)"
            echo "- biome: $(version_of biome)"
            echo "- pkl: $(version_of pkl)"
            echo "- pkl-lsp: $(version_of pkl-lsp)"
            echo "- tsc: $(version_of tsc)"
            echo "- tsserver-lsp: $(version_of typescript-language-server)"
            echo "- rust: $(version_of rustc)"
            echo "- sce: $(version_of sce)"
            echo "- opencode: $(version_of opencode)"
            echo "- claude-code: $(version_of claude)"
            echo "- turso: $(version_of turso)"
            echo "- pkl-generate: nix run .#pkl-generate"
            echo "- pkl-check-generated: nix run .#pkl-check-generated"
            echo "- release-artifacts: nix run .#release-artifacts -- --help"
            echo "- release-manifest: nix run .#release-manifest -- --help"
            echo "- release-npm-package: nix run .#release-npm-package -- --help"
            ${pkgs.lib.optionalString pkgs.stdenv.isLinux ''
              echo "- flatpak: $(version_of flatpak)"
              echo "- flatpak-builder: $(version_of flatpak-builder)"
              echo "- appstreamcli: $(version_of appstreamcli)"
              echo "- flatpak-validate: nix run .#flatpak-validate"
              echo "- flatpak-local-manifest: nix run .#flatpak-local-manifest"
              echo "- flatpak-build: nix run .#flatpak-build -- --help"
              echo "- release-flatpak-package: nix run .#release-flatpak-package -- --help"
              echo "- release-flatpak-bundle: nix run .#release-flatpak-bundle -- --help"
            ''}
          '';
        };
      }
    );
}
