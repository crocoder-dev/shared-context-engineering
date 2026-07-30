# Plan: project-overview-html-skill

## Change summary

Add a new SCE skill `sce-project-overview-html` that generates a single self-contained HTML document from the project's `context/` Markdown files, helping a human reader quickly understand the project and how it works. The HTML embeds Mermaid.js (client-side, CDN) so existing Mermaid diagrams in context render in-browser, and includes CSS styling for a clean, readable layout. Output is written to `context/tmp/project-overview.html` (disposable, gitignored).

This plan uses a two-phase staging approach:
- **Phase 1 (T01):** Hand-author the skill directly in the repo-root `.opencode/skills/` active runtime tree so the user can manually test it with OpenCode immediately.
- **Phase 2 (T02–T07):** Once manually validated, promote the skill into the canonical Pkl source layer, register it across all metadata/renderers, regenerate the generated config trees, update context discoverability, and validate.

## Success criteria

- A new skill `sce-project-overview-html` exists in the repo-root `.opencode/skills/sce-project-overview-html/SKILL.md` and is manually testable via OpenCode (Phase 1).
- The skill is promoted to canonical Pkl sources and generated into all three target trees (manual OpenCode, automated OpenCode, Claude) with parity verified (Phase 2).
- The skill body instructs the agent to read `context/overview.md`, `context/architecture.md`, `context/patterns.md`, `context/glossary.md`, and `context/context-map.md`, plus any linked domain files referenced in `context-map.md`, and render them into a single self-contained HTML document.
- The generated HTML includes:
  - Embedded Mermaid.js (CDN `<script>`) so existing Mermaid blocks in context render in-browser.
  - Inline CSS for readable typography, section navigation, code block styling, and responsive layout.
  - Sectioned content (Overview, Architecture, Patterns, Glossary, Context Map) with anchor navigation.
  - Mermaid diagram blocks preserved as `<pre class="mermaid">` so the client-side library renders them.
- Output is written to `context/tmp/project-overview.html` (disposable, gitignored by existing `context/tmp/.gitignore`).
- `nix run .#pkl-check-generated` passes (generated outputs match canonical Pkl sources).
- `nix flake check` passes.
- The skill is invocable by the Shared Context Code agent (registered in its `skill:` permission allowlist in both manual and automated OpenCode metadata).
- `context/context-map.md` is updated with a discoverability link to the new skill's canonical description (in `context/sce/`).

## Constraints and non-goals

- **Source of truth is `context/` only.** The skill does not scan or analyze application code. It renders the current-state context memory into HTML.
- **No new runtime dependencies.** Mermaid.js is loaded from CDN at view time; no vendoring, no build step, no Node tooling added to the repo.
- **No new CLI command.** This is a skill, not a `sce` subcommand. It is invoked via the Shared Context Code agent loading the skill.
- **Output is disposable.** `context/tmp/project-overview.html` is regenerated on demand and is not committed (covered by existing `context/tmp/.gitignore` with `*` ignore).
- **No offline support in this plan.** Mermaid.js CDN requires internet to view diagrams. Vendoring/offline is a non-goal for this iteration.
- **No pre-rendering to SVG.** No `mermaid-cli` or headless browser build dependency is introduced.
- **No code scanning or enrichment beyond context/.** If context is stale, the HTML reflects stale context; the skill should note this in its instructions but not attempt to repair context (that is `sce-context-sync`'s job).
- **No changes to the `sce` Rust CLI.** This plan touches only the repo-root `.opencode/skills/` tree (Phase 1), Pkl sources, generated config outputs, and context files (Phase 2).
- **Phase 1 does not touch `config/.opencode/`.** The repo-root `.opencode/skills/` is the active runtime tree; `config/.opencode/skills/` is Pkl-generated and is only updated in Phase 2 via regeneration. Phase 1 and Phase 2 must not both edit the same generated path by hand.

## Task stack

### Phase 1 — Stage skill in active runtime tree for manual testing

- [x] T01: `Create sce-project-overview-html skill in repo-root .opencode/skills/` (status:done)
  - Task ID: T01
  - Goal: Hand-author `.opencode/skills/sce-project-overview-html/SKILL.md` with OpenCode skill frontmatter (`name`, `description`, `compatibility: opencode`) and the full skill body (when to use, source files to read, HTML structure contract, Mermaid.js CDN embedding, inline CSS guidance, output path `context/tmp/project-overview.html`, disposable-output note, stale-context caveat). This makes the skill immediately loadable and testable by OpenCode in this repo.
  - Boundaries (in/out of scope): In - new `.opencode/skills/sce-project-overview-html/SKILL.md` file only. Out - `config/.opencode/` generated tree (Phase 2), Pkl sources (Phase 2), metadata files (Phase 2), context files (Phase 2). Do not edit any other `.opencode/` files.
  - Done when: `.opencode/skills/sce-project-overview-html/SKILL.md` exists with valid frontmatter and the complete skill body; OpenCode can discover the skill (visible in skill list / invocable).
  - Verification notes (commands or checks): `ls .opencode/skills/sce-project-overview-html/SKILL.md`; `head -6 .opencode/skills/sce-project-overview-html/SKILL.md` shows correct frontmatter; user manually invokes the skill in OpenCode and confirms it produces `context/tmp/project-overview.html`.
  - **Manual test gate:** Do not start T02 until the user confirms the skill works as expected in OpenCode.
  - **Status:** done
  - **Completed:** 2026-07-01 (revised 2026-07-02)
  - **Files changed:** `.opencode/skills/sce-project-overview-html/SKILL.md` (new, then revised)
  - **Evidence:** `ls` confirms file exists; `head -6` shows valid frontmatter (`name`, `description`, `compatibility: opencode`); `nix run .#pkl-check-generated` prints `Generated outputs are up to date.` (repo-root `.opencode/skills/` is not part of generated parity); `git status` shows only the new untracked skill directory.
  - **Notes:** Skill body follows the same frontmatter shape and section structure (`What I do` / `When to use` / `How to run this` / `Expected output` / `Related skills`) as existing repo-root skills. Revised after user feedback: skill now explicitly instructs the agent to author the HTML directly with its own file tools (no Python/conversion script generation) and uses a left-side navigation sidebar layout. Awaiting user manual test confirmation before T02.

### Phase 2 — Promote skill to canonical Pkl pipeline

- [x] T02: `Author sce-project-overview-html skill body in shared-content-code.pkl` (status:done)
  - Task ID: T02
  - Goal: Add the `sce-project-overview-html` skill `UnitSpec` (title + canonicalBody) to the `skills` Mapping in `config/pkl/base/shared-content-code.pkl`, and the mirrored entry in `config/pkl/base/shared-content-automated-code.pkl`. Use the skill body validated in T01 as the canonical source.
  - Boundaries (in/out of scope): In - new `UnitSpec` entry in both manual and automated code Pkl modules. Out - aggregation surface changes (T03), metadata registration (T04), generated output regeneration (T05), context-map update (T06).
  - Done when: Both `shared-content-code.pkl` and `shared-content-automated-code.pkl` contain a `["sce-project-overview-html"]` entry in their `skills` Mapping with identical `title` and `canonicalBody` (skill body is shared between profiles; only agent permission/metadata differs).
  - Verification notes (commands or checks): `grep -n 'sce-project-overview-html' config/pkl/base/shared-content-code.pkl config/pkl/base/shared-content-automated-code.pkl` returns the new entry in both files; diff of the two `canonicalBody` strings shows identical content.
  - **Status:** done
  - **Completed:** 2026-07-04
  - **Files changed:** `config/pkl/base/shared-content-code.pkl`, `config/pkl/base/shared-content-automated-code.pkl`
  - **Evidence:** `grep -n 'sce-project-overview-html'` returns line 295 in `shared-content-code.pkl` and line 282 in `shared-content-automated-code.pkl`; `diff` of the extracted `["sce-project-overview-html"]` entries from both files shows identical content; `nix run .#pkl-check-generated` prints `Generated outputs are up to date.` (Pkl syntax valid; generated trees unchanged because aggregation surfaces are T03).
  - **Notes:** New entry placed last in each `skills` Mapping (after `["sce-validation"]`) to preserve existing ordering and minimize diff noise. `title = "SCE Project Overview HTML"` follows the `UpperCamelCase` convention of other `UnitSpec` titles. The `canonicalBody` is the T01-validated skill body (content after YAML frontmatter). No triple-double-quote sequences in the body, so no Pkl escaping needed.

- [x] T03: `Register skill in shared-content aggregation surfaces` (status:done)
  - Task ID: T03
  - Goal: Add `["sce-project-overview-html"]` entries to the `skills` Mapping in both `config/pkl/base/shared-content.pkl` (manual) and `config/pkl/base/shared-content-automated.pkl` (automated), referencing the new `UnitSpec` from `code.skills["sce-project-overview-html"]`.
  - Boundaries (in/out of scope): In - the two aggregation surface Pkl files. Out - renderer files (T04), Pkl regeneration (T05).
  - Done when: Both aggregation files contain a `["sce-project-overview-html"]` entry in `skills` that references `code.skills["sce-project-overview-html"]` with `id = "skill.sce-project-overview-html"`, `kind = "skill"`, `slug = "sce-project-overview-html"`, and the correct `title`/`canonicalBody` forwarding.
  - Verification notes (commands or checks): `grep -n 'sce-project-overview-html' config/pkl/base/shared-content.pkl config/pkl/base/shared-content-automated.pkl` returns the new entry in both files.
  - **Status:** done
  - **Completed:** 2026-07-04
  - **Files changed:** `config/pkl/base/shared-content.pkl`, `config/pkl/base/shared-content-automated.pkl`
  - **Evidence:** `grep -n 'sce-project-overview-html'` returns line 120 in `shared-content.pkl` and line 134 in `shared-content-automated.pkl`; both entries reference `code.skills["sce-project-overview-html"]` with `id = "skill.sce-project-overview-html"`, `kind = "skill"`, `slug = "sce-project-overview-html"`, and `title`/`canonicalBody` forwarding.
  - **Notes:** Executed together with T04 and T05 in the same session because T03 alone breaks `pkl-check-generated` (the renderer looks up `metadata.skillDescriptions[unitSlug]`, which T04 owns). User approved scope expansion to T03+T04+T05 so the repo stays green. New entry placed last in each `skills` Mapping (after `["sce-validation"]`) to preserve existing ordering and minimize diff noise.

- [x] T04: `Register skill descriptions and permissions in all four metadata files` (status:done)
  - Task ID: T04
  - Goal: Add the `sce-project-overview-html` description to `skillDescriptions` in all four metadata files (`config/pkl/renderers/opencode-metadata.pkl`, `config/pkl/renderers/opencode-automated-metadata.pkl`, `config/pkl/renderers/claude-metadata.pkl`, `config/pkl/renderers/common.pkl`) and add `sce-project-overview-html` to the Shared Context Code agent `skill:` allowlist in both `opencode-metadata.pkl` and `opencode-automated-metadata.pkl` permission blocks.
  - Boundaries (in/out of scope): In - the four metadata/renderer files plus the Shared Context Code permission allowlists. Out - skill body content (T02), aggregation (T03), regeneration (T05).
  - Done when: All four metadata files have a `["sce-project-overview-html"]` entry in `skillDescriptions`; both OpenCode metadata files list `"sce-project-overview-html": allow` under the `shared-context-code` agent's `skill:` block; `common.pkl` `skillDescriptions` Mapping includes the new slug.
  - Verification notes (commands or checks): `grep -n 'sce-project-overview-html' config/pkl/renderers/opencode-metadata.pkl config/pkl/renderers/opencode-automated-metadata.pkl config/pkl/renderers/claude-metadata.pkl config/pkl/renderers/common.pkl` returns entries in all four files; `grep -A2 'sce-project-overview-html' config/pkl/renderers/opencode-metadata.pkl` shows it under the `shared-context-code` permission block.
  - **Status:** done
  - **Completed:** 2026-07-04
  - **Files changed:** `config/pkl/renderers/common.pkl`, `config/pkl/renderers/opencode-metadata.pkl`, `config/pkl/renderers/opencode-automated-metadata.pkl`, `config/pkl/renderers/claude-metadata.pkl`
  - **Evidence:** `grep -n 'sce-project-overview-html'` returns entries in all four metadata files; `grep -B1 -A2` confirms `"sce-project-overview-html": allow` sits under the `shared-context-code` `skill:` block in both OpenCode metadata files (manual line 72, automated line 75). The full skill description (matching the T01 SKILL.md frontmatter `description`) is added to `opencode-metadata.pkl`, `opencode-automated-metadata.pkl`, and `claude-metadata.pkl`; a shorter shared description is added to `common.pkl`.
  - **Notes:** Executed together with T03 and T05. The `common.pkl` description is a shorter shared form ("Use when the user wants a project overview as HTML...") consistent with the shorter shared descriptions already present for other skills in `common.pkl`; the three target-specific metadata files carry the full description matching the T01 SKILL.md frontmatter.

- [x] T05: `Regenerate generated config outputs and verify parity` (status:done)
  - Task ID: T05
  - Goal: Run `nix develop -c pkl eval -m . config/pkl/generate.pkl` to regenerate `config/.opencode/skills/sce-project-overview-html/SKILL.md`, `config/automated/.opencode/skills/sce-project-overview-html/SKILL.md`, and `config/.claude/skills/sce-project-overview-html/SKILL.md`, then verify parity with `nix run .#pkl-check-generated`. Also sync the repo-root `.opencode/skills/sce-project-overview-html/SKILL.md` with the regenerated `config/.opencode/` version so the active runtime tree matches the canonical generated output.
  - Boundaries (in/out of scope): In - running Pkl regeneration, the parity check, and syncing the repo-root `.opencode/skills/` copy with the regenerated `config/.opencode/skills/` output. Out - editing generated files by hand (generated trees are build artifacts), editing Pkl sources (covered by T02-T04).
  - Done when: `nix run .#pkl-check-generated` prints `Generated outputs are up to date.` and exits 0; the three generated `SKILL.md` files exist with correct frontmatter (`name`, `description`, `compatibility`) and the authored body; the repo-root `.opencode/skills/sce-project-overview-html/SKILL.md` matches `config/.opencode/skills/sce-project-overview-html/SKILL.md`.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `ls config/.opencode/skills/sce-project-overview-html/SKILL.md config/automated/.opencode/skills/sce-project-overview-html/SKILL.md config/.claude/skills/sce-project-overview-html/SKILL.md`; `diff .opencode/skills/sce-project-overview-html/SKILL.md config/.opencode/skills/sce-project-overview-html/SKILL.md` shows no differences.
  - **Status:** done
  - **Completed:** 2026-07-04
  - **Files changed:** `config/.opencode/skills/sce-project-overview-html/SKILL.md` (new), `config/automated/.opencode/skills/sce-project-overview-html/SKILL.md` (new), `config/.claude/skills/sce-project-overview-html/SKILL.md` (new), `config/.opencode/agent/Shared Context Code.md` (permission allowlist), `config/automated/.opencode/agent/Shared Context Code.md` (permission allowlist), `.opencode/skills/sce-project-overview-html/SKILL.md` (synced with regenerated canonical output)
  - **Evidence:** `nix run .#pkl-check-generated` prints `Generated outputs are up to date.` and exits 0; `ls` confirms all three generated `SKILL.md` files exist; `head -6` confirms correct frontmatter (`name`, `description`, `compatibility: opencode` for OpenCode variants, `compatibility: claude` for Claude variant); `diff .opencode/skills/sce-project-overview-html/SKILL.md config/.opencode/skills/sce-project-overview-html/SKILL.md` shows no differences after sync (trailing newline added to repo-root copy to match generated canonical output).
  - **Notes:** Executed together with T03 and T04. Regeneration also updated the two Shared Context Code agent files (`config/.opencode/agent/Shared Context Code.md` and `config/automated/.opencode/agent/Shared Context Code.md`) with the new `"sce-project-overview-html": allow` permission entry, as expected from the T04 metadata edits.

- [x] T06: `Update context-map.md with new skill discoverability link` (status:done)
  - Task ID: T06
  - Goal: Add a `context/sce/project-overview-html-skill.md` domain file describing the new skill's current-state contract, and add a discoverability link to it in `context/context-map.md` under the SCE working-area or feature/domain section.
  - Boundaries (in/out of scope): In - new `context/sce/project-overview-html-skill.md` file and one new bullet in `context/context-map.md`. Out - editing other context files, editing `context/overview.md` (this is a localized skill addition, verify-only for root context per `sce-context-sync` gating).
  - Done when: `context/sce/project-overview-html-skill.md` exists with a concise current-state description of the skill (purpose, source files, output path, Mermaid.js CDN dependency, disposable-output policy); `context/context-map.md` contains a bullet linking to it.
  - Verification notes (commands or checks): `ls context/sce/project-overview-html-skill.md`; `grep -n 'project-overview-html' context/context-map.md`.
  - **Status:** done
  - **Completed:** 2026-07-05
  - **Files changed:** `context/sce/project-overview-html-skill.md` (new), `context/context-map.md` (one new bullet in Feature/domain context)
  - **Evidence:** `ls context/sce/project-overview-html-skill.md` confirms the file exists (4464 bytes); `grep -n 'project-overview-html' context/context-map.md` returns line 25 with the new discoverability bullet; `nix run .#pkl-check-generated` prints `Generated outputs are up to date.` (no Pkl/generated changes, parity unaffected).
  - **Notes:** Domain file follows the current-state contract style of existing `context/sce/*.md` entries (purpose, source of truth, files read, rendering contract, HTML structure, Mermaid.js CDN dependency, output path/disposable policy, canonical Pkl authoring/generation surfaces, related skills). The context-map bullet is placed in the Feature/domain context list immediately after `dedup-ownership-table.md`, grouping it with the other SCE workflow/skill entries. No root context files (`overview.md`, `architecture.md`, `patterns.md`, `glossary.md`) were edited — this is a localized skill addition, so root context is verify-only per `sce-context-sync` gating.

- [x] T07: `Validation and cleanup` (status:done)
  - Task ID: T07
  - Goal: Run full repo validation (`nix flake check`), confirm all success criteria are met, and verify no stray temporary artifacts were left behind.
  - Boundaries (in/out of scope): In - `nix flake check`, success-criteria walkthrough, cleanup of any `context/tmp/` scratch files produced during planning/testing. Out - new code or context edits beyond cleanup.
  - Done when: `nix flake check` passes; each success criterion above has concrete evidence (command output or file path); no unexpected files exist under `context/tmp/` other than `.gitignore`.
  - Verification notes (commands or checks): `nix flake check`; `git status` shows only intended changes (repo-root `.opencode/skills/` new file, Pkl sources, generated config, new context file, updated context-map).
  - **Status:** done
  - **Completed:** 2026-07-06
  - **Files changed:** `context/plans/project-overview-html-skill.md` (T07 status update); `context/tmp/project-overview.html` and `context/tmp/cli_sce_md_contents.json` deleted (plan scratch cleanup).
  - **Evidence:**
    - `nix run .#pkl-check-generated` prints `Generated outputs are up to date.` and exits 0.
    - `nix flake check` exits 0 with all 14 `aarch64-darwin` checks ✅ (`cli-tests`, `cli-clippy`, `cli-fmt`, `integrations-install-tests`, `integrations-install-clippy`, `integrations-install-fmt`, `workflow-actionlint`, `pkl-parity`, `npm-bun-tests`, `npm-biome-check`, `npm-biome-format`, `config-lib-bun-tests`, `config-lib-biome-check`, `config-lib-biome-format`).
    - Success-criteria walkthrough (all met):
      - SC1 (skill in repo-root `.opencode/skills/`): `.opencode/skills/sce-project-overview-html/SKILL.md` exists (8309 bytes) with valid frontmatter (`name`, `description`, `compatibility: opencode`).
      - SC2 (promoted to Pkl + generated into all three trees): `sce-project-overview-html` present in `shared-content-code.pkl`, `shared-content-automated-code.pkl`, `shared-content.pkl`, `shared-content-automated.pkl`; generated `SKILL.md` exists in `config/.opencode/`, `config/automated/.opencode/`, and `config/.claude/`; `nix run .#pkl-check-generated` passes (parity verified).
      - SC3 (skill body reads context files + renders HTML): verified in T01/T02 skill body — reads `context/overview.md`, `architecture.md`, `patterns.md`, `glossary.md`, `context-map.md` plus linked domain files; renders single self-contained HTML.
      - SC4 (HTML includes Mermaid.js CDN, inline CSS, sectioned content with anchor nav, `<pre class="mermaid">` blocks): verified in T01/T02 skill body contract.
      - SC5 (output to `context/tmp/project-overview.html`, gitignored): `context/tmp/.gitignore` is `*` + `!.gitignore`, covering the output path.
      - SC6 (`pkl-check-generated` + `nix flake check` pass): both pass (see above).
      - SC7 (invocable by Shared Context Code agent): `"sce-project-overview-html": allow` present in both `config/.opencode/agent/Shared Context Code.md` and `config/automated/.opencode/agent/Shared Context Code.md` (line 31 each); `skillDescriptions` entries present in all four renderer metadata files.
      - SC8 (context-map discoverability link): `context/sce/project-overview-html-skill.md` exists (4464 bytes); `context/context-map.md` line 25 has the discoverability bullet.
    - Repo-root `.opencode/skills/sce-project-overview-html/SKILL.md` is identical to generated `config/.opencode/skills/sce-project-overview-html/SKILL.md` (`diff` shows no differences).
    - Cleanup: deleted plan-produced scratch files `context/tmp/project-overview.html` and `context/tmp/cli_sce_md_contents.json`. Remaining `context/tmp/` contents are gitignored runtime byproducts (`*-diff-trace.json`, `*-post-commit.json`, `sce.log`) from normal `sce hooks` operation, not plan scratch files.
    - `git status --porcelain` is empty (working tree clean; all plan changes were committed in prior tasks).
  - **Notes:** This is the final plan task. All 7 tasks (T01–T07) are complete. Plan is ready for closure.

## Open questions

None. All blocking ambiguities were resolved during the clarification gate:
- Source of truth: `context/` files only.
- Output location: `context/tmp/project-overview.html` (disposable, gitignored).
- Diagram rendering: Mermaid.js client-side via CDN.
- Skill ownership: Shared Context Code agent.
- Staging: Phase 1 in repo-root `.opencode/skills/` for manual testing, Phase 2 promotes to Pkl canonical pipeline.

## Validation Report

### Commands run
- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (all 14 `aarch64-darwin` checks ✅: `cli-tests`, `cli-clippy`, `cli-fmt`, `integrations-install-tests`, `integrations-install-clippy`, `integrations-install-fmt`, `workflow-actionlint`, `pkl-parity`, `npm-bun-tests`, `npm-biome-check`, `npm-biome-format`, `config-lib-bun-tests`, `config-lib-biome-check`, `config-lib-biome-format`)
- `git status --porcelain` -> empty (working tree clean)
- Removed: `context/tmp/project-overview.html` and `context/tmp/cli_sce_md_contents.json` (plan-produced scratch files from T01 manual testing)

### Success-criteria verification
- [x] SC1: Skill exists in repo-root `.opencode/skills/sce-project-overview-html/SKILL.md` and is manually testable -> confirmed via `ls` (8309 bytes) + `head -6` shows valid frontmatter (`name`, `description`, `compatibility: opencode`)
- [x] SC2: Skill promoted to canonical Pkl sources and generated into all three target trees with parity verified -> `sce-project-overview-html` present in `shared-content-code.pkl`, `shared-content-automated-code.pkl`, `shared-content.pkl`, `shared-content-automated.pkl`; generated `SKILL.md` exists in `config/.opencode/`, `config/automated/.opencode/`, `config/.claude/`; `nix run .#pkl-check-generated` passes
- [x] SC3: Skill body instructs reading `context/overview.md`, `architecture.md`, `patterns.md`, `glossary.md`, `context-map.md` plus linked domain files and rendering into single HTML -> verified in T01/T02 skill body
- [x] SC4: Generated HTML includes Mermaid.js CDN `<script>`, inline CSS, sectioned content with anchor navigation, `<pre class="mermaid">` blocks -> verified in T01/T02 skill body contract
- [x] SC5: Output written to `context/tmp/project-overview.html` (disposable, gitignored) -> `context/tmp/.gitignore` is `*` + `!.gitignore`, covering the path
- [x] SC6: `nix run .#pkl-check-generated` passes + `nix flake check` passes -> both pass (see Commands run)
- [x] SC7: Skill invocable by Shared Context Code agent (registered in `skill:` allowlist in both manual and automated OpenCode metadata) -> `"sce-project-overview-html": allow` at line 31 in both `config/.opencode/agent/Shared Context Code.md` and `config/automated/.opencode/agent/Shared Context Code.md`; `skillDescriptions` entries in all four renderer metadata files
- [x] SC8: `context/context-map.md` updated with discoverability link to `context/sce/project-overview-html-skill.md` -> line 25 bullet; domain file exists (4464 bytes)

### Context sync
- Classification: verify-only (localized skill addition; no root-level behavior/architecture/terminology impact).
- Root files (`overview.md`, `architecture.md`, `glossary.md`, `patterns.md`) verified against code truth — no edits needed. The skill is a standalone utility skill not wired into command-body orchestration, so it does not belong in the command-orchestration descriptions like `/next-task`/`/change-to-plan`/`/commit` skills.
- Feature existence documentation present and linked: `context/sce/project-overview-html-skill.md` + `context/context-map.md` line 25.
- Repo-root `.opencode/skills/sce-project-overview-html/SKILL.md` is identical to generated `config/.opencode/skills/sce-project-overview-html/SKILL.md`.

### Failed checks and follow-ups
- None.

### Residual risks
- None identified. Plan is complete; all 7 tasks (T01–T07) done.