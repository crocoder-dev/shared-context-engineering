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
          extensions = [ "rustfmt" ];
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

          nativeBuildInputs = [ rustToolchain ];
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
      }
    );
}
