# Plan: claude-workflow-single-skill-packages

## Change summary

Replace Claude's multi-skill workflow rendering with one self-contained package per workflow. Each of `/change-to-plan`, `/next-task`, `/validate`, and `/commit` will have exactly one generated command entrypoint, one corresponding workflow skill, and one `references/output.md` document defining that workflow's user-visible Markdown return shapes. The skill's `SKILL.md` owns the complete phase sequence, internal status handling, user wait/resume gates, and final response rendering.

This removes Claude's inter-skill handoff seam instead of trying to make structured phase results continue reliably. Canonical Pi and OpenCode workflows remain decomposed into their current eight skills, while Claude renders four workflow-level skills and no Claude phase-skill packages.

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the check that proves it. `/validate` runs these checks; no task in the stack performs final validation.

- [ ] AC1: Generated Claude contains exactly four workflow command entrypoints and four workflow skill packages, with every command invoking exactly one corresponding skill and no Claude command sequencing multiple skills.
  - Validate: generate a temporary payload with `tmp="$(mktemp -d)"; nix run .#pkl-generate -- "$tmp"`; inspect `config/.claude/commands/` and `config/.claude/skills/` and assert a one-entrypoint-to-one-skill mapping for `change-to-plan`, `next-task`, `validate`, and `commit`.
- [ ] AC2: Every generated Claude workflow skill is self-contained: its `SKILL.md` owns all former phase behavior, status branching, approval or clarification waits, same-session resume behavior, plan/evidence writes, and workflow continuation rules without invoking any other SCE skill.
  - Validate: inspect all four generated Claude `SKILL.md` files; confirm no `Skill` invocation targets a phase or sibling `sce-*` package and verify the former phase boundaries and terminal statuses remain represented inside the owning workflow skill.
- [ ] AC3: Every generated Claude workflow skill package contains exactly two files—`SKILL.md` and `references/output.md`—and `output.md` is the package's single reference document defining all and only that workflow's user-visible Markdown return and gate layouts.
  - Validate: `find "$tmp/config/.claude/skills" -type f | sort`; assert exactly one `references/output.md` per package and no other reference, machine-contract, plan-template, result-contract, or sibling-package dependency files; inspect each `SKILL.md` branch against `output.md` and confirm every user-visible outcome names and follows one defined Markdown shape.
- [ ] AC4: Claude preserves the established workflow behavior despite package collapse: `/change-to-plan` loads context before authoring and supports clarification/revision; `/next-task` reviews one task, presents the implementation gate, waits for approval, executes, records evidence, and synchronizes task context; `/validate` validates only complete plans and synchronizes plan context only after success; `/commit` preserves regular proposal and explicit bypass modes.
  - Validate: inspect the generated commands, four composite skills, and four output references against `config/pkl/base/workflow-{change-to-plan,next-task,validate,commit}.pkl` and `workflow-context-sync.pkl`; verify every canonical gate, branch, and user-facing layout has one Claude owner.
- [ ] AC5: Exact metadata coverage enforces the four-command/four-skill/two-files-per-skill Claude inventory and rejects stale phase-skill packages or extra references.
  - Validate: `nix develop -c pkl eval config/pkl/renderers/metadata-coverage-check.pkl`; `nix run .#pkl-check-generated`.
- [ ] AC6: OpenCode and Pi retain their canonical eight-skill packages, YAML phase contracts, command behavior, and generated content byte-for-byte.
  - Validate: generate payloads from the implementation base and working tree, then run `diff -rq` over their `config/.opencode` and `config/.pi` trees; only Claude workflow Markdown may differ.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of which criterion they map to.

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- Update `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, and `context/context-map.md` to describe Claude's four workflow-level skill packages and the one-output-reference rule.
- Update `context/sce/shared-context-plan-workflow.md`, `context/sce/shared-context-code-workflow.md`, and `context/sce/atomic-commit-workflow.md` only where target-specific ownership needs clarification; preserve canonical Pi/OpenCode phase semantics.
- Add or update an architecture decision that supersedes the claim in `context/decisions/2026-07-27-workflow-oriented-pkl-generation.md` that Claude receives the same eight generated skill packages as Pi and OpenCode.

## Constraints and non-goals

- **In scope:** Claude renderer composition, four command entrypoints with one-to-one command-to-skill routing, four workflow-level `SKILL.md` documents, one Markdown-shape `references/output.md` per workflow, exact Claude inventory coverage, and affected generation documentation.
- **Out of scope:** Canonical Pi/OpenCode workflow decomposition, status or approval semantics, application runtime code, hooks/settings/plugins, generated repository target trees, or changes to the project-root `.pi/` baseline.
- **Constraints:** Author behavior from canonical `config/pkl/base/workflow-*.pkl` and `workflow-context-sync.pkl`; implement the collapse only in `config/pkl/renderers/`; preserve all user decision points and same-session resume behavior; keep each generated Claude workflow package independent of every sibling package.
- **Non-goal:** Preserve machine-readable inter-phase result contracts inside Claude. Those contracts exist to cross phase boundaries that the composite Claude skills remove; retain only internal state and branching instructions needed to execute the workflow correctly.

## Assumptions

- The four Claude workflow skill slugs may be normalized to workflow-level names such as `sce-change-to-plan`, `sce-next-task`, `sce-validate`, and `sce-commit`; exact names follow existing naming conventions as long as the command mapping is unambiguous.
- `references/output.md` may contain multiple exact Markdown layouts for different statuses or user gates, but it is the package's only reference file and contains no inter-skill machine handoff contract.
- The composite skill directly renders user-facing workflow output according to `references/output.md`. The command is a thin entrypoint and does not consume a structured result from the skill after invocation.
- Wait points remain real turn boundaries inside the composite skill: bootstrap, clarification, plan revision, implementation approval, blocked/incomplete execution, and failed validation resume in the same workflow session where the canonical behavior requires it.
- `/commit` already has one behavioral skill, but it is normalized to the same workflow-package shape and one `output.md` reference so Claude's inventory follows one rule.

## Task stack

- [x] T01: `Collapse Claude workflows into four self-contained skills` (status:done)
  - Task ID: T01
  - Goal: Render each Claude workflow as one command entrypoint, one complete workflow skill, and one reference defining its Markdown output shapes while preserving canonical behavior and leaving Pi/OpenCode unchanged.
  - Boundaries (in/out of scope): In — `config/pkl/renderers/claude-content.pkl`, `config/pkl/renderers/claude-workflow-results.pkl` or a replacement focused Claude composition module, `config/pkl/renderers/claude-metadata.pkl`, `config/pkl/renderers/metadata-coverage-check.pkl`, and directly affected Pkl generation documentation. Out — canonical workflow package edits for Claude-only behavior, Pi/OpenCode renderers or baselines, CLI runtime code, and unrelated generated assets.
  - Dependencies: none
  - Done when: Temporary generation emits four Claude command entrypoints and four two-file workflow skill packages; each entrypoint routes to one skill; each skill contains all former phase behavior and invokes no other SCE skill; each package has only `SKILL.md` plus `references/output.md`; every user-visible branch follows a Markdown shape defined by that reference; exact coverage rejects stale phase skills/references; base/current OpenCode and Pi trees are byte-identical.
  - Verification notes (commands or checks): `nix develop -c pkl eval config/pkl/renderers/{claude-content,metadata-coverage-check}.pkl`; generate a temporary payload and inspect command/skill/reference inventories plus workflow gates; compare implementation-base/current OpenCode and Pi payload trees; `nix run .#pkl-check-generated`; `git diff --check -- config/pkl context/plans/claude-workflow-single-skill-packages.md`.
  - Completed: 2026-07-29
  - Files changed: `config/pkl/README.md`, `config/pkl/renderers/claude-content.pkl`, `config/pkl/renderers/claude-metadata.pkl`, `config/pkl/renderers/claude-workflow-results.pkl`, `config/pkl/renderers/metadata-coverage-check.pkl`, `context/plans/claude-workflow-single-skill-packages.md`
  - Evidence: Both renderer evaluations passed; temporary generation produced four Claude commands and four two-file workflow packages with exact one-to-one routes and no phase-skill slug references; HEAD/current generated OpenCode and Pi trees were byte-identical; `nix run .#pkl-check-generated` passed with 72 files and inventory SHA-256 `8cbbdcdc903e677916de1ed8cbe6b90ec21666d04afe677bbb4514c6ee7a359c`; scoped `git diff --check` passed.
  - Notes: Claude now keeps phase statuses as internal state within workflow-level skills; canonical Pi/OpenCode phase packages and contracts were not modified.

## Open questions

None. The requested architecture is explicit: Claude gets one skill and one output reference per workflow, while canonical phase decomposition remains available to Pi and OpenCode.
