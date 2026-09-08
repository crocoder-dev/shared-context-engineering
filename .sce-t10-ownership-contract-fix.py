from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
old = r'''assertion = r'''hidden assertDecisionGateTemplateOwnership = (documents: Mapping) ->
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
'''
new = r'''assertion = r'''hidden assertDecisionGateTemplateOwnership = (workflowDocs: Mapping, decisionDocs: Mapping) ->
  if (
    workflowDocs.every((path, text) ->
      !path.endsWith("/skills/sce-next-task/references/context-sync.md")
      || text.contains("sole owner of decision qualification")
    )
    && decisionDocs.every((path, text) ->
      if (path.endsWith("/skills/sce-decision/SKILL.md"))
        requiredDecisionOwnershipSkillTokens.every((token) -> text.contains(token))
        && forbiddenDecisionSkillGateTokens.every((token) -> !text.contains(token))
        && forbiddenDecisionSkillTemplateRestatementTokens.every((token) -> !text.contains(token))
      else if (path.endsWith("/skills/sce-decision/references/adr-template.md"))
        text.contains("Status: {validated decision status}")
        && requiredDecisionTemplateTokens.every((token) -> text.contains(token))
        && !text.contains("Proposed|Accepted|Rejected|Deprecated|Superseded")
      else true
    )
  ) "sce-decision: context sync owns qualification and ADR template owns persisted section semantics"
  else throw("decision qualification must exist only in next-task context-sync, while adr-template.md owns persisted ADR section semantics and sce-decision consumes both contracts")
'''
'''
if old not in text:
    raise SystemExit("original T10 ownership assertion not found")
text = text.replace(old, new, 1)
old_registration = '["decision-gate-template-ownership"] = assertDecisionGateTemplateOwnership.apply(workflowDocuments)'
new_registration = '["decision-gate-template-ownership"] = assertDecisionGateTemplateOwnership.apply(workflowDocuments, decisionSkillDocuments)'
if old_registration not in text:
    raise SystemExit("original T10 ownership registration not found")
path.write_text(text.replace(old_registration, new_registration, 1))
