from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
text = path.read_text()
new_assertion = """assertion = r'''hidden assertDecisionGateTemplateOwnership = (artifacts: Mapping) ->
  let (contexts = new Listing {
    artifacts[\"config/.pi/skills/sce-next-task/references/context-sync.md\"]
    artifacts[\"config/.claude/skills/sce-next-task/references/context-sync.md\"]
    artifacts[\"config/.agents/skills/sce-next-task/references/context-sync.md\"]
  })
    let (skills = new Listing {
      artifacts[\"config/.pi/skills/sce-decision/SKILL.md\"]
      artifacts[\"config/.claude/skills/sce-decision/SKILL.md\"]
      artifacts[\"config/.agents/skills/sce-decision/SKILL.md\"]
    })
      let (templates = new Listing {
        artifacts[\"config/.pi/skills/sce-decision/references/adr-template.md\"]
        artifacts[\"config/.claude/skills/sce-decision/references/adr-template.md\"]
        artifacts[\"config/.agents/skills/sce-decision/references/adr-template.md\"]
      })
        if (
          contexts.every((text) -> text.contains(\"sole owner of decision qualification\"))
          && skills.every((text) ->
            text.contains(\"## Qualification handoff\")
            && text.contains(\"caller's gate result as authoritative\")
            && text.contains(\"sole authority for\")
            && text.contains(\"persisted ADR schema and section semantics\")
            && !text.contains(\"## Decision gate\")
            && !text.contains(\"**Context** states\")
            && !text.contains(\"**Decision** states\")
          )
          && templates.every((text) ->
            text.contains(\"Status: {validated decision status}\")
            && text.contains(\"## Context\")
            && text.contains(\"## Decision\")
            && text.contains(\"## Rationale\")
            && !text.contains(\"Proposed|Accepted|Rejected|Deprecated|Superseded\")
          )
        ) \"sce-decision: context sync owns qualification and ADR template owns persisted section semantics\"
        else throw(\"decision qualification must be owned by tracked context-sync outputs while sce-decision consumes the gate and adr-template.md owns persisted ADR schema\")
'''"""
pattern = r"assertion = r'''hidden assertDecisionGateTemplateOwnership = \(documents: Mapping\) ->.*?\n'''"
text, count = re.subn(pattern, new_assertion, text, count=1, flags=re.S)
if count != 1:
    raise SystemExit(f"original T10 ownership assertion replacement count: {count}")
old_registration = '["decision-gate-template-ownership"] = assertDecisionGateTemplateOwnership.apply(workflowDocuments)'
new_registration = '["decision-gate-template-ownership"] = assertDecisionGateTemplateOwnership.apply(generatedArtifacts)'
if old_registration not in text:
    raise SystemExit("original T10 ownership registration not found")
path.write_text(text.replace(old_registration, new_registration, 1))