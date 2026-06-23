{
  pkgs,
  flatpakBuilderToolsSrc,
  cargoLock,
  checkedInJson,
}:

let
  pythonEnv = pkgs.python3.withPackages (ps: [ ps.aiohttp ps.tomlkit ]);

  generatorScript = "${flatpakBuilderToolsSrc}/cargo/flatpak-cargo-generator.py";

  cargoSourcesJson = pkgs.stdenvNoCC.mkDerivation {
    pname = "sce-flatpak-cargo-sources";
    version = "1";

    dontUnpack = true;

    nativeBuildInputs = [ pythonEnv ];

    buildPhase = ''
      runHook preBuild
      cp ${cargoLock} ./Cargo.lock
      python3 ${generatorScript} ./Cargo.lock -o cargo-sources.json
      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall
      cp cargo-sources.json $out
      runHook postInstall
    '';

    outputHashMode = "flat";
    outputHashAlgo = "sha256";
    outputHash = "sha256-3xwpQ3TQ3/QSduJ5rKmpiU55heIEDOs2P89RFOZoqeo=";
  };

  regenerateApp = pkgs.writeShellApplication {
    name = "regenerate-cargo-sources";
    runtimeInputs = [ pkgs.coreutils ];
    text = ''
      set -euo pipefail
      if [ -z "''${SCE_REPO_ROOT:-}" ]; then
        SCE_REPO_ROOT="$(pwd)"
      fi
      target="$SCE_REPO_ROOT/packaging/flatpak/cargo-sources.json"
      if [ ! -e "$target" ]; then
        echo "regenerate-cargo-sources: expected $target to exist" >&2
        exit 1
      fi
      install -m 0644 ${cargoSourcesJson} "$target"
      echo "regenerate-cargo-sources: wrote $target" >&2
    '';
  };

  parityCheck = pkgs.runCommand "cargo-sources-parity"
    {
      nativeBuildInputs = [ pkgs.diffutils ];
    }
    ''
      set -euo pipefail
      if ! diff -u ${checkedInJson} ${cargoSourcesJson}; then
        echo "" >&2
        echo "cargo-sources-parity: packaging/flatpak/cargo-sources.json is out of sync with cli/Cargo.lock." >&2
        echo "  Regenerate with: nix run .#regenerate-cargo-sources" >&2
        exit 1
      fi
      mkdir -p "$out"
    '';

in {
  inherit cargoSourcesJson regenerateApp parityCheck;
}
