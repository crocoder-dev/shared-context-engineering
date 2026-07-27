# Decision: Generate Pkl Payloads Ephemerally for Builds and Packages

Date: 2026-07-27
Status: Accepted
Plan: `context/plans/generate-cli-assets-in-cargo-out-dir.md`

## Decision

- Keep `config/pkl/` and referenced `config/lib/` files as the canonical authoring sources.
- Do not commit generated `config/.opencode`, `config/.claude`, `config/.pi`, `config/schema/sce-config.schema.json`, or `cli/assets/generated` outputs.
- Repository and Nix/Crane Cargo builds evaluate `config/pkl/generate.pkl` directly into Cargo `OUT_DIR`; `cli/build.rs` stages non-Pkl hooks, schemas, and migrations there and production Rust embeds only from `OUT_DIR`.
- `nix run .#pkl-check-generated` validates metadata coverage, required outputs, forbidden repository paths, and deterministic two-pass temporary inventories rather than comparing committed snapshots.
- Published crates and source-built Flatpak packages may carry a checksummed packaging-only fallback generated from canonical Pkl in a temporary clean workspace. Downstream Pkl-free builds validate and copy that payload into their own `OUT_DIR`.

## Rationale

Committed target trees duplicated canonical Pkl ownership, made source filters and release helpers depend on snapshots, and allowed generated source artifacts to drift from the build that embedded them. Build-local generation keeps one authoring source and makes the exact payload part of each Cargo build. Packaging-only fallbacks preserve Pkl-free crates.io and Flatpak consumers without restoring repository mirrors or bundling a Pkl evaluator.

## Consequences

- Generated path names remain stable payload-relative install layouts, but they appear only under temporary generation roots, Cargo `OUT_DIR`, crate/Flatpak staging directories, and final `sce setup` destinations.
- Crane source filters include canonical Pkl, referenced plugin/extension sources, and static inputs; Pkl is a native build input for repository package/test/clippy derivations.
- Flatpak helpers prepare `cli-package-fallback/` before entering the sandbox, and manifests copy it to `cli/package-fallback` as a `type: dir` source.
- Contributor workflows inspect temporary output and must not run Pkl generation with repository root as the output directory.

## Superseded scope

This decision supersedes the checked-in generated-output ownership and snapshot-parity portions of `2026-07-27-workflow-oriented-pkl-generation.md`. That decision's workflow matrix, canonical Pkl ownership, renderer boundaries, and exact target inventory remain in force.
