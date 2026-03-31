# Plan: nix-flake-check-verification-guidance

## Change summary

Replace broad direct Cargo verification guidance with repository-level `nix flake check` guidance wherever the repo currently tells contributors or generated agent artifacts to use Cargo test/fmt verification commands directly.

Preserve the distinction between verification and autofix flows:

- verification/check commands should prefer `nix flake check`
- autofix formatting commands may still use `cargo fmt`

The rollout must update canonical authoring sources first, then regenerate or align derived artifacts so repo docs, generated skills/agents, and bash-policy examples all describe the same current-state workflow.

## Success criteria

- Repo guidance no longer recommends direct Cargo verification commands where `nix flake check` is the intended repository-level validation path.
- Direct verification references such as `cargo test`, `cargo test <pattern>`, and `cargo fmt --check` are replaced or reframed to use `nix flake check` in in-scope guidance/docs.
- Autofix guidance continues to allow `cargo fmt` where the intent is formatting, not verification.
- Canonical generated-content sources are updated so generated OpenCode/Claude artifacts stay aligned after regeneration.
- Bash-policy messaging/tests remain consistent with the documented verification-vs-autofix policy.
- Final validation confirms no in-scope guidance drift remains.

## Constraints and non-goals

- In scope: canonical guidance sources, generated-content authoring sources, focused durable context files, and bash-policy documentation/tests needed to keep the verification policy consistent.
- In scope: replacing broad verification guidance with `nix flake check` when the command intent is repo validation.
- In scope: preserving `cargo fmt` for explicit autofix/formatting workflows.
- Out of scope: removing all direct Cargo invocations from the repository regardless of intent.
- Out of scope: changing build/install commands like `cargo build`, `cargo run`, or `cargo install` unless they are incorrectly presented as verification commands.
- Out of scope: rewriting completed plan history except where an active plan must stay internally coherent.
- Every executable task must remain one coherent commit unit.

## Task stack

- [x] T01: `Update canonical verification guidance to prefer nix flake check` (status:done)
  - Task ID: T01
  - Goal: Update the primary human-facing current-state guidance so repository validation points to `nix flake check`, while keeping `cargo fmt` only for autofix use.
  - Boundaries (in/out of scope): In - `AGENTS.md`, relevant root context files, and focused current-state docs that currently recommend direct Cargo verification commands. Out - generated artifacts, code-level enforcement logic, and unrelated command guidance.
  - Done when: Core repo guidance consistently says verification uses `nix flake check`, any surviving `cargo fmt` references are explicitly autofix-oriented, and touched current-state docs no longer imply direct Cargo verification is preferred.
  - Verification notes (commands or checks): Manual parity review across `AGENTS.md`, `context/overview.md`, `context/glossary.md`, `context/patterns.md`, and `context/cli/placeholder-foundation.md` against the intended verification policy.
  - Completed: 2026-03-31
  - Files changed: `AGENTS.md`, `context/cli/placeholder-foundation.md`, `context/glossary.md`, `context/patterns.md`
  - Evidence: Manual parity review across the in-scope guidance files; direct Cargo verification guidance removed from the touched current-state docs while explicit `cargo fmt` autofix guidance was preserved and root context now records the repo-level verification preference.

- [x] T02: `Align canonical generated-content sources with the verification policy` (status:done)
  - Task ID: T02
  - Goal: Update the canonical authored sources that generate agent/skill content so generated artifacts describe `nix flake check` as the default verification path instead of direct Cargo test/fmt-check examples.
  - Boundaries (in/out of scope): In - canonical Pkl/shared-content sources and any other non-generated authoring files that feed generated skills/commands. Out - hand-editing generated outputs without updating their canonical source.
  - Done when: The canonical generation sources encode the verification-vs-autofix distinction correctly, and regeneration would yield generated skill/agent content that no longer recommends direct Cargo verification commands by default.
  - Verification notes (commands or checks): Manual parity review of `config/pkl/base/shared-content.pkl`, `config/pkl/base/shared-content-automated.pkl`, and any touched generated-surface source ownership notes; if generated outputs are updated in the implementation task, include `nix run .#pkl-check-generated`.
  - Completed: 2026-03-31
  - Files changed: `config/pkl/base/shared-content.pkl`, `config/pkl/base/shared-content-automated.pkl`
  - Evidence: Manual parity review confirmed both canonical `sce-validation` content bodies now prefer `nix flake check` as the repository-level verification flow and explicitly preserve `cargo fmt` as autofix-only guidance; no generated outputs were hand-edited in this task.

- [x] T03: `Repair policy examples and focused contract docs that still show direct cargo verification` (status:done)
  - Task ID: T03
  - Goal: Update the remaining in-scope policy examples, tests, and focused context docs so they reflect the same rule: `nix flake check` for verification, `cargo fmt` only for autofix.
  - Boundaries (in/out of scope): In - `.sce/config.json` if messaging needs refinement, `config/lib/bash-policy-plugin/bash-policy-runtime.test.ts`, `context/sce/bash-tool-policy-enforcement-contract.md`, and focused context contract files whose verification examples still show direct Cargo test/fmt-check commands. Out - unrelated feature docs that merely preserve historical evidence in completed plans.
  - Done when: In-scope examples/tests/docs no longer contradict the verification policy, nested-shell examples still cover blocked direct Cargo verification patterns, and any retained direct Cargo formatting example is clearly autofix-only.
  - Verification notes (commands or checks): Manual parity review of touched policy/test/context files; targeted check that bash-policy examples still distinguish blocked `cargo test` / `cargo fmt --check` from allowed autofix `cargo fmt` semantics.
  - Completed: 2026-03-31
  - Files changed: `.sce/config.json`, `config/lib/bash-policy-plugin/bash-policy-runtime.test.ts`, `context/sce/bash-tool-policy-enforcement-contract.md`, `context/sce/agent-trace-commit-msg-coauthor-policy.md`, `context/sce/agent-trace-payload-builder-validation.md`, `context/sce/agent-trace-schema-adapter.md`, `context/sce/cli-security-hardening-contract.md`, `context/sce/agent-trace-retry-queue-observability.md`, `context/sce/agent-trace-rewrite-trace-transformation.md`, `context/sce/agent-trace-reconciliation-schema-ingestion.md`, `context/sce/agent-trace-post-rewrite-local-remap-ingestion.md`, `context/sce/agent-trace-hosted-event-intake-orchestration.md`, `context/sce/agent-trace-post-commit-dual-write.md`, `context/sce/agent-trace-core-schema-migrations.md`
  - Evidence: `bun test -t "use-nix-flake-over-cargo"` passed in `config/lib/bash-policy-plugin/` (4 tests, 0 failures); manual parity review confirmed focused contract docs now point to `nix flake check`, while the remaining bash-policy examples intentionally retain blocked `cargo test` / `cargo fmt --check` cases and now state that direct `cargo fmt` remains autofix-only.

- [x] T04: `Run validation and cleanup for verification-guidance alignment` (status:done)
  - Task ID: T04
  - Goal: Validate the documentation/config/generation changes together and remove any remaining in-scope drift around verification guidance.
  - Boundaries (in/out of scope): In - generated-output parity if canonical generated sources changed, full repo validation, and final context-sync verification for touched docs/policy files. Out - new policy features beyond the scoped guidance alignment.
  - Done when: Required validation passes, in-scope verification guidance is coherent across canonical docs and generated-content sources, and no touched file still recommends direct Cargo verification contrary to repo policy.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; manual parity review across touched docs, policy files, and generated-output ownership boundaries.
  - Completed: 2026-03-31
  - Files changed: `config/.opencode/**`, `config/automated/.opencode/**`, `config/.claude/**`, `config/schema/sce-config.schema.json`, `context/plans/nix-flake-check-verification-guidance.md`
  - Evidence: Initial validation found stale generated skill outputs for `sce-validation`; regenerated outputs with `nix develop -c pkl eval -m . config/pkl/generate.pkl`, then `nix run .#pkl-check-generated` passed and `nix flake check` completed successfully on `x86_64-linux`.

## Validation Report

### Commands run
- `nix run .#pkl-check-generated` -> exit 1 initially; reported generated-output drift in `config/.opencode/skills/sce-validation/SKILL.md`, `config/automated/.opencode/skills/sce-validation/SKILL.md`, and `config/.claude/skills/sce-validation/SKILL.md`
- `nix develop -c pkl eval -m . config/pkl/generate.pkl` -> exit 0; regenerated generated OpenCode/Claude agents, commands, skills, plugin assets, and schema outputs from canonical sources
- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (`running 10 flake checks...`; warning only that incompatible non-local systems were omitted)

### Failed checks and follow-ups
- Initial `pkl-check-generated` failure was fixable and in scope: generated outputs were stale after canonical validation-guidance text changed in T02. Regenerated outputs from canonical Pkl sources, then reran parity and full flake validation successfully.

### Success-criteria verification
- [x] Repo guidance no longer recommends direct Cargo verification commands where `nix flake check` is the intended repository-level validation path -> confirmed by T01/T03 file updates plus passing parity after regeneration
- [x] Direct verification references such as `cargo test`, `cargo test <pattern>`, and `cargo fmt --check` are replaced or reframed to use `nix flake check` in in-scope guidance/docs -> confirmed by manual parity review of touched current-state docs and focused contract files; retained Cargo examples are intentional blocked-policy cases or targeted-debug exceptions
- [x] Autofix guidance continues to allow `cargo fmt` where the intent is formatting, not verification -> confirmed in `.sce/config.json`, `config/lib/bash-policy-plugin/bash-policy-runtime.test.ts`, and `context/sce/bash-tool-policy-enforcement-contract.md`
- [x] Canonical generated-content sources are updated so generated OpenCode/Claude artifacts stay aligned after regeneration -> confirmed by regenerated `config/.opencode/**`, `config/automated/.opencode/**`, and `config/.claude/**` outputs and passing `nix run .#pkl-check-generated`
- [x] Bash-policy messaging/tests remain consistent with the documented verification-vs-autofix policy -> confirmed by passing `bun test -t "use-nix-flake-over-cargo"` from T03 and current `.sce/config.json`/test messaging
- [x] Final validation confirms no in-scope guidance drift remains -> confirmed by passing parity and full flake validation after regeneration

### Residual risks
- None identified for the scoped verification-guidance alignment work.

## Open questions

- None. The user confirmed broad-scope replacement for verification guidance and clarified that every verification/check flow should prefer `nix flake check`, while autofix formatting should continue to use `cargo fmt`.
