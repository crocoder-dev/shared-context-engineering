{
  pkgs,
  checkedInYaml,
  releaseCommit ? "b7f0fa002fca5f5320791ff5e4abfaadfcddf187",
}:

let
  inherit (pkgs) lib;

  yamlFormat = pkgs.formats.yaml { };

  appId = "dev.crocoder.sce";
  manifestFileName = "${appId}.yml";

  localPathPlaceholder = "__SCE_LOCAL_REPO_PATH__";
  commitPlaceholder = "__SCE_RELEASE_COMMIT__";

  releaseGitSource = commit: {
    type = "git";
    url = "https://github.com/crocoder-dev/shared-context-engineering.git";
    inherit commit;
  };

  localDirSource = repoPath: {
    type = "dir";
    path = repoPath;
  };

  baseManifest = sceSource: {
    id = appId;
    runtime = "org.freedesktop.Platform";
    runtime-version = "25.08";
    sdk = "org.freedesktop.Sdk";
    sdk-extensions = [ "org.freedesktop.Sdk.Extension.rust-stable" ];
    command = "sce";
    finish-args = [
      "--share=network"
      "--filesystem=home"
      "--talk-name=org.freedesktop.Flatpak"
      "--talk-name=org.freedesktop.secrets"
    ];
    modules = [
      {
        name = "sce";
        buildsystem = "simple";
        build-options = {
          append-path = "/usr/lib/sdk/rust-stable/bin";
          env = {
            CARGO_HOME = "/run/build/sce/cargo";
          };
        };
        build-commands = [
          ''bash ./scripts/prepare-cli-generated-assets.sh "$PWD"''
          "cargo --offline build --release --manifest-path cli/Cargo.toml --bin sce"
          "install -Dm755 cli/target/release/sce /app/bin/sce"
          "install -Dm755 packaging/flatpak/git-host-bridge /app/bin/git"
          "install -Dm644 packaging/flatpak/dev.crocoder.sce.metainfo.xml /app/share/metainfo/dev.crocoder.sce.metainfo.xml"
        ];
        sources = [
          sceSource
          {
            type = "file";
            path = "dev.crocoder.sce.metainfo.xml";
            dest = "packaging/flatpak";
          }
          {
            type = "file";
            path = "git-host-bridge";
            dest = "packaging/flatpak";
          }
          "cargo-sources.json"
        ];
      }
    ];
  };

  manifestFor =
    {
      sourceKind,
      commit ? null,
      repoPath ? null,
    }:
    let
      sceSource =
        if sourceKind == "release" then
          releaseGitSource releaseCommit
        else if sourceKind == "commit" then
          releaseGitSource (
            if commit == null then commitPlaceholder else commit
          )
        else if sourceKind == "local" then
          localDirSource (if repoPath == null then localPathPlaceholder else repoPath)
        else
          throw "manifest.nix: unknown sourceKind '${sourceKind}'";
    in
    baseManifest sceSource;

  releaseManifest = yamlFormat.generate manifestFileName (manifestFor { sourceKind = "release"; });
  localManifestTemplate = yamlFormat.generate "${appId}.local-template.yml" (
    manifestFor { sourceKind = "local"; }
  );
  commitManifestTemplate = yamlFormat.generate "${appId}.commit-template.yml" (
    manifestFor { sourceKind = "commit"; }
  );

  regenerateApp = pkgs.writeShellApplication {
    name = "regenerate-flatpak-manifest";
    runtimeInputs = [ pkgs.coreutils ];
    text = ''
      set -euo pipefail
      if [ -z "''${SCE_REPO_ROOT:-}" ]; then
        SCE_REPO_ROOT="$(pwd)"
      fi
      target="$SCE_REPO_ROOT/packaging/flatpak/${manifestFileName}"
      if [ ! -e "$target" ]; then
        echo "regenerate-flatpak-manifest: expected $target to exist" >&2
        exit 1
      fi
      install -m 0644 ${releaseManifest} "$target"
      echo "regenerate-flatpak-manifest: wrote $target" >&2
    '';
  };

  parityCheck =
    pkgs.runCommand "flatpak-manifest-parity"
      {
        nativeBuildInputs = [ pkgs.diffutils ];
      }
      ''
        set -euo pipefail
        if ! diff -u ${checkedInYaml} ${releaseManifest}; then
          echo "" >&2
          echo "flatpak-manifest-parity: packaging/flatpak/${manifestFileName} is out of sync with nix/flatpak/manifest.nix." >&2
          echo "  Regenerate with: nix run .#regenerate-flatpak-manifest" >&2
          exit 1
        fi
        mkdir -p "$out"
      '';
in
{
  inherit
    releaseManifest
    localManifestTemplate
    commitManifestTemplate
    regenerateApp
    parityCheck
    localPathPlaceholder
    commitPlaceholder
    ;
}
