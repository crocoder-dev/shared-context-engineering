from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
text = path.read_text()
new_assertion = """assertion = r'''hidden assertDecisionGateTemplateOwnership = (workflowDocs: Mapping, decisionDocs: Mapping) ->
  if (
    workflowDocs.every((path, text) ->
      !path.endsWith(\"/skills/sce-next-task/references/context-sync.md\")
      || text.contains(\"This subsection is the sole owner of decision qualification.\")
    )
    && decisionDocs.every((path, text) ->
      !path.endsWith(\"/skills/sce-decision/SKILL.md\")
      || (
        text.contains(\"## Qualification handoff\")
        && text.contains(\"Treat the caller's gate result as authoritative\")
        && text.contains(\"Read `references/adr-template.md` before writing.\")
        && !text.contains(\"## Decision gate\")
        && !text.contains(\"**Context** states\")
        && !text.contains(\"**Decision** states\")
      )
    )
    && decisionDocs.every((path, text) ->
      !path.endsWith(\"/skills/sce-decision/references/adr-template.md\")
      || (
        text.contains(\"Status: {validated decision status}\")
        && text.contains(\"## Context\")
        && text.contains(\"## Decision\")
        && text.contains(\"## Rationale\")
        && !text.contains(\"Status: {Proposed|Accepted|Rejected|Deprecated|Superseded}\")
      )
    )
  ) \"sce-decision: context sync owns qualification and ADR template owns persisted section semantics\"
  else throw(\"decision qualification must be owned by next-task context-sync while sce-decision consumes the gate and adr-template.md owns persisted ADR schema\")
'''"""
pattern = r"assertion = r'''hidden assertDecisionGateTemplateOwnership = \(documents: Mapping\) ->.*?\n'''"
text, count = re.subn(pattern, new_assertion, text, count=1, flags=re.S)
if count != 1:
    raise SystemExit(f"original T10 ownership assertion replacement count: {count}")
old_registration = '["decision-gate-template-ownership"] = assertDecisionGateTemplateOwnership.apply(workflowDocuments)'
new_registration = '["decision-gate-template-ownership"] = assertDecisionGateTemplateOwnership.apply(workflowDocuments, decisionSkillDocuments)'
if old_registration not in text:
    raise SystemExit("original T10 ownership registration not found")
path.write_text(text.replace(old_registration, new_registration, 1))