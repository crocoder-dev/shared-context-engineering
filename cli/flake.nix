{
  description = "SCE CLI flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        src = builtins.path {
          path = ../.;
          name = "source";
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rustfmt"
            "clippy"
          ];
        };

        rustPlatform = pkgs.makeRustPlatform {
          cargo = rustToolchain;
          rustc = rustToolchain;
        };

        scePackage = rustPlatform.buildRustPackage {
          pname = "sce";
          version = "0.1.0";
          inherit src;
          sourceRoot = "source/cli";

          cargoLock = {
            lockFile = ../cli/Cargo.lock;
          };

          nativeBuildInputs = [
            rustToolchain
          ];

          nativeCheckInputs = [ pkgs.git ];
          doCheck = false;
        };
      in
      {
        packages = {
          sce = scePackage;
          default = scePackage;
        };

        apps.sce = {
          type = "app";
          program = "${scePackage}/bin/sce";
          meta = {
            description = "Run the packaged sce CLI";
          };
        };

        apps.clippy = {
          type = "app";
          program = toString (
            pkgs.writeShellScript "sce-clippy" ''
              exec ${rustToolchain}/bin/cargo clippy --manifest-path cli/Cargo.toml --all-targets --all-features "$@"
            ''
          );
          meta = {
            description = "Run clippy for the sce CLI crate";
          };
        };

        checks.cli-setup-command-surface = rustPlatform.buildRustPackage {
          pname = "sce-cli-setup-command-surface-check";
          version = "0.1.0";
          inherit src;
          sourceRoot = "source/cli";

          cargoLock = {
            lockFile = ../cli/Cargo.lock;
          };

          nativeBuildInputs = [ rustToolchain ];

          buildPhase = ''
            runHook preBuild
            runHook postBuild
          '';

          checkPhase = ''
            runHook preCheck

            cargo fmt --check
            cargo test command_surface::tests::help_text_mentions_setup_target_flags
            cargo test parser_routes_setup
            cargo test run_setup_reports

            runHook postCheck
          '';

          installPhase = ''
            runHook preInstall
            mkdir -p "$out"
            runHook postInstall
          '';
        };

        checks.cli-setup-integration = rustPlatform.buildRustPackage {
          pname = "sce-cli-setup-integration-check";
          version = "0.1.0";
          inherit src;
          sourceRoot = "source/cli";

          cargoLock = {
            lockFile = ../cli/Cargo.lock;
          };

          nativeBuildInputs = [ rustToolchain ];

          buildPhase = ''
            runHook preBuild
            runHook postBuild
          '';

          checkPhase = ''
            runHook preCheck

            export PATH="${pkgs.git}/bin:$PATH"
            cargo test --test setup_integration

            runHook postCheck
          '';

          installPhase = ''
            runHook preInstall
            mkdir -p "$out"
            runHook postInstall
          '';
        };

        checks.cli-clippy = rustPlatform.buildRustPackage {
          pname = "sce-cli-clippy-check";
          version = "0.1.0";
          inherit src;
          sourceRoot = "source/cli";

          cargoLock = {
            lockFile = ../cli/Cargo.lock;
          };

          nativeBuildInputs = [ rustToolchain ];

          buildPhase = ''
            runHook preBuild
            runHook postBuild
          '';

          checkPhase = ''
            runHook preCheck

            cargo clippy --all-targets --all-features

            runHook postCheck
          '';

          installPhase = ''
            runHook preInstall
            mkdir -p "$out"
            runHook postInstall
          '';
        };
      }
    );
}
