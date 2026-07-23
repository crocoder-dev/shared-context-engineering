# Flake build, devShell, and CI performance

## Current structure

The root flake separates ordinary development from release-only work:

- `packages.sce` and `packages.default` build the native development CLI.
- `packages.sce-release` builds static musl on Linux and a commit-bearing native
  release on Darwin.
- `packages.ci-checks` is the explicit expensive tier. It forces the release
  package and, on Linux, audits the real release binary for forbidden
  `/nix/store/` runtime references.
- `nix flake check` runs the normal test/lint/format/parity/static checks without
  building `.#sce-release`.
- `devShells.default` provides development tools without `scePackage` or the
  Turso CLI. `devShells.database` adds Turso.
- `.github/workflows/pr-ci.yml` runs native package smoke tests in `nix-ci` and
  runs `.#ci-checks` separately in `release-validation` on every commit.

Git-commit embedding is release-only. Native packages and normal CLI check
derivations do not receive `SCE_GIT_COMMIT`; release packages do.

## Benchmark method

Measurements were taken on x86_64-linux with 8 logical cores, Nix 2.34.8, and a
shared developer store. "Cold" means the requested derivation was absent or
rebuilt while fetched sources, toolchains, and dependency closures remained
available; it does not mean an empty Nix store. Warm timings include evaluation
and cached-output lookup.

The detailed session record is retained in
`context/tmp/flake-speedup-benchmarks.md`; the durable results are summarized
here.

## Before and after

| Measure | Before | After | Result |
|---|---:|---:|---|
| Warm `nix flake check --no-build` | 5.494 s ± 0.294 s | 5.533 s ± 0.132 s | Evaluation unchanged. |
| Eval heap | ~465 MiB | ~465 MiB | Unchanged. |
| Warm `nix flake check` | 6.27 s | 7.00 s | Comparable; release output no longer forced. |
| Native final crate, deps cached | Not exposed as a package | 18.41 s wall / 12.83 s Cargo | Native development output is directly buildable. |
| Musl dependency compile | 4 m 06.2 s | 4 m 02 s | Release compile cost unchanged but isolated. |
| Musl final crate | 15.22 s wall / 12.07 s Cargo | 14.73 s Cargo | Comparable. |
| Warm default devShell | 1.54 s | 1.78 s | Comparable; cold shell no longer compiles SCE or Turso. |

The baseline native dependency graph remained the largest cold cost at about
8 minutes and 991 compile lines because it includes dev/test dependencies for
Crane test and Clippy derivations. The musl release graph remains about 4
minutes. This restructure isolates those costs; it does not make Rust dependency
compilation intrinsically faster.

## Validation evidence

Final validation on x86_64-linux confirmed:

- `nix flake check --print-build-logs` passed.
- Native `.#sce` and release `.#sce-release` produced distinct store outputs.
- Native `sce version` reported commit `unknown`; release reported the current
  short commit.
- The Linux release was static-pie and passed direct and `.#ci-checks`
  real-binary portability audits.
- A temporary empty commit left the native derivation and output paths unchanged.
- Native/check derivations contained no current commit value; the release
  derivation contained it.
- `nix develop -c true` required no SCE/Turso build, while
  `nix develop .#database -c turso --version` worked.
- Workflow actionlint, generated-output parity, embedded-asset SHA-256 tests,
  and public-output evaluation passed.

Darwin and aarch64 outputs were evaluation/wiring-verified from Linux; CI owns
native execution on Linux and macOS.

## Investigated no-ops

- Turso `default-features = false` was deferred. It could remove the Tantivy and
  mimalloc stacks, but the full native/musl build, size, database, and encryption
  validation matrix was not completed, so no dependency change was justified.
- Isolating the root `turso` and `flatpak-builder-tools` inputs was rejected.
  Turso's transitive flake inputs already follow root inputs, and
  `flatpak-builder-tools` is a small non-flake source input; warm evaluation was
  unchanged and isolation offered no material benefit.

## Remaining trade-off

The separate `release-validation` job intentionally builds `.#ci-checks` on
every PR and push. This preserves always-on release coverage but means CI still
pays the release compile cost in a distinct job. Path filtering is deferred.
