{
  description = "Shared Context Engineering repository and sce CLI flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    crane.url = "github:ipetkov/crane";
    turso.url = "github:tursodatabase/turso/e7cb62a8bd2f3655a661a621ee389365c1a1e43e";
    turso.inputs.nixpkgs.follows = "nixpkgs";
    turso.inputs.flake-utils.follows = "flake-utils";
    turso.inputs.crane.follows = "crane";
    turso.inputs.rust-overlay.follows = "rust-overlay";
    flatpak-builder-tools.url = "github:flatpak/flatpak-builder-tools";
    flatpak-builder-tools.flake = false;
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
      crane,
      turso,
      flatpak-builder-tools,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
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

        # Rust target triple for fully static musl Linux builds.
        # null on non-Linux systems where the musl pipeline is never referenced.
        muslTarget =
          if pkgs.stdenv.isLinux then
            if pkgs.stdenv.hostPlatform.isAarch64 then
              "aarch64-unknown-linux-musl"
            else if pkgs.stdenv.hostPlatform.isx86_64 then
              "x86_64-unknown-linux-musl"
            else
              throw "Unsupported Linux architecture for musl build"
          else
            null;

        # Per-target environment variables that tell cc-rs (and CMake) to
        # compile C/C++ dependencies with the musl toolchain rather than the
        # host glibc toolchain.  Without these, native deps like aws-lc-sys
        # and zstd-sys emit glibc symbols that musl cannot satisfy.
        # Nix's musl gcc wrapper exposes target-prefixed binaries
        # (e.g. x86_64-unknown-linux-musl-cc), not bin/cc.
        # Use cc.targetPrefix to construct the correct path.
        muslEnvVars =
          let
            staticPkgs = pkgs.pkgsStatic;
            cc = staticPkgs.stdenv.cc;
            bintools = staticPkgs.stdenv.cc.bintools;
            ts = pkgs.lib.replaceStrings [ "-" ] [ "_" ] muslTarget;
            tsUpper = pkgs.lib.toUpper ts;
          in
          {
            "CC_${ts}" = "${cc}/bin/${cc.targetPrefix}cc";
            "CXX_${ts}" = "${cc}/bin/${cc.targetPrefix}c++";
            "AR_${ts}" = "${bintools}/bin/${bintools.targetPrefix}ar";
            "CARGO_TARGET_${tsUpper}_LINKER" =
              "${cc}/bin/${cc.targetPrefix}cc";
            "CFLAGS_${ts}" =
              "-U_FORTIFY_SOURCE -D_FORTIFY_SOURCE=0";
            "CXXFLAGS_${ts}" =
              "-U_FORTIFY_SOURCE -D_FORTIFY_SOURCE=0";
          };

        # Separate toolchain that adds the musl target alongside the host target.
        # Host-targeted checks (cli-tests, cli-clippy, cli-fmt) continue to use
        # rustToolchain and are completely unaffected by this addition.
        rustToolchainMusl = pkgs.rust-bin.stable.${rustVersion}.default.override {
          extensions = [
            "rustfmt"
            "clippy"
            "rust-src"
          ];
          targets = pkgs.lib.optional (muslTarget != null) muslTarget;
        };

        craneLibMusl = (crane.mkLib pkgs).overrideToolchain (_: rustToolchainMusl);

        tursoCraneLib = (crane.mkLib pkgs).overrideToolchain (_: tursoToolchain);

        workspaceRoot = ./.;
        generatedConfigFileset = pkgs.lib.fileset.unions [
          (pkgs.lib.fileset.maybeMissing ./config/.opencode)
          (pkgs.lib.fileset.maybeMissing ./config/.claude)
          (pkgs.lib.fileset.maybeMissing ./config/.pi)
          ./config/schema/sce-config.schema.json
          ./config/schema/agent-trace.schema.json
        ];
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
            generatedConfigFileset
            (pkgs.lib.fileset.maybeMissing ./cli/assets/hooks)
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

        pklParitySrc = pkgs.lib.fileset.toSource {
          root = workspaceRoot;
          fileset = pkgs.lib.fileset.unions [
            ./config/pkl
            (pkgs.lib.fileset.maybeMissing ./config/.opencode/agent)
            (pkgs.lib.fileset.maybeMissing ./config/.opencode/command)
            (pkgs.lib.fileset.maybeMissing ./config/.opencode/skills)
            (pkgs.lib.fileset.maybeMissing ./config/.opencode/lib/bash-policy-presets.json)
            (pkgs.lib.fileset.maybeMissing ./config/.opencode/plugins)
            (pkgs.lib.fileset.maybeMissing ./config/.opencode/opencode.json)
            (pkgs.lib.fileset.maybeMissing ./config/automated/.opencode/agent)
            (pkgs.lib.fileset.maybeMissing ./config/automated/.opencode/command)
            (pkgs.lib.fileset.maybeMissing ./config/automated/.opencode/skills)
            (pkgs.lib.fileset.maybeMissing ./config/automated/.opencode/lib/bash-policy-presets.json)
            (pkgs.lib.fileset.maybeMissing ./config/automated/.opencode/plugins)
            (pkgs.lib.fileset.maybeMissing ./config/automated/.opencode/opencode.json)
            (pkgs.lib.fileset.maybeMissing ./config/.claude/agents)
            (pkgs.lib.fileset.maybeMissing ./config/.claude/commands)
            (pkgs.lib.fileset.maybeMissing ./config/.claude/skills)
            (pkgs.lib.fileset.maybeMissing ./config/.claude/hooks/run-sce-or-show-install-guidance.sh)
            (pkgs.lib.fileset.maybeMissing ./config/.claude/settings.json)
            (pkgs.lib.fileset.maybeMissing ./config/.pi/prompts)
            (pkgs.lib.fileset.maybeMissing ./config/.pi/skills)
            (pkgs.lib.fileset.maybeMissing ./config/.pi/extensions)
            (pkgs.lib.fileset.maybeMissing ./config/schema/sce-config.schema.json)
            (pkgs.lib.fileset.maybeMissing ./.opencode/agent)
            (pkgs.lib.fileset.maybeMissing ./.opencode/command)
            (pkgs.lib.fileset.maybeMissing ./.opencode/skills)
            (pkgs.lib.fileset.maybeMissing ./.claude/agents)
            (pkgs.lib.fileset.maybeMissing ./.claude/commands)
            (pkgs.lib.fileset.maybeMissing ./.claude/skills)
            (pkgs.lib.fileset.maybeMissing ./.pi/prompts)
            (pkgs.lib.fileset.maybeMissing ./.pi/skills)
            (pkgs.lib.fileset.maybeMissing ./templates)
            ./config/lib/pi-plugin/sce-pi-extension.ts
            ./config/lib/bash-policy-plugin/opencode-bash-policy-plugin.ts
            ./config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts
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
          # Shebang patching would embed the bash store path, breaking the
          # fixed-output hash whenever bash changes.
          dontFixup = true;
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
            then "sha256-jQAnW/deCeux0/oxmH27lBXmJoK3RGNJDtIGM+Eepmo="
            else "sha256-ia8V9TQM4pHR+A7jEjsARGMX1vljmZDp30M87Wi4oWA=";
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

        cargoBaseArgs = {
          inherit version;
          cargoToml = ./cli/Cargo.toml;
          cargoLock = ./cli/Cargo.lock;
          strictDeps = true;
          doCheck = false;
          nativeBuildInputs = [ rustToolchain ];
        };

        # Commit embedding is release-only: SCE_GIT_COMMIT is applied via
        # releaseCommitArgs to the release derivations only. Keeping it out of
        # commonCargoArgs means the native package, cargo tests, Clippy, and fmt
        # stay commit-independent (cache-reusable across commits).
        releaseCommitArgs = {
          SCE_GIT_COMMIT = shortGitCommit;
        };

        commonCargoArgs = cargoBaseArgs // {
          pname = "sce";
          src = workspaceSrc;

          postUnpack = ''
            mkdir -p "$sourceRoot/cli/assets/generated/config"
            cp -R ${./config/.opencode} "$sourceRoot/cli/assets/generated/config/opencode"
            cp -R ${./config/.claude} "$sourceRoot/cli/assets/generated/config/claude"
            cp -R ${./config/.pi} "$sourceRoot/cli/assets/generated/config/pi"
            mkdir -p "$sourceRoot/cli/assets/generated/config/schema"
            cp ${./config/schema/sce-config.schema.json} "$sourceRoot/cli/assets/generated/config/schema/sce-config.schema.json"
            cp ${./config/schema/agent-trace.schema.json} "$sourceRoot/cli/assets/generated/config/schema/agent-trace.schema.json"

            cd "$sourceRoot/cli"
            sourceRoot="."
          '';
        };

        cargoDepsArgs = cargoBaseArgs // {
          pname = "sce-deps";
          src = craneLib.cleanCargoSource ./cli;
        };

        cargoArtifacts = craneLib.buildDepsOnly cargoDepsArgs;

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

        # Separate Crane pipeline that builds a fully static musl binary on Linux.
        # On macOS this derivation is never referenced (sceReleasePackage selects
        # scePackage), so the null muslTarget is harmless.
        cargoDepsArgsMusl =
          (cargoDepsArgs // muslEnvVars)
          // {
            pname = "sce-deps-musl";
            nativeBuildInputs = [
              rustToolchainMusl
              pkgs.cmake
              pkgs.perl
              pkgs.pkg-config
              pkgs.pkgsStatic.stdenv.cc
              pkgs.pkgsStatic.binutils
            ];
            CARGO_BUILD_TARGET = muslTarget;
          };

        cargoArtifactsMusl = craneLibMusl.buildDepsOnly cargoDepsArgsMusl;

        scePackageMusl = craneLibMusl.buildPackage (
          (commonCargoArgs // muslEnvVars // releaseCommitArgs)
          // {
            inherit cargoArtifactsMusl;
            nativeBuildInputs = [
              rustToolchainMusl
              pkgs.cmake
              pkgs.perl
              pkgs.pkg-config
              pkgs.pkgsStatic.stdenv.cc
              pkgs.pkgsStatic.binutils
            ];
            CARGO_BUILD_TARGET = muslTarget;

            # libloading (pulled by turso_core) emits `-ldl`, but musl has no
            # separate libdl — the symbols live in libc.  Provide a stub archive
            # so the linker can satisfy `-ldl` without pulling in glibc.
            preBuild = ''
              mkdir -p /tmp/musl-dl-shim
              cat > /tmp/musl-dl-shim/dl.c <<'CEOF'
              void __dummy_libdl(void) {}
              CEOF
              $CC -c /tmp/musl-dl-shim/dl.c -o /tmp/musl-dl-shim/dl.o
              ar rcs /tmp/musl-dl-shim/libdl.a /tmp/musl-dl-shim/dl.o
              export LIBRARY_PATH="/tmp/musl-dl-shim:$LIBRARY_PATH"
            '';

            meta = {
              mainProgram = "sce";
              description = "Shared Context Engineering CLI (static musl)";
            };
          }
        );

        # Native-toolchain release build for Darwin. Identical to scePackage
        # except it embeds the real Git commit (SCE_GIT_COMMIT), so on macOS the
        # release output reports the commit while native `.#sce` stays
        # commit-independent. cargoArtifacts is deps-only and unaffected by
        # SCE_GIT_COMMIT (only the cli crate's build.rs reads it), so it is
        # shared with scePackage at no extra rebuild cost.
        sceReleasePackageNative = craneLib.buildPackage (
          commonCargoArgs
          // releaseCommitArgs
          // {
            inherit cargoArtifacts;
            meta = {
              mainProgram = "sce";
              description = "Shared Context Engineering CLI (release)";
            };
          }
        );

        # Release package selection: static musl on Linux (fully static, no
        # /nix/store runtime references), native-with-commit on macOS.
        sceReleasePackage = if pkgs.stdenv.isLinux then scePackageMusl else sceReleasePackageNative;

        tursoCargoArgs = {
          pname = "turso";
          version = "0.7.0";
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

        # Shared development tooling for the default shell. Excludes `scePackage`
        # and `tursoPackage` so `nix develop` does not compile either CLI;
        # opt-in shells layer them back on when needed.
        defaultDevShellPackages =
          (with pkgs; [
            biome
            bunPackage
            jq
            pkl
            pkl-lsp
            typescript
            typescript-language-server
            vscode-json-languageserver
            rust-analyzer
          ])
          ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
            pkgs.appstream
            pkgs.flatpak
            pkgs.flatpak-builder
          ]
          ++ [ rustToolchain ];

        defaultDevShellHook = ''
          version_of() {
            "$1" --version 2>/dev/null | awk 'match($0, /[0-9]+(\.[0-9]+)+/) { print substr($0, RSTART, RLENGTH); exit }'
          }
          echo "- bun: $(version_of bun)"
          echo "- biome: $(version_of biome)"
          echo "- pkl: $(version_of pkl)"
          echo "- pkl-lsp: $(version_of pkl-lsp)"
          echo "- tsc: $(version_of tsc)"
          echo "- tsserver-lsp: $(version_of typescript-language-server)"
          echo "- rust: $(version_of rustc)"
          echo "- pkl-generate: nix run .#pkl-generate"
          echo "- pkl-check-generated: nix run .#pkl-check-generated"
          echo "- release-artifacts: nix run .#release-artifacts -- --help"
          echo "- native-portability-audit: nix run .#native-portability-audit -- --help"
          echo "- release-manifest: nix run .#release-manifest -- --help"
          echo "- release-npm-package: nix run .#release-npm-package -- --help"
          ${pkgs.lib.optionalString pkgs.stdenv.isLinux ''
            echo "- flatpak: $(version_of flatpak)"
            echo "- flatpak-builder: $(version_of flatpak-builder)"
            echo "- appstreamcli: $(version_of appstreamcli)"
            echo "- sce-flatpak: nix run .#sce-flatpak -- --help"
            echo "- release-flatpak-package: nix run .#release-flatpak-package -- --help"
            echo "- release-flatpak-bundle: nix run .#release-flatpak-bundle -- --help"
          ''}
        '';

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
          ]
          ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.binutils ]
          ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [ pkgs.darwin.cctools ];
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
                  printf 'x86_64-unknown-linux-musl'
                  ;;
                Linux:aarch64)
                  printf 'aarch64-unknown-linux-musl'
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

            audit_platform_for_os() {
              case "$1" in
                Linux)
                  printf 'linux'
                  ;;
                Darwin)
                  printf 'macos'
                  ;;
                *)
                  printf 'Unsupported release audit platform for os=%s\n' "$1" >&2
                  exit 1
                  ;;
              esac
            }

            sanitize_macos_binary() {
              local release_binary="$1"
              local otool_bin="''${NATIVE_PORTABILITY_AUDIT_OTOOL:-otool}"
              local install_name_tool_bin="''${SCE_RELEASE_INSTALL_NAME_TOOL:-install_name_tool}"
              local codesign_bin="''${SCE_RELEASE_CODESIGN:-codesign}"
              local otool_output="$tmp_dir/otool-before-sanitize.txt"
              local changed=0

              if ! command -v "$otool_bin" >/dev/null 2>&1; then
                printf 'macOS release binary sanitization failed: otool is required for install-name inspection\n' >&2
                exit 1
              fi

              if ! command -v "$install_name_tool_bin" >/dev/null 2>&1; then
                printf 'macOS release binary sanitization failed: install_name_tool is required to rewrite Nix store dylib references\n' >&2
                exit 1
              fi

              if ! "$otool_bin" -L "$release_binary" > "$otool_output" 2> "$tmp_dir/otool-before-sanitize.err"; then
                printf 'macOS release binary sanitization failed: otool -L could not inspect %s\n' "$release_binary" >&2
                cat "$tmp_dir/otool-before-sanitize.err" >&2
                exit 1
              fi

              while IFS= read -r line; do
                local install_name="''${line%% (*}"
                install_name="''${install_name#"''${install_name%%[![:space:]]*}"}"

                if [[ "$install_name" != /nix/store/* ]]; then
                  continue
                fi

                case "$install_name" in
                  */lib/libiconv.*.dylib)
                    local replacement
                    replacement="/usr/lib/$(basename "$install_name")"
                    printf 'Rewriting macOS release install name: %s -> %s\n' "$install_name" "$replacement"
                    "$install_name_tool_bin" -change "$install_name" "$replacement" "$release_binary"
                    changed=1
                    ;;
                  *)
                    printf 'macOS release binary sanitization failed: unsupported Nix store install name %s\n' "$install_name" >&2
                    exit 1
                    ;;
                esac
              done < "$otool_output"

              if [[ "$changed" -eq 1 ]]; then
                if ! command -v "$codesign_bin" >/dev/null 2>&1; then
                  printf 'macOS release binary sanitization failed: codesign is required after mutating install names\n' >&2
                  exit 1
                fi

                "$codesign_bin" --force --sign - "$release_binary"
              fi
            }

            os_name="$(uname -s)"
            arch_name="$(normalize_arch "$(uname -m)")"
            target_triple="$(detect_target_triple "$os_name" "$arch_name")"
            archive_name="sce-v''${version}-''${target_triple}.tar.gz"
            checksum_name="''${archive_name}.sha256"
            manifest_name="sce-v''${version}-''${target_triple}.json"
            archive_root="sce-v''${version}-''${target_triple}"

            mkdir -p "$out_dir"

            nix build .#sce-release --out-link result

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

            release_binary_path="$tmp_dir/$archive_root/bin/sce"
            audit_platform="$(audit_platform_for_os "$os_name")"

            if [[ "$audit_platform" == "macos" ]]; then
              sanitize_macos_binary "$release_binary_path"
            fi

            bash ${./nix/release/native-portability-audit.sh} --platform "$audit_platform" --binary "$release_binary_path"

            release_binary_version="$($release_binary_path version --format json | ${pkgs.jq}/bin/jq -r '.version')"
            if [[ "$release_binary_version" != "$version" ]]; then
              printf 'Prepared release CLI version %s does not match release version %s\n' "$release_binary_version" "$version" >&2
              exit 1
            fi

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

        nativePortabilityAuditApp = pkgs.writeShellApplication {
          name = "native-portability-audit";
          runtimeInputs =
            [ pkgs.coreutils ]
            ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.binutils ]
            ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [ pkgs.darwin.cctools ];
          text = ''
            exec bash ${./nix/release/native-portability-audit.sh} "$@"
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

        flatpakManifest = import ./nix/flatpak/manifest.nix {
          inherit pkgs;
          checkedInYaml = ./packaging/flatpak/dev.crocoder.sce.yml;
        };

        flatpakStaticCheckApp = pkgs.writeShellApplication {
          name = "flatpak-static-check";
          runtimeInputs = [
            pkgs.coreutils
            pkgs.gawk
            pkgs.jq
            pkgs.libxml2
          ];
          text = builtins.readFile ./nix/flatpak/static-validate.sh;
        };

        flatpakVersionParityCheckApp = pkgs.writeShellApplication {
          name = "flatpak-version-parity-check";
          runtimeInputs = [
            pkgs.coreutils
            pkgs.gnused
            pkgs.jq
            pkgs.libxml2
          ];
          text = builtins.readFile ./nix/flatpak/version-parity.sh;
        };

        bumpVersionApp = pkgs.writeShellApplication {
          name = "bump-version";
          runtimeInputs = [
            pkgs.coreutils
            pkgs.gnused
          ];
          text = builtins.readFile ./nix/bump-version.sh;
        };

        flatpakLocalManifestCheckApp = pkgs.writeShellApplication {
          name = "flatpak-local-manifest-check";
          runtimeInputs = [ pkgs.coreutils ];
          text = builtins.readFile ./nix/flatpak/local-manifest-validate.sh;
        };

        flatpakToolApp = pkgs.writeShellApplication {
          name = "sce-flatpak";
          runtimeInputs = flatpakToolRuntimeInputs;
          text = ''
            export SCE_FLATPAK_RELEASE_MANIFEST="${flatpakManifest.releaseManifest}"
            export SCE_FLATPAK_LOCAL_MANIFEST_TEMPLATE="${flatpakManifest.localManifestTemplate}"
            export SCE_FLATPAK_COMMIT_MANIFEST_TEMPLATE="${flatpakManifest.commitManifestTemplate}"
            export SCE_FLATPAK_LOCAL_PATH_PLACEHOLDER="${flatpakManifest.localPathPlaceholder}"
            export SCE_FLATPAK_COMMIT_PLACEHOLDER="${flatpakManifest.commitPlaceholder}"
            export SCE_FLATPAK_STATIC_CHECK="${flatpakStaticCheckApp}/bin/flatpak-static-check"
            export SCE_FLATPAK_VERSION_PARITY_CHECK="${flatpakVersionParityCheckApp}/bin/flatpak-version-parity-check"
            export SCE_FLATPAK_LOCAL_MANIFEST_CHECK="${flatpakLocalManifestCheckApp}/bin/flatpak-local-manifest-check"
            exec ${pkgs.bash}/bin/bash ${./packaging/flatpak/sce-flatpak.sh} "$@"
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

        flatpakCargoSources = import ./nix/flatpak/cargo-sources.nix {
          inherit pkgs;
          flatpakBuilderToolsSrc = flatpak-builder-tools;
          cargoLock = ./cli/Cargo.lock;
          checkedInJson = ./packaging/flatpak/cargo-sources.json;
        };

        flatpakStaticValidationCheck = pkgs.runCommand "flatpak-static-validation"
          {
            nativeBuildInputs = [ flatpakStaticCheckApp ];
          }
          ''
            set -euo pipefail

            cp -r "${flatpakPackagingSrc}" ./repo
            chmod -R u+w ./repo
            cd ./repo

            flatpak-static-check --repo-root "$PWD"

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

              # Copy only the Pkl authoring inputs and generated outputs that
              # this parity check reads, so unrelated repository changes do not
              # invalidate the cheap generated-output drift check.
              cp -r "${pklParitySrc}" ./repo
              chmod -R u+w ./repo
              cd ./repo

              export GIT_PAGER=cat

              tmp_dir="$(mktemp -d)"
              cleanup() {
                rm -rf "$tmp_dir"
              }
              trap cleanup EXIT

              pkl eval config/pkl/renderers/metadata-coverage-check.pkl -x summary >/dev/null
              pkl eval config/pkl/renderers/portable-execution-profile-check.pkl -x summary >/dev/null
              pkl eval config/pkl/renderers/instruction-unit-validator-check.pkl -x summary >/dev/null
              pkl eval -m "$tmp_dir" config/pkl/generate.pkl >/dev/null

              paths=(
                "config/.opencode/agent"
                "config/.opencode/command"
                "config/.opencode/skills"
                "config/.opencode/lib"
                "config/.opencode/plugins"
                "config/.opencode/opencode.json"
                "config/automated/.opencode/agent"
                "config/automated/.opencode/command"
                "config/automated/.opencode/skills"
                "config/automated/.opencode/lib"
                "config/automated/.opencode/plugins"
                "config/automated/.opencode/opencode.json"
                "config/.claude/agents"
                "config/.claude/commands"
                "config/.claude/skills"
                "config/.claude/hooks/run-sce-or-show-install-guidance.sh"
                "config/.claude/settings.json"
                "config/.pi/prompts"
                "config/.pi/skills"
                "config/.pi/extensions"
                "config/schema/sce-config.schema.json"
                ".opencode/agent"
                ".opencode/command"
                ".opencode/skills"
                ".claude/agents"
                ".claude/commands"
                ".claude/skills"
                ".pi/prompts"
                ".pi/skills"
                "templates"
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

        mkCopiedSourceCheck =
          { name
          , src
          , workdir
          , nativeBuildInputs
          , beforeCheck ? ""
          , checkCommand
          }:
          pkgs.runCommand name
            {
              inherit nativeBuildInputs;
            }
            ''
              set -euo pipefail

              cp -r "${src}" ./repo
              chmod -R u+w ./repo
              cd ./repo/${workdir}

              ${beforeCheck}
              ${checkCommand}

              mkdir -p "$out"
            '';

        mkBunCheck =
          { name
          , src
          , workdir
          , testCommand
          , beforeCheck ? ""
          }:
          mkCopiedSourceCheck {
            inherit name src workdir beforeCheck;
            nativeBuildInputs = [ pkgs.bun ];
            checkCommand = testCommand;
          };

        mkBiomeCheck =
          { name
          , src
          , workdir
          , mode
          }:
          mkCopiedSourceCheck {
            inherit name src workdir;
            nativeBuildInputs = [ pkgs.biome ];
            checkCommand = "biome check --${mode}-enabled=false .";
          };

        configLibBunTests = mkBunCheck {
          name = "config-lib-bun-tests";
          src = configLibBashPolicySrc;
          workdir = "config/lib";
          beforeCheck = ''
            cp -r "${configLibBashPolicyDeps}/node_modules" ./
          '';
          testCommand = "bun test";
        };

        configLibBiomeCheck = mkBiomeCheck {
          name = "config-lib-biome-check";
          src = configLibBashPolicySrc;
          workdir = "config/lib";
          mode = "formatter";
        };

        configLibBiomeFormat = mkBiomeCheck {
          name = "config-lib-biome-format";
          src = configLibBashPolicySrc;
          workdir = "config/lib";
          mode = "linter";
        };

        npmTests = mkBunCheck {
          name = "npm-bun-tests";
          src = npmSrc;
          workdir = "npm";
          testCommand = "bun test ./test/*.test.js";
        };

        npmBiomeCheck = mkBiomeCheck {
          name = "npm-biome-check";
          src = npmSrc;
          workdir = "npm";
          mode = "formatter";
        };

        npmBiomeFormat = mkBiomeCheck {
          name = "npm-biome-format";
          src = npmSrc;
          workdir = "npm";
          mode = "linter";
        };

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

        nativePortabilityAuditCheck =
          pkgs.runCommand "native-portability-audit-check"
            { }
            ''
              set -euo pipefail

              audit=${./nix/release/native-portability-audit.sh}

              cat > ./fake-strings <<EOF
              #!${pkgs.bash}/bin/bash
              set -euo pipefail

              if [[ "\$1" != "-a" ]]; then
                exit 2
              fi

              cat "\$2"
              EOF
              chmod +x ./fake-strings

              cat > ./fake-readelf <<EOF
              #!${pkgs.bash}/bin/bash
              set -euo pipefail
              exit 0
              EOF
              chmod +x ./fake-readelf

              printf 'portable binary fixture\n' > ./linux-clean.bin
              NATIVE_PORTABILITY_AUDIT_READELF="$PWD/fake-readelf" NATIVE_PORTABILITY_AUDIT_STRINGS="$PWD/fake-strings" bash "$audit" --platform linux --binary ./linux-clean.bin

              printf 'bad /nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-libbad-1/lib/libbad.so reference\n' > ./linux-bad.bin
              if NATIVE_PORTABILITY_AUDIT_READELF="$PWD/fake-readelf" NATIVE_PORTABILITY_AUDIT_STRINGS="$PWD/fake-strings" bash "$audit" --platform linux --binary ./linux-bad.bin > ./linux-bad.out 2> ./linux-bad.err; then
                printf 'expected Linux forbidden-reference fixture to fail\n' >&2
                exit 1
              fi
              grep -F 'Native binary portability audit failed:' ./linux-bad.err >/dev/null
              grep -F '/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-libbad-1/lib/libbad.so' ./linux-bad.err >/dev/null

              cat > ./fake-otool <<EOF
              #!${pkgs.bash}/bin/bash
              set -euo pipefail

              if [[ "\$1" != "-L" ]]; then
                exit 2
              fi

              case "\$2" in
                *macos-clean*)
                  cat <<'OUT'
              ./macos-clean.bin:
                /usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1336.0.0)
              OUT
                  ;;
                *macos-bad*)
                  cat <<'OUT'
              ./macos-bad.bin:
                /nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-libiconv-50/lib/libiconv.2.dylib (compatibility version 7.0.0, current version 7.0.0)
                /usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1336.0.0)
              OUT
                  ;;
                *)
                  exit 3
                  ;;
              esac
              EOF
              chmod +x ./fake-otool

              printf 'macos clean fixture\n' > ./macos-clean.bin
              NATIVE_PORTABILITY_AUDIT_OTOOL="$PWD/fake-otool" bash "$audit" --platform macos --binary ./macos-clean.bin

              printf 'macos bad fixture\n' > ./macos-bad.bin
              if NATIVE_PORTABILITY_AUDIT_OTOOL="$PWD/fake-otool" bash "$audit" --platform macos --binary ./macos-bad.bin > ./macos-bad.out 2> ./macos-bad.err; then
                printf 'expected macOS forbidden-reference fixture to fail\n' >&2
                exit 1
              fi
              grep -F 'Native binary portability audit failed:' ./macos-bad.err >/dev/null
              grep -F '/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-libiconv-50/lib/libiconv.2.dylib' ./macos-bad.err >/dev/null

              mkdir -p "$out"
            '';

        # Linux-only: audit the real static-musl release binary for forbidden
        # /nix/store runtime references. Unlike nativePortabilityAuditCheck
        # (a fixture-based unit test of the audit script), this inspects the
        # actual sceReleasePackage binary.
        releasePortabilityAuditCheck =
          pkgs.runCommand "sce-release-portability-audit"
            {
              nativeBuildInputs = [
                pkgs.binutils
                pkgs.coreutils
              ];
            }
            ''
              set -euo pipefail

              bash ${./nix/release/native-portability-audit.sh} \
                --platform linux \
                --binary ${sceReleasePackage}/bin/sce

              mkdir -p "$out"
            '';

        # Explicit long-running validation tier. Its primary member is the
        # static-musl release build; on Linux it also forces the release
        # portability audit over the real binary. Building this aggregate
        # (nix build .#ci-checks) is the expensive command, keeping
        # nix flake check fast (it never builds .#sce-release).
        ciChecks =
          pkgs.runCommand "sce-ci-checks"
            { }
            ''
              set -euo pipefail

              mkdir -p "$out"
              ln -s ${sceReleasePackage} "$out/sce-release"
              ${pkgs.lib.optionalString pkgs.stdenv.isLinux ''
                ln -s ${releasePortabilityAuditCheck} "$out/release-portability-audit"
              ''}
            '';

        sceApp = {
          type = "app";
          program = "${scePackage}/bin/sce";
          meta = {
            description = "Run the packaged sce CLI (native)";
          };
        };

        sceReleaseApp = {
          type = "app";
          program = "${sceReleasePackage}/bin/sce";
          meta = {
            description = "Run the packaged sce release CLI (static musl on Linux, native on Darwin)";
          };
        };
      in
      {
        packages = {
          sce = scePackage;
          sce-release = sceReleasePackage;
          ci-checks = ciChecks;
          bun = bunPackage;
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
              cargoDepsArgs
              // {
                pname = "sce-cli-fmt";
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
            native-portability-audit = nativePortabilityAuditCheck;
          }
          // pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
            flatpak-static-validation = flatpakStaticValidationCheck;
            cargo-sources-parity = flatpakCargoSources.parityCheck;
            flatpak-manifest-parity = flatpakManifest.parityCheck;
          };

        apps =
          {
            sce = sceApp;
            default = sceApp;
            sce-release = sceReleaseApp;

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

            native-portability-audit = {
              type = "app";
              program = "${nativePortabilityAuditApp}/bin/native-portability-audit";
              meta = {
                description = "Audit native release binaries for forbidden Nix store runtime references";
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

            bump-version = {
              type = "app";
              program = "${bumpVersionApp}/bin/bump-version";
              meta = {
                description = "Bump the checked-in version in .version, Cargo.toml, Cargo.lock, npm package.json, and Flatpak metainfo";
              };
            };

          }
          // pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
            sce-flatpak = {
              type = "app";
              program = "${flatpakToolApp}/bin/sce-flatpak";
              meta = {
                description = "Flatpak packaging umbrella (validate, prepare-local-manifest, release-package, release-bundle)";
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

            flatpak-static-check = {
              type = "app";
              program = "${flatpakStaticCheckApp}/bin/flatpak-static-check";
              meta = {
                description = "Static Flatpak packaging validation (manifest, banned snippets, cargo-sources, metainfo)";
              };
            };

            flatpak-version-parity-check = {
              type = "app";
              program = "${flatpakVersionParityCheckApp}/bin/flatpak-version-parity-check";
              meta = {
                description = "Validate sce release version parity across .version, Cargo.toml, npm package.json, and Flatpak metainfo";
              };
            };

            flatpak-local-manifest-check = {
              type = "app";
              program = "${flatpakLocalManifestCheckApp}/bin/flatpak-local-manifest-check";
              meta = {
                description = "Validate a generated local-checkout Flatpak manifest";
              };
            };

            regenerate-cargo-sources = {
              type = "app";
              program = "${flatpakCargoSources.regenerateApp}/bin/regenerate-cargo-sources";
              meta = {
                description = "Regenerate packaging/flatpak/cargo-sources.json from cli/Cargo.lock";
              };
            };

            regenerate-flatpak-manifest = {
              type = "app";
              program = "${flatpakManifest.regenerateApp}/bin/regenerate-flatpak-manifest";
              meta = {
                description = "Regenerate packaging/flatpak/dev.crocoder.sce.yml from nix/flatpak/manifest.nix";
              };
            };
          };

        devShells.default = pkgs.mkShell {
          packages = defaultDevShellPackages;

          shellHook = defaultDevShellHook;
        };

        # Opt-in shell exposing the repository-built CLI as `sce` while keeping
        # the default development shell fast.
        devShells.sce = pkgs.mkShell {
          packages = defaultDevShellPackages ++ [ scePackage ];

          shellHook = ''
            ${defaultDevShellHook}
            echo "- sce: $(sce version)"
          '';
        };

        # Opt-in shell layering the Turso CLI on top of the default tools, so
        # database work can pull Turso in explicitly via `nix develop .#database`.
        devShells.database = pkgs.mkShell {
          packages = defaultDevShellPackages ++ [ tursoPackage ];

          shellHook = ''
            ${defaultDevShellHook}
            echo "- turso: $(version_of turso)"
          '';
        };
      }
    );
}
