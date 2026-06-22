# Shared Context Engineering (SCE)

[![crates.io](https://img.shields.io/crates/v/shared-context-engineering?logo=rust)](https://crates.io/crates/shared-context-engineering)
[![npm](https://img.shields.io/npm/v/%40crocoder-dev%2Fsce?logo=npm)](https://www.npmjs.com/package/@crocoder-dev/sce)
[![Nix CI](https://github.com/crocoder-dev/shared-context-engineering/actions/workflows/pr-ci.yml/badge.svg?branch=main)](https://github.com/crocoder-dev/shared-context-engineering/actions/workflows/pr-ci.yml)

Shared Context Engineering is AI-assisted software delivery with explicit, versioned context.

This repository contains the `sce` CLI, generated assistant configuration, and the shared `context/` memory used across SCE workflows.

## Documentation

- [Documentation site](https://sce.crocoder.dev/)
- [Getting started](https://sce.crocoder.dev/docs/getting-started)
- [Motivation](https://sce.crocoder.dev/docs/motivation)
- [GitHub repository](https://github.com/crocoder-dev/shared-context-engineering)

## Install the `sce` CLI

### Nix

```bash
nix run github:crocoder-dev/shared-context-engineering -- --help
```

To install it into your profile:

```bash
nix profile install github:crocoder-dev/shared-context-engineering
```

### Cargo

Published releases target the `shared-context-engineering` crate and install the `sce` binary.

```bash
cargo install shared-context-engineering --locked
```

Additional supported Cargo install paths:

```bash
cargo install --git https://github.com/crocoder-dev/shared-context-engineering shared-context-engineering --locked
cargo install --path cli --locked
```

### npm

Published npm releases target the `@crocoder-dev/sce` package and install the `sce` launcher.

```bash
npm install -g @crocoder-dev/sce
```

### Flatpak (Linux-only, source-built)

The `sce` CLI is also available as a **source-built** Flatpak package (`dev.crocoder.sce`)
for Linux. The Flatpak builds `sce` from source inside the Flatpak sandbox using the
Freedesktop SDK Rust extension — it does not wrap a prebuilt Nix, Cargo, or npm binary.

> **Release scope:** GitHub Releases now include both Flatpak source-manifest
> assets (packaging metadata for source builds) and source-built `.flatpak` bundle
> assets (pre-built Flatpak apps for direct install). The bundles are built from
> Rust source inside Flatpak — they are not prebuilt binaries, Nix packages, or
> Flathub submissions. Automatic Flathub publication and release-version bumping
> remain out of scope.

#### Prerequisites

- [Flatpak](https://flatpak.org/) and [flatpak-builder](https://docs.flatpak.org/en/latest/flatpak-builder.html)
  installed on your Linux system.
- The [Freedesktop SDK](https://docs.flatpak.org/en/latest/available-runtimes.html) runtime
  and SDK extension are downloaded automatically by flatpak-builder when needed.

#### Preferred path: Nix-backed workflow

If you are working from the repository checkout and have Nix available, use the
Nix-backed entrypoints. They provide Flatpak tooling, generate a local-checkout
manifest, and run validation without bypassing the Flatpak source build.

```bash
# Enter the dev shell with Flatpak tooling (Linux only)
nix develop

# Validate packaging metadata and local-source manifest generation
nix run .#flatpak-validate

# Generate a Flatpak manifest that builds from the current checkout
nix run .#flatpak-local-manifest

# Build the Flatpak from the current checkout
nix run .#flatpak-build -- --help
```

The `nix run .#flatpak-build` command accepts the same arguments as
`sce-flatpak build` (see `--help`). For example, to build and install
into your user installation:

```bash
nix run .#flatpak-build -- \
  --install --user \
  --install-deps-from=flathub
```

The default `nix flake check` runs lightweight static validation
(`flatpak-static-validation`) without a full Flatpak build. Full builds
are opt-in via `nix run .#flatpak-build` and require network access for
SDK runtime downloads.

#### GitHub Release source-manifest assets

Each `sce` GitHub Release also includes Flatpak source-manifest assets for
contributors or downstream packagers who want the release-pinned Flatpak build
inputs:

- `sce-v<version>-flatpak-manifest.tar.gz`
- `sce-v<version>-flatpak-manifest.tar.gz.sha256`
- `sce-v<version>-flatpak.json`

The tarball contains a deterministic top-level
`sce-v<version>-flatpak-manifest/` directory with the Flathub-style source
manifest and support files:

- `dev.crocoder.sce.yml`
- `dev.crocoder.sce.metainfo.xml`
- `cargo-sources.json`
- `git-host-bridge`

The packaged manifest pins its git source to the release commit without
mutating the checked-in manifest. The checksum and JSON metadata describe the
source-manifest package; they are separate from the signed native release
manifest consumed by npm and do not contain a prebuilt `sce` binary.

Maintainers can build the same asset set locally from a checked-out release
commit:

```bash
nix run .#release-flatpak-package -- \
  --version "$(tr -d '\n' < .version)" \
  --out-dir dist/flatpak
```

#### GitHub Release bundle assets

Each `sce` GitHub Release also includes source-built `.flatpak` bundles for
direct installation without cloning the repository or building from source:

- `sce-v<version>-x86_64.flatpak` + `.sha256` + `.json`
- `sce-v<version>-aarch64.flatpak` + `.sha256` + `.json`

Install directly from the release URL:

```bash
flatpak install --user \
  https://github.com/crocoder-dev/shared-context-engineering/releases/download/v<version>/sce-v<version>-x86_64.flatpak
```

These bundles are **source-built** — the Flatpak builds `sce` from Rust source
inside the Flatpak sandbox, not from a prebuilt Nix, Cargo, or npm binary.
They are not Flathub submissions and are published only as GitHub Release assets.

#### Direct flatpak-builder fallback

If you do not use Nix, you can use `sce-flatpak.sh` and `flatpak-builder` directly:

```bash
# Generate a local-checkout manifest
packaging/flatpak/sce-flatpak.sh prepare-local-manifest \
  --repo-root "$PWD" \
  --out-dir /tmp/sce-flatpak-manifest

# Build and install from the generated manifest
flatpak-builder \
  --force-clean \
  --install --user \
  --install-deps-from=flathub \
  /tmp/sce-flatpak-build/dev.crocoder.sce \
  /tmp/sce-flatpak-manifest/dev.crocoder.sce.yml
```

#### Release-source vs local-checkout override

- The **checked-in manifest** (`packaging/flatpak/dev.crocoder.sce.yml`) pins a
  release git commit as its source — this is the Flathub-ready release manifest.
- The **generated local manifest** replaces that git source with a `type: dir`
  source pointing at your local checkout directory. Cargo dependencies remain
  sourced from the checked-in `cargo-sources.json` and are still built offline
  inside Flatpak.

The local manifest is produced by `nix run .#flatpak-local-manifest` or
`sce-flatpak.sh prepare-local-manifest`. It is never committed; it lives in a
temporary or user-specified output directory.

#### Run the Flatpak

Once built and installed:

```bash
# Run sce from the command line
flatpak run dev.crocoder.sce -- --help

# Or with full examples
flatpak run dev.crocoder.sce version
flatpak run dev.crocoder.sce doctor
```

#### Host-git bridge

Some `sce` commands (`setup`, `doctor`, and hooks) require git access. Inside the
Flatpak sandbox, git is provided by the installed `/app/bin/git` wrapper
(`packaging/flatpak/git-host-bridge`), which delegates to the host system's git
via `flatpak-spawn --host git`. This requires the `--talk-name=org.freedesktop.Flatpak`
permission declared in the manifest.

Commands that rely on the user's git configuration, SSH keys, or credential helpers
work transparently as long as the host git session is configured.

#### Troubleshooting

| Symptom | Likely cause | Resolution |
|---|---|---|
| `sce` commands fail with git errors | Host git not available or misconfigured | Verify `flatpak-spawn --host git version` works outside the sandbox |
| Flatpak build fails on Cargo dependencies | Network unavailable for first build | Ensure `--install-deps-from=flathub` is used; the SDK runtime provides cached crate sources from `cargo-sources.json` |
| `flatpak-builder` not found | Missing host tooling | Install flatpak-builder via your system package manager or use the Nix dev shell |
| Validation reports missing files | Checkout missing Flatpak packaging files | Verify `packaging/flatpak/` exists and contains all expected files |

#### Uninstall and cleanup

```bash
# Remove the installed application
flatpak uninstall dev.crocoder.sce

# Remove build artifacts (adjust paths if you used custom --build-dir / --out-dir)
rm -rf /tmp/sce-flatpak-build
rm -rf /tmp/sce-flatpak-manifest
```

Built by [CroCoder](https://www.crocoder.dev/)
