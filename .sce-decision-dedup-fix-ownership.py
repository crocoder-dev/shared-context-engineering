from pathlib import Path
p = Path('/tmp/sce-decision-dedup.py')
s = p.read_text()
old = r'''hidden assertDecisionGateTemplateOwnership = (documents: Mapping) ->
  if (
    new Listing { ".opencode"; ".claude"; ".pi"; ".agents" }.every((target) ->
      let (skillPath = "config/\(target)/skills/sce-decision/SKILL.md")
        let (templatePath = "config/\(target)/skills/sce-decision/references/adr-template.md")
          let (syncPath = "config/\(target)/skills/sce-next-task/references/context-sync.md")
            documents.containsKey(skillPath)
            && documents.containsKey(templatePath)
            && documents.containsKey(syncPath)
            && requiredDecisionWorkflowTokens.every((token) -> documents[syncPath].contains(token))
            && requiredDecisionOwnershipSkillTokens.every((token) -> documents[skillPath].contains(token))
            && forbiddenDecisionSkillGateTokens.every((token) -> !documents[skillPath].contains(token))
            && forbiddenDecisionSkillTemplateRestatementTokens.every((token) -> !documents[skillPath].contains(token))
            && requiredDecisionTemplateTokens.every((token) -> documents[templatePath].contains(token))
            && requiredDecisionOwnershipTemplateTokens.every((token) -> documents[templatePath].contains(token))
            && forbiddenDecisionTemplateGateTokens.every((token) -> !documents[templatePath].contains(token))
    )
  ) "sce-decision: context sync owns qualification and ADR template owns persisted section semantics"
  else throw("decision qualification must exist only in context sync, while adr-template.md owns persisted ADR section semantics and sce-decision consumes both contracts")

'''
new = r'''hidden assertDecisionGateTemplateOwnership = (documents: Mapping) ->
  if (
    new Listing { ".opencode"; ".claude"; ".pi"; ".agents" }.every((target) ->
      let (skillPath = "config/\(target)/skills/sce-decision/SKILL.md")
        let (templatePath = "config/\(target)/skills/sce-decision/references/adr-template.md")
          let (syncPath = "config/\(target)/skills/sce-next-task/references/context-sync.md")
            documents.containsKey(skillPath)
            && documents.containsKey(templatePath)
            && documents.containsKey(syncPath)
            && documents[syncPath].contains("sole owner of decision qualification")
            && documents[syncPath].contains("System boundaries or ownership")
            && documents[syncPath].contains("Routine implementation details")
            && requiredDecisionOwnershipSkillTokens.every((token) -> documents[skillPath].contains(token))
            && forbiddenDecisionSkillGateTokens.every((token) -> !documents[skillPath].contains(token))
            && forbiddenDecisionSkillTemplateRestatementTokens.every((token) -> !documents[skillPath].contains(token))
            && documents[templatePath].contains("Status: {validated decision status}")
            && requiredDecisionTemplateTokens.every((token) -> documents[templatePath].contains(token))
            && !documents[templatePath].contains("Proposed|Accepted|Rejected|Deprecated|Superseded")
            && forbiddenDecisionTemplateGateTokens.every((token) -> !documents[templatePath].contains(token))
    )
  ) "sce-decision: context sync owns qualification and ADR template owns persisted section semantics"
  else throw("decision qualification must exist only in context sync, while adr-template.md owns persisted ADR section semantics and sce-decision consumes both contracts")

'''
if s.count(old) != 1:
    raise SystemExit(f'expected one over-specified ownership assertion, found {s.count(old)}')
p.write_text(s.replace(old, new))
