from pathlib import Path
import subprocess

PR_HEAD = "58b7cd0eaa905976cee4297bd2ec27f10c06be7b"
PR_BRANCH = "codex/compress-workflow-preambles"
TARGETS = [".pi", ".claude", ".agents"]


def run(*args):
    subprocess.run(list(args), check=True)


def between(text, start, end, new, label):
    if text.count(start) != 1 or text.count(end) < 1:
        raise SystemExit(f"{label}: ambiguous section")
    left, rest = text.split(start, 1)
    old, right = rest.split(end, 1)
    return left + new + end + right


def once(text, old, new, label):
    if text.count(old) != 1:
        raise SystemExit(f"{label}: expected one occurrence, found {text.count(old)}")
    return text.replace(old, new)


run("git", "config", "user.name", "David Abram")
run("git", "config", "user.email", "davidabram@users.noreply.github.com")
run("git", "checkout", "--detach", PR_HEAD)

rels = ["skills/sce-handover/SKILL.md", "skills/sce-handover/references/handover-template.md"]
before = {t: sum(len((Path(t) / r).read_text().splitlines()) for r in rels) for t in TARGETS}

TEMPLATE = """### Completeness contract

- The first four `##` sections shown in **Layout** are required and must appear
  in that order.
- Each required section's content, up to the next `##` heading or the end of the
  file, must contain non-whitespace content. An empty list marker, unreplaced
  `{...}` placeholder, or other template scaffolding alone is invalid.
- Explicit `None identified.` statements or section-appropriate equivalents are
  real content and are valid.
- Writer mode must satisfy this contract before reporting success; loader mode
  validates the same contract before presenting a handover.

### Rules

- Include `Plan` and `Task` only when the session was working one identifiable
  plan task; omit them rather than guessing.
- Keep `Assumptions` scoped to details actually labeled as inferred elsewhere
  in the document; do not duplicate confirmed facts here.
- Describe durable state useful to a future session, not a transcript of this
  one.
"""
WRITER = """#### 3. Compose the handover document

Read `references/handover-template.md` before composing. It is the sole
authority for the persisted schema and **Completeness contract**. Populate its
layout from the gathered facts and satisfy that contract without redefining the
required-section set here.

Label inferred or assumed details inline as assumptions; do not blend them with
confirmed facts.

"""
WRITE = """#### 5. Write exactly one file

Write the composed document to the path resolved in step 2. Before reporting
success, validate the written file against the template's **Completeness
contract**.

"""
LOADER = """#### 2. Validate handover completeness

Read the file and `references/handover-template.md`. Validate the file against
the template's **Completeness contract**; do not define a second required-section
list or a different content-validity rule here.

When that contract fails, render the **Loader blocked** layout (invalid handover)
and stop.

"""

for target in TARGETS:
    skill = Path(target) / "skills/sce-handover/SKILL.md"
    s = skill.read_text()
    s = between(s, "#### 3. Compose the handover document\n", "#### 4. Confirm the context root\n", WRITER, f"{target} writer")
    s = between(s, "#### 5. Write exactly one file\n", "#### 6. Report\n", WRITE, f"{target} write validation")
    s = between(s, "#### 2. Validate handover completeness\n", "#### 3. Present for continuation\n", LOADER, f"{target} loader")
    skill.write_text(s)
    template = Path(target) / "skills/sce-handover/references/handover-template.md"
    t = template.read_text()
    t = t[:t.index("### Rules\n")] + TEMPLATE
    template.write_text(t)

src = Path("config/pkl/base/workflow-handover.pkl")
s = src.read_text()
indent = lambda x: "\n".join(("    " + line if line else "") for line in x.splitlines()) + "\n"
s = between(s, "    ### Rules\n", "    \"\"\"\n\nlocal renderSkillBody", indent(TEMPLATE), "canonical template")
s = between(s, "    #### 3. Compose the handover document\n", "    #### 4. Confirm the context root\n", indent(WRITER), "canonical writer")
s = between(s, "    #### 5. Write exactly one file\n", "    #### 6. Report\n", indent(WRITE), "canonical write validation")
s = between(s, "    #### 2. Validate handover completeness\n", "    #### 3. Present for continuation\n", indent(LOADER), "canonical loader")
src.write_text(s)

contract = Path("config/pkl/renderers/generation-contract-check.pkl")
c = contract.read_text()
start = "local requiredHandoverSkillTokens = new Listing {\n"
end = "local compactWorkflowSlugs = new Listing {\n"
new_defs = """local requiredHandoverSkillTokens = new Listing {
  "selects **writer mode**"
  "selects **loader mode**"
  "### Writer path (no arguments)"
  "### Loader path (one path argument)"
  "references/handover-template.md"
  "**Completeness contract**"
  "Writer mode never overwrites an existing handover file"
  "Loading is read-only"
  "Never invoke another SCE skill, sibling SCE package, or SCE workflow command"
}

local requiredHandoverTemplateCompletenessTokens = new Listing {
  "### Completeness contract"
  "first four `##` sections shown in **Layout** are required"
  "next `##` heading or the end of the"
  "empty list marker"
  "`{...}` placeholder"
  "Explicit `None identified.` statements"
  "Writer mode must satisfy this contract before reporting success"
  "validates the same contract before presenting a handover"
}

local requiredHandoverSkillCompletenessConsumerTokens = new Listing {
  "sole\nauthority for the persisted schema and **Completeness contract**"
  "validate the written file against the template's **Completeness\ncontract**"
  "Validate the file against\nthe template's **Completeness contract**"
}

local forbiddenHandoverSkillCompletenessTokens = new Listing {
  "`Current Task State`"
  "`Decisions Made`"
  "`Open Questions / Blockers`"
  "`Next Recommended Step`"
  "all four required sections"
  "placeholder such as `{What is being worked on...}`"
  "missing, empty, or placeholder-only"
}

"""
c = between(c, start, end, new_defs, "handover contract defs")
assertion = """hidden assertHandoverSchemaCompletenessOwnership = (documents: Mapping) ->
  if (
    new Listing { ".opencode"; ".claude"; ".pi"; ".agents" }.every((target) ->
      let (skillPath = "config/\(target)/skills/sce-handover/SKILL.md")
        let (templatePath = "config/\(target)/skills/sce-handover/references/handover-template.md")
          documents.containsKey(skillPath)
          && documents.containsKey(templatePath)
          && requiredHandoverSkillCompletenessConsumerTokens.every((token) -> documents[skillPath].contains(token))
          && forbiddenHandoverSkillCompletenessTokens.every((token) -> !documents[skillPath].contains(token))
          && requiredHandoverTemplateCompletenessTokens.every((token) -> documents[templatePath].contains(token))
    )
  ) "sce-handover: template solely owns persisted schema and completeness validity"
  else throw("sce-handover handover-template.md must own required-section/completeness policy; SKILL.md may invoke that contract but must not restate it")

"""
c = once(c, "local assertBrownfieldContent = (documents: Mapping) ->\n", assertion + "local assertBrownfieldContent = (documents: Mapping) ->\n", "insert handover ownership assertion")
c = once(c, "  [\"handover-package-content\"] = assertHandoverContent.apply(handoverSkillDocuments)\n", "  [\"handover-package-content\"] = assertHandoverContent.apply(handoverSkillDocuments)\n  [\"handover-schema-completeness-ownership\"] = assertHandoverSchemaCompletenessOwnership.apply(workflowDocuments)\n", "register handover ownership assertion")
contract.write_text(c)

ownership = Path("context/sce/dedup-ownership-table.md")
o = ownership.read_text()
anchor = "| Compact workflow execution contract | `executionContract` in `workflow-content.pkl` | Inline in change-to-plan, next-task, commit, validate, and handover entrypoints on all four targets; no extra runtime read | shared rendering, preserved behavior |\n"
row = "| Handover persisted schema and completeness validity | `references/handover-template.md` generated from `renderPersistedFormatBody` in `workflow-handover.pkl` | Writer composes and validates against this contract; loader validates against the same contract; `sce-handover/SKILL.md` owns routing, path checks, read/write boundaries, and terminal layout selection without restating required-section or content-validity rules | intentional/keep |\n"
o = once(o, anchor, row + anchor, "handover ownership row")
ownership.write_text(o)

ctx = Path("context/sce/handover-workflow.md")
x = ctx.read_text()
x = once(x, "The package contains `SKILL.md`, which owns mode routing, writer and loader\nbehavior, `references/handover-template.md`, which owns the persisted document\nformat, and `references/output.md`, which owns every human-visible layout. No\n", "The package contains `SKILL.md`, which owns mode routing plus writer/loader\nside-effect boundaries, `references/handover-template.md`, which solely owns\nthe persisted document schema and completeness contract, and\n`references/output.md`, which owns every human-visible layout. No\n", "handover context ownership")
ctx.write_text(x)

after = {t: sum(len((Path(t) / r).read_text().splitlines()) for r in rels) for t in TARGETS}
reductions = {t: before[t] - after[t] for t in TARGETS}
if len(set(reductions.values())) != 1 or next(iter(reductions.values())) <= 0:
    raise SystemExit(f"unexpected reductions: {reductions}")
reduction = next(iter(reductions.values()))

plan = Path("context/plans/compress-workflow-execution-preamble.md")
p = plan.read_text()
p += f"""

## Follow-up: handover schema and completeness ownership

- [x] T09: `Deduplicate handover required-section and completeness contract` (status:done)
  - Scope: `sce-handover` writer composition/post-write validation, loader completeness
    validation, `references/handover-template.md`, canonical Pkl, generated semantic
    checks, tracked Pi/Claude/Codex mirrors, handover context, and the ownership table.
  - Ownership after change: `references/handover-template.md` solely owns persisted
    schema and completeness validity. Writer and loader both consume that contract;
    `SKILL.md` owns routing, path checks, read/write boundaries, and terminal layouts.
  - Behavior preserved: same writer layout/order; `None identified.`-style content is
    valid; whitespace-only, empty-list-marker, and unreplaced-placeholder-only required
    sections remain invalid; loader stays read-only; writer validates before success.
  - Result: `sce-handover/SKILL.md` plus `references/handover-template.md` shrink by
    {reduction} Markdown lines per tracked target, {reduction * len(TARGETS)} across
    Pi/Claude/Codex.
  - Verify: `pkl eval config/pkl/renderers/generation-contract-check.pkl`,
    `nix run .#pkl-check-generated`, targeted ownership checks, and `git diff --check`.
  - Context synchronization: synced.
"""
plan.write_text(p)

required_t = ["### Completeness contract", "first four `##` sections shown in **Layout** are required", "empty list marker", "`{...}` placeholder", "Explicit `None identified.` statements"]
required_s = ["sole\nauthority for the persisted schema and **Completeness contract**", "validate the written file against the template's **Completeness\ncontract**", "Validate the file against\nthe template's **Completeness contract**"]
forbidden_s = ["`Current Task State`", "`Decisions Made`", "`Open Questions / Blockers`", "`Next Recommended Step`", "all four required sections", "missing, empty, or placeholder-only"]
for target in TARGETS:
    skill = (Path(target) / "skills/sce-handover/SKILL.md").read_text()
    template = (Path(target) / "skills/sce-handover/references/handover-template.md").read_text()
    if not all(t in template for t in required_t) or not all(t in skill for t in required_s) or any(t in skill for t in forbidden_s):
        raise SystemExit(f"{target}: handover ownership assertion failed")

run("pkl", "eval", "config/pkl/renderers/generation-contract-check.pkl")
run("nix", "run", ".#pkl-check-generated")
run("git", "diff", "--check")

planned = {"config/pkl/base/workflow-handover.pkl", "config/pkl/renderers/generation-contract-check.pkl", "context/plans/compress-workflow-execution-preamble.md", "context/sce/dedup-ownership-table.md", "context/sce/handover-workflow.md"}
for t in TARGETS:
    planned |= {f"{t}/skills/sce-handover/SKILL.md", f"{t}/skills/sce-handover/references/handover-template.md"}
changed = set(subprocess.check_output(["git", "diff", "--name-only"], text=True).splitlines())
if changed != planned:
    raise SystemExit(f"unexpected changed paths: extra={sorted(changed-planned)} missing={sorted(planned-changed)}")

print(f"handover SKILL + template Markdown reduction per tracked target: {reduction}")
print(f"total tracked Pi/Claude/Codex reduction: {reduction * len(TARGETS)}")
run("git", "add", *sorted(planned))
run("git", "commit", "-m", "config: Deduplicate handover completeness contract")
run("git", "push", "origin", f"HEAD:{PR_BRANCH}")
print(subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip())
