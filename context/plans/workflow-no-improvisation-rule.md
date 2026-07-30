# Plan: workflow-no-improvisation-rule

## Change summary

Every generated SCE workflow skill (`sce-change-to-plan`, `sce-next-task`,
`sce-validate`, `sce-commit`) must state explicitly that the executing agent may
not improvise: it follows the canonical workflow's steps, gates, and stops as
written, and it emits only the user-visible responses defined in
`references/output.md`. Today the shared composite skill preamble says to use
`references/output.md` "for every gate and terminal response" and to render no
raw internal state, but it never forbids inventing, skipping, reordering, or
merging process steps, and it never forbids adding commentary, preambles,
summaries, or extra sections around the canonical layouts.

The rule is added once, in the shared `renderSkill` preamble of
`config/pkl/renderers/workflow-composite.pkl`, which all four workflows and all
three targets (OpenCode, Claude, Pi) already compose through. This preserves the
existing single-owner rendering model rather than repeating the same prose in
four canonical workflow modules. The change is prose-only inside generated
`SKILL.md` documents; no artifact paths, counts, frontmatter, or contracts move.

## Acceptance criteria

- [ ] AC1: Every generated workflow `SKILL.md` for every target states that the
      canonical workflow's steps and stops must be followed as written and that
      no step may be invented, skipped, reordered, or merged.
  - Validate: generate into a temp root with
    `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"`, then confirm all
    12 workflow `SKILL.md` files under `.opencode/`, `.claude/`, and `.pi/`
    contain the new no-improvisation process sentence.
- [ ] AC2: Every generated workflow `SKILL.md` for every target states that
      user-visible output is limited to the layouts in `references/output.md`,
      with no invented layouts, added preambles, commentary, or extra sections.
  - Validate: same generated payload; confirm all 12 workflow `SKILL.md` files
    contain the new no-improvisation output sentence.
- [ ] AC3: The generation contract is unchanged — the same artifact paths,
      counts, metadata, and forbidden-path assertions still hold.
  - Validate: `nix run .#pkl-check-generated`

### Full validation

- `nix run .#pkl-check-generated`
- `nix flake check`

### Context sync

- `context/sce/shared-context-plan-workflow.md`
- `context/sce/shared-context-code-workflow.md`
- `context/sce/atomic-commit-workflow.md`
- `context/overview.md` (composite renderer preamble description)

## Constraints and non-goals

- **In scope:** the `renderSkill` preamble in
  `config/pkl/renderers/workflow-composite.pkl`.
- **Out of scope:** the four canonical workflow modules in
  `config/pkl/base/workflow-*.pkl`, the per-workflow `references/output.md`
  bodies, `renderCommand`, target metadata renderers, OpenCode routing agents,
  and any Rust CLI surface.
- **Constraints:** generated targets are ephemeral; edit canonical Pkl only and
  verify through ephemeral generation. The composite renderer stays
  target-neutral — the rule must apply identically to OpenCode, Claude, and Pi.
  Existing preamble sentences and section headings must not be rewritten beyond
  what the new rule requires.
- **Non-goal:** introducing a new configurable "strictness" knob, a per-workflow
  override, or a machine-checkable lint for improvised output.

## Assumptions

- "Every workflow" is satisfied by editing the one shared composite preamble
  rather than by adding four copies of the same prose, per the single-owner
  rendering model recorded in
  `context/decisions/2026-07-29-cross-target-workflow-skill-packages.md`.
- The rule belongs in `SKILL.md` (which the agent reads as instruction) rather
  than in `references/output.md` (which each workflow already scopes with "Use
  only the applicable layout").

## Task stack

- [x] T01: `Forbid workflow and output improvisation in the composite skill preamble` (status:done)
  - Task ID: T01
  - Goal: Add an explicit no-improvisation rule to the shared `renderSkill`
    preamble so all four workflow `SKILL.md` documents, on all three targets,
    instruct the agent to execute the canonical process exactly as written and
    to emit only `references/output.md` layouts as user-visible output.
  - Boundaries (in/out of scope): In — the `Purpose` and `User-visible output`
    preamble text inside `renderSkill` in
    `config/pkl/renderers/workflow-composite.pkl`. Out — canonical
    `config/pkl/base/workflow-*.pkl` modules, the four `*_OUTPUT` layout
    constants, `renderCommand`, and every target metadata renderer.
  - Dependencies: none
  - Done when: the preamble forbids inventing, skipping, reordering, or merging
    workflow steps and stops, and forbids user-visible output beyond the
    `references/output.md` layouts (no added preambles, commentary, summaries,
    or extra sections); ephemeral generation shows the new text in all 12
    workflow `SKILL.md` files; `nix run .#pkl-check-generated` and
    `nix flake check` pass.
  - Verification notes (commands or checks):
    `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"` then grep the
    generated `sce-{change-to-plan,next-task,validate,commit}/SKILL.md` under
    `.opencode/skills`, `.claude/skills`, and `.pi/skills` for the new
    sentences; `nix run .#pkl-check-generated`; `nix flake check`.
  - Evidence: `config/pkl/renderers/workflow-composite.pkl` — the `renderSkill`
    preamble now ends `Purpose` with "Follow the canonical workflow's steps,
    gates, and stops exactly as written: never invent, skip, reorder, or merge a
    step." and ends `User-visible output` with "User-visible output is limited to
    those layouts: never invent a layout, and never wrap one in an added
    preamble, commentary, summary, or extra section." No other file changed;
    existing preamble sentences and headings were left intact.
  - Verification: `nix run .#pkl-generate -- "$(mktemp -d -t sce-gen-XXXX)"` ->
    passed; both sentences present in all 12 generated workflow `SKILL.md` files
    under `.claude/skills`, `.opencode/skills`, and `.pi/skills` (matched with
    newline-insensitive grep, since the rendered prose wraps across lines).
    `nix run .#pkl-check-generated` -> passed (46 files, inventory sha256
    314a5846c990ac8068b74afd569b62e653119a6f33c8b33516ecef5dd6008b9f).
    `nix flake check` -> all checks passed.
  - Assumptions: both halves of the rule were implemented, per the plan's open
    question offering to drop the output sentence only on request.

## Open questions

- The output half of this rule is already partially covered: the preamble says
  to use `references/output.md` for every gate and terminal response and to
  render no raw internal state, and each `references/output.md` says "Use only
  the applicable layout." The genuinely new coverage is the process half (no
  invented, skipped, reordered, or merged steps) plus the explicit ban on
  wrapping the canonical layouts in extra prose. If you only wanted the process
  half, say so and the output sentence will be dropped rather than restated.
- Prose instructions of this kind are unenforceable by generation checks — the
  contract checks assert paths and metadata, not agent behavior. This change
  raises the odds an agent complies; it cannot guarantee it. If you want a
  guarantee, that is a different plan (structured output validation), and I do
  not think it is worth building yet.
