# Plan: Fix Human-Facing Registry READMEs

## Change Summary

Update the three human-facing README files that are published to registries and GitHub to have consistent branding, clear install instructions per channel, badges for CI/registry status, and links to the documentation website.

**Affected files:**
- `README.md` (root) - GitHub landing page, sce.crocoder.dev
- `cli/README.md` - crates.io, docs.rs
- `npm/README.md` - npmjs.com

**Out of scope:**
- `AGENTS.md` (AI agent instructions)
- `config/pkl/README.md` (internal documentation)

## Success Criteria

1. All three READMEs have consistent project branding and description
2. Each README has clear, channel-specific install instructions
3. All READMEs link to `https://sce.crocoder.dev/` documentation
4. Root README has badges for CI status, crates.io version, npm version
5. cli/README.md has badges for crates.io version and docs.rs
6. npm/README.md has badge for npm version
7. No outdated information (version references, links, commands)
8. Consistent structure across all three READMEs

## Constraints and Non-Goals

**Constraints:**
- Must not change Cargo.toml or package.json version fields
- Must preserve existing install command accuracy
- Must preserve links to GitHub repository

**Non-goals:**
- Changing AGENTS.md or internal docs
- Adding new installation channels (Homebrew is deferred)
- Modifying CI workflows or release processes
- Changing the documentation website content

## Task Stack

- [x] T01: `Update root README.md with badges and consistent branding` (status:done)
  - Task ID: T01
  - Goal: Add CI/registry badges, improve structure, ensure consistent branding with links to docs site
  - Boundaries (in/out of scope): In - badges, install instructions, links, structure. Out - version numbers in Cargo.toml/package.json, CI workflow changes.
  - Done when: Root README has badges (CI, crates.io, npm), clear install sections for each channel, links to docs site, consistent project description
  - Verification notes (commands or checks): `cat README.md` to verify badges render correctly, links are valid
  - Completed: 2026-04-13
  - Files changed: `README.md`, `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`
  - Evidence: README updated with badge + install sections; docs site and badge URLs returned HTTP 200; `nix run .#pkl-check-generated` passed; `nix flake check` completed without reported failures
  - Notes: Root README now documents the published Cargo crate name `shared-context-engineering`, the npm package name `@crocoder-dev/sce`, and links to `https://sce.crocoder.dev/`

- [x] T02: `Update cli/README.md with crates.io-specific content and badges` (status:done)
  - Task ID: T02
  - Goal: Add crates.io/docs.rs badges, link to documentation site, ensure consistent branding
  - Boundaries (in/out of scope): In - badges, docs link, install instructions, project description. Out - Cargo.toml changes, version bumps.
  - Done when: cli/README.md has crates.io version badge, docs.rs badge, link to sce.crocoder.dev, consistent project description with root README
  - Verification notes (commands or checks): `cat cli/README.md` to verify badges and links
  - Completed: 2026-04-13
  - Files changed: `cli/README.md`, `context/plans/fix-registry-readmes.md`
  - Evidence: `cli/README.md` now uses the published `shared-context-engineering` crate name, includes crates.io + docs.rs badges, links to `https://sce.crocoder.dev/`, and aligns branding with the root README; `nix run .#pkl-check-generated` passed; `nix flake check` completed without reported failures
  - Notes: Context sync classified this as verify-only; `context/overview.md`, `context/architecture.md`, and `context/glossary.md` were verified unchanged against the updated README

- [x] T03: `Update npm/README.md with npm-specific content and badges` (status:done)
  - Task ID: T03
  - Goal: Add npm version badge, link to documentation site, ensure consistent branding
  - Boundaries (in/out of scope): In - badges, docs link, install instructions, project description, supported platforms. Out - package.json changes, version bumps.
  - Done when: npm/README.md has npm version badge, link to sce.crocoder.dev, consistent project description with root README
  - Verification notes (commands or checks): `cat npm/README.md` to verify badges and links
  - Completed: 2026-04-13
  - Files changed: `npm/README.md`, `context/plans/fix-registry-readmes.md`
  - Evidence: `npm/README.md` now uses the published `@crocoder-dev/sce` package name, includes the npm badge, links to `https://sce.crocoder.dev/`, preserves supported platform mappings, and aligns branding with the root + CLI READMEs; `nix run .#pkl-check-generated` passed; `nix flake check` completed without reported failures
  - Notes: Context sync classified this as verify-only; `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, and `context/context-map.md` were verified unchanged against the updated npm README and existing npm distribution context

- [x] T04: `Validate all README links and badges render correctly` (status:done)
  - Task ID: T04
  - Goal: Final validation that all badges render and links work across all three READMEs
  - Boundaries (in/out of scope): In - verify badge URLs, verify documentation links, verify GitHub links. Out - external website changes.
  - Done when: All badge image URLs return valid images, all documentation links resolve, all GitHub links resolve
  - Verification notes (commands or checks): Manual verification of badge URLs, `curl -I` on key links, visual check in GitHub preview
  - Completed: 2026-04-13
  - Files changed: `context/plans/fix-registry-readmes.md`
  - Evidence: Badge image URLs returned `image/*`; documentation, GitHub, and docs.rs links resolved successfully; crates.io and npm registry pages were confirmed via web fetch despite direct `curl` requests returning environment-specific `403` responses; `nix run .#pkl-check-generated` passed; `nix flake check` completed without reported failures
  - Notes: Validation required no README content changes; existing badge Markdown and link targets were kept as-is. Context sync classified this as verify-only, and `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/patterns.md`, and `context/context-map.md` were reverified unchanged

## Open Questions

None - requirements are clear from user input.

## Validation Report

### Commands run
- Targeted URL validation for README badge images plus documentation/GitHub/docs.rs links -> success
- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (`running 13 flake checks...`; no failures reported)

### Additional verification
- `https://crates.io/crates/shared-context-engineering` -> registry page fetch succeeded via web fetch; direct `curl` returned `403` from this environment
- `https://www.npmjs.com/package/@crocoder-dev/sce` -> package page fetch succeeded via web fetch; direct `curl` returned `403` from this environment

### Failed checks and follow-ups
- None

### Success-criteria verification summary
- [x] All three READMEs have consistent project branding and description -> verified in `README.md`, `cli/README.md`, and `npm/README.md`
- [x] Each README has clear, channel-specific install instructions -> verified in each README's install section
- [x] All READMEs link to `https://sce.crocoder.dev/` documentation -> verified via targeted URL validation
- [x] Root README has badges for CI status, crates.io version, npm version -> verified via badge image checks in `README.md`
- [x] `cli/README.md` has badges for crates.io version and docs.rs -> verified via badge image checks in `cli/README.md`
- [x] `npm/README.md` has badge for npm version -> verified via badge image checks in `npm/README.md`
- [x] No outdated information (version references, links, commands) -> no validation issues found in the current README content
- [x] Consistent structure across all three READMEs -> verified by direct review of the current files

### Residual risks
- Registry page validation for crates.io and npm depends on fetch tooling that is not blocked by this environment's direct `curl` `403` responses
