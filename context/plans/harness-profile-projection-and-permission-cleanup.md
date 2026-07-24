# Plan: harness-profile-projection-and-permission-cleanup

## Change summary
Remove native Shared Context agent projections from Claude while retaining Claude's composed workflow commands and skills. Remove HTML execution-profile comments from every generated command/prompt surface without adding a replacement marker. Tighten manual OpenCode profile permissions so Shared Context Code allows Bash and Shared Context Plan blocks Bash, while OpenCode commands continue to carry explicit tool permissions, entry-skill metadata, required-skill metadata, and skill permission entries.

## Success criteria
- Claude generation and root mirrors contain no `shared-context-code` or `shared-context-plan` native agent files, and stale Claude agent files are rejected or otherwise prevented from surviving generation/parity checks.
- Claude commands remain profile-composed and keep capability-derived `allowed-tools`; Claude skills remain generated.
- No generated or root-mirrored OpenCode command, Claude command, or Pi prompt contains an HTML comment, including the removed `sce-execution-profile` marker, and no replacement marker is introduced.
- The manual OpenCode `Shared Context Code` profile renders `bash: allow` and the manual OpenCode `Shared Context Plan` profile renders `bash: block`.
- Every manual OpenCode workflow command retains an explicit `permission` block, `entry-skill`, ordered `skills`, and required-skill permission entries. Planning commands cannot enable Bash above the planning profile's blocked posture; code workflow permissions remain explicit and preserve narrower workflow/approval rules where applicable.
- Canonical Pkl model checks, structural validation, generated-output parity, and repository validation pass with updated deterministic projection counts and fixtures.
- Durable context describes Claude as composed-command/skill only, marker-free command/prompt composition, and the resulting OpenCode permission contract.

## Constraints and non-goals
- Planning and implementation remain separate; this plan does not authorize code changes.
- Pkl remains the canonical owner of generated harness configuration; generated files and root mirrors must not be edited as standalone sources.
- Do not remove Claude commands, Claude skills, Claude hooks/settings, OpenCode profile agents, Pi prompts, or Pi skills.
- Do not add a visible, hidden, or HTML replacement for the removed execution-profile marker; composition must be validated from canonical profile content and bindings instead.
- Do not broaden the Shared Context Plan capability ceiling to process execution.
- Do not change the Rust CLI, Bash policy runtime, Agent Trace integrations, or automated workflow semantics beyond shared validation/count updates required by this projection change.
- Preserve workflow-specific approval behavior, including explicit version-control commit approval, rather than treating broad Code-profile Bash access as approval for every command action.

## Task stack
- [x] T01: `Remove native Claude agent projections` (status:done)
  - Task ID: T01
  - Goal: Make Claude a composed-command-and-skill target with no native Shared Context agent carrier.
  - Boundaries (in/out of scope): Update the canonical projection inventory, Claude renderer/metadata surfaces, generation output list, parity/stale-output handling, projection/count checks, structural fixtures, generated config, root mirrors, and focused durable projection documentation. Delete only the four generation-owned Claude agent files currently under `config/.claude/agents/` and `.claude/agents/`; do not remove Claude commands, skills, hooks, or settings.
  - Done when: Neither Claude agent directory contains the two profile files; generation no longer emits them; stale copies cannot pass the generated-output gate; manual profiles project only to OpenCode; Claude workflow commands still compose their bound profile policy and retain exact effective `allowed-tools`; deterministic projection counts and topology assertions reflect the removal.
  - Verification notes (commands or checks): Run `nix develop -c pkl eval config/pkl/renderers/portable-execution-profile-check.pkl -x summary`, `nix develop -c pkl eval config/pkl/renderers/instruction-unit-validator-check.pkl -x summary`, and `nix run .#pkl-check-generated`; inspect `find config/.claude/agents .claude/agents -maxdepth 1 -type f` and the generated Claude command/skill inventory.
  - Completion date: 2026-07-24.
  - Files changed: canonical inventory; Claude content/metadata renderer; generator and shell/Nix parity ownership; portable and structural validator gates; four removed generated Claude agent files; root/focused projection context; this plan.
  - Evidence: Portable model check exited 0 with 58 projections and three valid/12 invalid fixtures; instruction-unit validation exited 0 with 58 rendered units, 99 generated-file units, and zero diagnostics; metadata coverage exited 0 with 58 projections/99 committed files; regeneration and `nix run .#pkl-check-generated` exited 0; an injected stale `config/.claude/agents/stale.md` made the generated-output gate exit 1 with the expected stale-path diagnostic and was removed; Claude inventories remained five commands and eight skills in both config and root trees with exact capability-derived `allowed-tools`; both Claude agent directories are absent; `git diff --check` and `nix flake check` exited 0 on x86_64-linux.
  - Notes: Context impact is root-edit required because the supported cross-harness projection topology changed. `context/{overview,architecture,glossary,context-map}.md` and focused projection/validator ownership docs now describe Claude as composed-command/skill only and the 58/41/99 path contract. No dependencies or runtime behavior changed.

- [ ] T02: `Remove HTML comments from command and prompt projections` (status:todo)
  - Task ID: T02
  - Goal: Render all command/prompt bodies without HTML comments while preserving canonical profile composition behavior.
  - Boundaries (in/out of scope): Remove the `sce-execution-profile` marker from the shared composition boundary and remove marker-dependent Claude/Pi validator rules and fixtures; replace marker-based validation only with canonical binding/content checks, not another marker. Regenerate all affected config and root command/prompt projections and update focused durable composition documentation. Do not flatten or duplicate profile policy text.
  - Done when: OpenCode command, Claude command, and Pi prompt trees in both `config/` and root mirrors contain no `<!--`; Claude and Pi workflows still include their bound profile policy, guardrails, failure handling, and required Pi entry-skill read; validators reject missing composed policy content without requiring identity comments; no replacement marker exists.
  - Verification notes (commands or checks): Run the portable profile and instruction-unit validator checks through `nix develop`; run `nix run .#pkl-check-generated`; use `grep -RIn --include='*.md' '<!--' config/.opencode/command config/automated/.opencode/command config/.claude/commands config/.pi/prompts .opencode/command .claude/commands .pi/prompts` and require no matches.

- [ ] T03: `Enforce explicit OpenCode profile and command permissions` (status:todo)
  - Task ID: T03
  - Goal: Give manual OpenCode Code and Plan profiles the requested opposite Bash postures while retaining explicit command and skill permissions.
  - Boundaries (in/out of scope): Adjust canonical manual capability/approval ownership and OpenCode rendering so the Code profile emits `bash: allow` and the Plan profile emits `bash: block`; ensure Plan-bound commands also block Bash and every command emits its effective tool permission block plus required-skill permissions. Preserve workflow narrowing and commit approval semantics; do not change Claude tool translation, automated behavior, or the runtime Bash policy plugin.
  - Done when: Generated config and root `Shared Context Code.md` files contain `bash: allow`; generated config and root `Shared Context Plan.md` files contain `bash: block`; `change-to-plan` cannot execute Bash; all manual OpenCode commands contain `permission:`, `entry-skill:`, ordered `skills:`, wildcard skill posture, and allow entries for exactly their required skills; process-capable Code workflows render allowed Bash while workflow-specific exclusions/approvals remain effective; focused fixtures assert these outcomes.
  - Verification notes (commands or checks): Run `nix develop -c pkl eval config/pkl/renderers/portable-execution-profile-check.pkl -x summary`, `nix develop -c pkl eval config/pkl/renderers/instruction-unit-validator-check.pkl -x summary`, and `nix run .#pkl-check-generated`; inspect both config and root OpenCode profile/command frontmatter for exact Bash and skill permission values.

- [ ] T04: `Validate generated harness cleanup and synchronize context` (status:todo)
  - Task ID: T04
  - Goal: Prove the complete projection, marker-removal, permission, and generated-output contract and leave durable context aligned with repository truth.
  - Boundaries (in/out of scope): Review all prior task changes, remove only obsolete generation-owned scaffolding or stale references introduced by this change, verify focused context and `context/context-map.md`, and run the complete repository checks. Do not add unrelated refactors or features.
  - Done when: All success criteria are evidenced; no stale Claude agent file, HTML command/prompt comment, obsolete marker fixture, old projection count, or contradictory durable-context statement remains; generated output is byte-identical to canonical Pkl generation; context-sync reports current-state documentation aligned; full repository validation passes or any external failure is recorded with exact evidence.
  - Verification notes (commands or checks): Run `nix run .#pkl-check-generated` and `nix flake check`; repeat filesystem searches for Claude agent files and command/prompt HTML comments; inspect `git diff --check`, generated path/count summaries, OpenCode profile/command permission blocks, and focused context references.

## Open questions
- None.
