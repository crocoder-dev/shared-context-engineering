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

        version = pkgs.lib.strings.trim (builtins.readFile "${src}/.version");

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
          inherit version;
          inherit src;
          sourceRoot = "source/cli";

          cargoLock = {
            lockFile = ./Cargo.lock;
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

        checks = {
          cli-tests = rustPlatform.buildRustPackage {
            pname = "sce-cli-tests";
            inherit version;
            inherit src;
            sourceRoot = "source/cli";

            cargoLock = {
              lockFile = ./Cargo.lock;
            };

            nativeBuildInputs = [ rustToolchain ];
            nativeCheckInputs = [ pkgs.git ];

            buildPhase = ''
              runHook preBuild
              runHook postBuild
            '';

            checkPhase = ''
              runHook preCheck
              cargo test
              runHook postCheck
            '';

            installPhase = ''
              runHook preInstall
              mkdir -p "$out"
              runHook postInstall
            '';
          };

          cli-clippy = rustPlatform.buildRustPackage {
            pname = "sce-cli-clippy";
            inherit version;
            inherit src;
            sourceRoot = "source/cli";

            cargoLock = {
              lockFile = ./Cargo.lock;
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

          cli-fmt =
            pkgs.runCommand "sce-cli-fmt-check"
              {
                nativeBuildInputs = [ rustToolchain ];
              }
              ''
                cp -r "${src}/cli" ./cli
                chmod -R u+w ./cli
                cd ./cli
                cargo fmt --check
                mkdir -p "$out"
              '';
        };
      }
    );
}
