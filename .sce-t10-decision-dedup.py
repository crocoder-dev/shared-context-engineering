from pathlib import Path
import subprocess

PR_HEAD = "a4a2bf05cc471aa4ce54e85137b43d10bb3d346b"
PR_BRANCH = "codex/compress-workflow-preambles"
TARGETS = [".pi", ".claude", ".agents"]

def run(*args):
    subprocess.run(list(args), check=True)

def once(text, old, new, label):
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one occurrence, found {count}")
    return text.replace(old, new)

def between(text, start, end, replacement, label):
    if text.count(start) != 1:
        raise SystemExit(f"{label}: start marker count {text.count(start)}")
    left, rest = text.split(start, 1)
    if end not in rest:
        raise SystemExit(f"{label}: end marker missing")
    _, right = rest.split(end, 1)
    return left + replacement + end + right

run("git", "config", "user.name", "David Abram")
run("git", "config", "user.email", "davidabram@users.noreply.github.com")
run("git", "checkout", "--detach", PR_HEAD)

tracked_rels = ["skills/sce-decision/SKILL.md", "skills/sce-decision/references/adr-template.md", "skills/sce-next-task/references/context-sync.md"]
before = {target: sum(len((Path(target) / rel).read_text().splitlines()) for rel in tracked_rels) for target in TARGETS}

OLD_DESCRIPTION = "  Write one immutable ADR for one qualifying system-wide decision"
NEW_DESCRIPTION = "  Write one immutable ADR for one decision already qualified by context synchronization"
OLD_PURPOSE = """Write exactly one architecture decision record for one qualifying system-wide
important decision during successful task context synchronization. Return
a deterministic internal handoff to the invoking synchronization phase. Do not
render an independent user-visible response.
"""
NEW_PURPOSE = """Write exactly one architecture decision record for one decision already qualified
by successful task context synchronization. Return a deterministic internal
handoff to the invoking synchronization phase. Do not render an independent
user-visible response.
"""
OLD_GATE_START = "## Decision gate\n"
NEW_QUALIFICATION = """## Qualification handoff

The invoking task context synchronization phase solely owns the decision
qualification threshold and decides whether this skill is invoked. Treat the
caller's gate result as authoritative; do not redefine, broaden, or independently
rerun that threshold here.

Require the request to state why the caller qualified the decision and include
supporting evidence. If the request explicitly represents a nonqualifying gate
outcome, return `not_qualified` (or `skipped` when the caller deliberately skipped
the gate) without writing an ADR. If a claimed qualification is unsupported by
its supplied evidence, return `not_qualified`. Missing, contradictory, or
otherwise unsafe material input remains `blocked`.

`not_qualified` and `skipped` are non-blocking to the invoking synchronization
phase.

"""
OLD_STEP4_START = "### 4. Write the ADR\n"
NEW_STEP4 = """### 4. Write the ADR

Read `references/adr-template.md` before writing. It is the sole authority for
the persisted ADR schema and section semantics. Populate that template from the
validated request and evidence; do not restate or invent a parallel section
contract here.

Use repository-relative Markdown links where practical. Describe durable truth,
not the implementation session. Do not edit current-state context; the invoking
synchronization phase owns linking the new ADR from authoritative context.

"""
OLD_STATUS_TEMPLATE = "Status: {Proposed|Accepted|Rejected|Deprecated|Superseded}"
NEW_STATUS_TEMPLATE = "Status: {validated decision status}"
OLD_DECISION_PLACEHOLDER = "{Exactly one durable system-wide choice.}"
NEW_DECISION_PLACEHOLDER = "{Exactly one durable choice already qualified by task context synchronization.}"
GATE_OWNER = """This subsection is the sole owner of decision qualification. The threshold below
decides whether `sce-decision` is invoked; the decision skill consumes that gate
result and must not restate or broaden it.

"""

for target in TARGETS:
    skill = Path(target) / "skills/sce-decision/SKILL.md"
    text = skill.read_text()
    text = once(text, OLD_DESCRIPTION, NEW_DESCRIPTION, f"{target} description")
    text = once(text, OLD_PURPOSE, NEW_PURPOSE, f"{target} purpose")
    text = between(text, OLD_GATE_START, "## Workflow\n", NEW_QUALIFICATION, f"{target} qualification")
    text = between(text, OLD_STEP4_START, "### 5. Verify the record\n", NEW_STEP4, f"{target} ADR write")
    skill.write_text(text)
    template = Path(target) / "skills/sce-decision/references/adr-template.md"
    body = template.read_text()
    body = once(body, OLD_STATUS_TEMPLATE, NEW_STATUS_TEMPLATE, f"{target} template status")
    body = once(body, OLD_DECISION_PLACEHOLDER, NEW_DECISION_PLACEHOLDER, f"{target} template decision")
    template.write_text(body)
    sync = Path(target) / "skills/sce-next-task/references/context-sync.md"
    sync_text = sync.read_text()
    marker = "## 3.5 Record qualifying architecture decisions\n\n"
    sync_text = once(sync_text, marker, marker + GATE_OWNER, f"{target} gate owner")
    sync.write_text(sync_text)

canonical = Path("config/pkl/base/decision-skill.pkl")
text = canonical.read_text()
text = once(text, 'local SKILL_DESCRIPTION = "Write one immutable ADR for one qualifying system-wide decision"', 'local SKILL_DESCRIPTION = "Write one immutable ADR for one decision already qualified by context synchronization"', "canonical description")
text = once(text, OLD_PURPOSE, NEW_PURPOSE, "canonical purpose")
text = between(text, OLD_GATE_START, "## Workflow\n", NEW_QUALIFICATION, "canonical qualification")
text = between(text, OLD_STEP4_START, "### 5. Verify the record\n", NEW_STEP4, "canonical ADR write")
text = once(text, OLD_STATUS_TEMPLATE, NEW_STATUS_TEMPLATE, "canonical template status")
text = once(text, OLD_DECISION_PLACEHOLDER, NEW_DECISION_PLACEHOLDER, "canonical template decision")
canonical.write_text(text)

sync_canonical = Path("config/pkl/base/workflow-context-sync.pkl")
sync_text = sync_canonical.read_text()
gate_start = 'local decisionGate = (evidenceSource: String, mode: workflow.WorkflowRenderMode) -> """\n'
sync_text = once(sync_text, gate_start, gate_start + "    " + GATE_OWNER.replace("\n", "\n    ").rstrip() + "\n\n", "canonical gate owner")
explicit_gate_heading = "## 3.5 Record qualifying architecture decisions\n\n"
sync_text = once(sync_text, explicit_gate_heading, explicit_gate_heading + GATE_OWNER, "canonical taskReference gate owner")
sync_canonical.write_text(sync_text)

contract = Path("config/pkl/renderers/generation-contract-check.pkl")
c = contract.read_text()
start = "local requiredDecisionSkillTokens = new Listing {\n"
end = "local requiredHandoverSkillTokens = new Listing {\n"
new_defs = r'''local requiredDecisionSkillTokens = new Listing {
  "already qualified"
  "YYYY-MM-DD-<decision-slug>.md"
  "`Proposed`"
  "`Accepted`"
  "`Rejected`"
  "`Deprecated`"
  "`Superseded`"
  "otherwise default"
  "Never edit an ADR whose status is `Accepted`"
  "creates a new dated ADR"
  "references/adr-template.md"
  "exactly one ADR"
  "`written`"
  "`blocked`"
}

local requiredDecisionOwnershipSkillTokens = new Listing {
  "## Qualification handoff"
  "Treat the caller's gate result as authoritative"
  "sole authority for\nthe persisted ADR schema and section semantics"
}

local requiredDecisionWorkflowTokens = new Listing {
  "Record qualifying architecture decisions"
  "System boundaries or ownership"
  "Public or cross-domain interfaces"
  "Data models or persistence"
  "Compatibility contracts"
  "Security posture"
  "Deployment or distribution strategy"
  "major dependency"
  "Routine implementation details"
  "invoke `sce-decision` once"
  "Reuse a written ADR path"
  "returned `adr_path`"
  "On `blocked`"
}

local requiredDecisionTemplateTokens = new Listing {
  "## Context"
  "## Decision"
  "## Rationale"
  "## Alternatives considered"
  "## Compatibility and risks"
  "## Guardrails"
  "## Consequences"
  "## Follow-up"
  "## References"
}

local forbiddenDecisionSkillGateTokens = new Listing {
  "System boundaries or ownership"
  "Public or cross-domain interfaces"
  "Data models or persistence"
  "Compatibility contracts"
  "Security posture"
  "Deployment or distribution strategy"
  "A major dependency"
  "Routine implementation details, local refactors"
  "similarly durable constraint that is costly or risky to reverse"
}

local forbiddenDecisionSkillTemplateRestatementTokens = new Listing {
  "**Context** states"
  "**Decision** states"
  "**Rationale** explains"
  "**Alternatives considered** names"
  "**Compatibility and risks** states"
  "**Guardrails** records"
  "**Consequences** records"
  "**Follow-up** lists"
  "**References** links"
}

'''
c = between(c, start, end, new_defs, "decision contract definitions")
marker = "local requiredDecisionTemplateTokens = new Listing {\n"
first = c.find(marker)
second = c.find(marker, first + len(marker))
if first < 0 or second < 0:
    raise SystemExit("expected expanded and legacy decision template token definitions")
block_end = c.find("}\n\n", second)
if block_end < 0:
    raise SystemExit("legacy decision template token block end missing")
c = c[:second] + c[block_end + 3:]
assertion_anchor = "local assertHandoverContent = (documents: Mapping) ->\n"
assertion = r'''hidden assertDecisionGateTemplateOwnership = (documents: Mapping) ->
  if (
    new Listing { ".opencode"; ".claude"; ".pi"; ".agents" }.every((target) ->
      let (skillPath = "config/\(target)/skills/sce-decision/SKILL.md")
        let (templatePath = "config/\(target)/skills/sce-decision/references/adr-template.md")
          let (syncPath = "config/\(target)/skills/sce-next-task/references/context-sync.md")
            documents.containsKey(skillPath)
            && documents.containsKey(templatePath)
            && documents.containsKey(syncPath)
            && documents[syncPath].contains("sole owner of decision qualification")
            && requiredDecisionOwnershipSkillTokens.every((token) -> documents[skillPath].contains(token))
            && forbiddenDecisionSkillGateTokens.every((token) -> !documents[skillPath].contains(token))
            && forbiddenDecisionSkillTemplateRestatementTokens.every((token) -> !documents[skillPath].contains(token))
            && documents[templatePath].contains("Status: {validated decision status}")
            && requiredDecisionTemplateTokens.every((token) -> documents[templatePath].contains(token))
            && !documents[templatePath].contains("Proposed|Accepted|Rejected|Deprecated|Superseded")
    )
  ) "sce-decision: context sync owns qualification and ADR template owns persisted section semantics"
  else throw("decision qualification must exist only in context sync, while adr-template.md owns persisted ADR section semantics and sce-decision consumes both contracts")
'''
c = once(c, assertion_anchor, assertion + assertion_anchor, "insert decision ownership assertion")
c = once(c, '  ["decision-package-content"] = assertDecisionContent.apply(decisionSkillDocuments)\n', '  ["decision-package-content"] = assertDecisionContent.apply(decisionSkillDocuments)\n  ["decision-gate-template-ownership"] = assertDecisionGateTemplateOwnership.apply(workflowDocuments)\n', "register decision ownership assertion")
contract.write_text(c)

ownership = Path("context/sce/dedup-ownership-table.md")
o = ownership.read_text()
old_row = "| Standalone ADR writing contract | `decision-skill.pkl` | Cross-target `sce-decision` package; successful task synchronization invokes it through the shared decision gate | intentional/keep |\n"
new_rows = """| Decision qualification gate | `decisionGate` in `workflow-context-sync.pkl` | Successful task context synchronization decides qualification and invokes `sce-decision` once per qualifying decision; `sce-decision` consumes the caller's gate result without restating or broadening the threshold | intentional/keep |
| Standalone ADR lifecycle, history, path, and result contract | `sce-decision/SKILL.md` generated from `skillText` in `decision-skill.pkl` | Cross-target internal `sce-decision` package; consumes one already-qualified request, validates status/history/path safety, writes or reuses one ADR, and returns the internal result | intentional/keep |
| Persisted ADR schema and section semantics | `references/adr-template.md` generated from `templateText` in `decision-skill.pkl` | `sce-decision` reads and populates the template at the write boundary without restating the section schema or field semantics | intentional/keep |
"""
o = once(o, old_row, new_rows, "decision ownership rows")
ownership.write_text(o)

after = {target: sum(len((Path(target) / rel).read_text().splitlines()) for rel in tracked_rels) for target in TARGETS}
reductions = {target: before[target] - after[target] for target in TARGETS}
if len(set(reductions.values())) != 1 or next(iter(reductions.values())) <= 0:
    raise SystemExit(f"unexpected decision reductions: {reductions}")
reduction = next(iter(reductions.values()))

plan = Path("context/plans/compress-workflow-execution-preamble.md")
p = plan.read_text()
p += f"""

## Follow-up: decision qualification and ADR-template ownership

- [x] T10: `Deduplicate decision gate and ADR section semantics` (status:done)
  - Scope: task context-sync decision qualification, the standalone `sce-decision`
    package, `references/adr-template.md`, canonical Pkl, generated semantic checks,
    tracked Pi/Claude/Codex mirrors, and the ownership table.
  - Ownership after change: `decisionGate` in `workflow-context-sync.pkl` solely owns
    the qualification threshold and invocation decision. `sce-decision/SKILL.md`
    consumes the caller's gate result and owns ADR lifecycle/history/path/result
    behavior. `references/adr-template.md` solely owns the persisted ADR schema and
    section semantics.
  - Behavior preserved: only successful task synchronization invokes `sce-decision`;
    routine/reversible changes remain nonqualifying; one request still writes or
    reuses at most one ADR; allowed status/default, immutability, supersession,
    collision handling, nonblocking `not_qualified`/`skipped`, blocking unsafe input,
    and internal result shapes remain unchanged.
  - Result: the selected decision skill/template plus next-task context-sync documents
    shrink by {reduction} Markdown lines per tracked target, {reduction * len(TARGETS)}
    across Pi/Claude/Codex.
  - Verify: `pkl eval config/pkl/renderers/generation-contract-check.pkl`,
    `nix run .#pkl-check-generated`, targeted ownership checks across all tracked
    mirrors, and `git diff --check`.
  - Context synchronization: synced.
"""
plan.write_text(p)

criteria = ["System boundaries or ownership", "Public or cross-domain interfaces", "Data models or persistence", "Compatibility contracts", "Security posture", "Deployment or distribution strategy", "A major dependency", "Routine implementation details, local refactors"]
old_section_rules = ["**Context** states", "**Decision** states", "**Rationale** explains", "**Alternatives considered** names", "**Compatibility and risks** states", "**Guardrails** records", "**Consequences** records", "**Follow-up** lists", "**References** links"]
for target in TARGETS:
    skill = (Path(target) / "skills/sce-decision/SKILL.md").read_text()
    template = (Path(target) / "skills/sce-decision/references/adr-template.md").read_text()
    sync = (Path(target) / "skills/sce-next-task/references/context-sync.md").read_text()
    if not all(token in sync for token in criteria): raise SystemExit(f"{target}: decision gate lost qualification criteria")
    if "sole owner of decision qualification" not in sync: raise SystemExit(f"{target}: decision gate ownership marker missing")
    if any(token in skill for token in criteria) or any(token in skill for token in old_section_rules): raise SystemExit(f"{target}: decision skill still restates owned policy")
    for token in ["## Qualification handoff", "solely owns the decision\nqualification threshold", "sole authority for\nthe persisted ADR schema and section semantics", "`not_qualified`", "`skipped`"]:
        if token not in skill: raise SystemExit(f"{target}: decision skill missing consumer token {token!r}")
    for token in ["Status: {validated decision status}", "## Context", "## Decision", "## Rationale", "## Alternatives considered", "## Compatibility and risks", "## Guardrails", "## Consequences", "## Follow-up", "## References"]:
        if token not in template: raise SystemExit(f"{target}: ADR template missing {token!r}")
    if "Proposed|Accepted|Rejected|Deprecated|Superseded" in template: raise SystemExit(f"{target}: template still owns status enum")

run("pkl", "eval", "config/pkl/renderers/generation-contract-check.pkl")
run("nix", "run", ".#pkl-check-generated")
run("git", "diff", "--check")
planned = {"config/pkl/base/decision-skill.pkl", "config/pkl/base/workflow-context-sync.pkl", "config/pkl/renderers/generation-contract-check.pkl", "context/plans/compress-workflow-execution-preamble.md", "context/sce/dedup-ownership-table.md"}
for target in TARGETS:
    planned |= {f"{target}/skills/sce-decision/SKILL.md", f"{target}/skills/sce-decision/references/adr-template.md", f"{target}/skills/sce-next-task/references/context-sync.md"}
changed = set(subprocess.check_output(["git", "diff", "--name-only"], text=True).splitlines())
if changed != planned: raise SystemExit(f"unexpected changed paths: extra={sorted(changed - planned)} missing={sorted(planned - changed)}")
print(f"decision skill/template/context-sync Markdown reduction per tracked target: {reduction}")
print(f"total tracked Pi/Claude/Codex reduction: {reduction * len(TARGETS)}")
run("git", "add", *sorted(planned))
run("git", "commit", "-m", "config: Deduplicate decision gate and ADR template")
run("git", "push", "origin", f"HEAD:{PR_BRANCH}")
print(subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip())
