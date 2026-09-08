from pathlib import Path
p = Path('/tmp/sce-decision-dedup.py')
s = p.read_text()
old_skill = r'''local requiredDecisionSkillTokens = new Listing {
  "already qualified by context synchronization"
  "## Qualification handoff"
  "solely owns the decision\nqualification threshold"
  "Treat the caller's gate result as authoritative"
  "`not_qualified`"
  "`skipped`"
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
  "sole authority for\nthe persisted ADR schema and section semantics"
  "exactly one file"
  "`written`"
  "`blocked`"
}
'''
new_skill = r'''local requiredDecisionSkillTokens = new Listing {
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
  "exactly one file"
  "`written`"
  "`blocked`"
}

local requiredDecisionOwnershipSkillTokens = new Listing {
  "## Qualification handoff"
  "solely owns the decision\nqualification threshold"
  "Treat the caller's gate result as authoritative"
  "`not_qualified`"
  "`skipped`"
  "sole authority for\nthe persisted ADR schema and section semantics"
}
'''
if s.count(old_skill) != 1:
    raise SystemExit(f'expected one expanded decision skill token block, found {s.count(old_skill)}')
s = s.replace(old_skill, new_skill)
old_template = r'''local requiredDecisionTemplateTokens = new Listing {
  "Status: {validated decision status}"
  "## Context"
  "Forces, constraints, and evidence that require this decision"
  "## Decision"
  "Exactly one durable choice already qualified by task context synchronization"
  "## Rationale"
  "Why this choice best satisfies the constraints"
  "## Alternatives considered"
  "Why it was not selected"
  "## Compatibility and risks"
  "Compatibility effect, migration concern, or material risk and mitigation"
  "## Guardrails"
  "Durable limit that keeps the decision narrow"
  "## Consequences"
  "Positive or negative resulting constraint"
  "## Follow-up"
  "Established follow-up work or condition"
  "## References"
}
'''
new_template = r'''local requiredDecisionTemplateTokens = new Listing {
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

local requiredDecisionOwnershipTemplateTokens = new Listing {
  "Status: {validated decision status}"
  "Forces, constraints, and evidence that require this decision"
  "Exactly one durable choice already qualified by task context synchronization"
  "Why this choice best satisfies the constraints"
  "Why it was not selected"
  "Compatibility effect, migration concern, or material risk and mitigation"
  "Durable limit that keeps the decision narrow"
  "Positive or negative resulting constraint"
  "Established follow-up work or condition"
}
'''
if s.count(old_template) != 1:
    raise SystemExit(f'expected one expanded decision template token block, found {s.count(old_template)}')
s = s.replace(old_template, new_template)
old_assert = '''            && requiredDecisionSkillTokens.every((token) -> documents[skillPath].contains(token))\n            && forbiddenDecisionSkillGateTokens.every((token) -> !documents[skillPath].contains(token))\n            && forbiddenDecisionSkillTemplateRestatementTokens.every((token) -> !documents[skillPath].contains(token))\n            && requiredDecisionTemplateTokens.every((token) -> documents[templatePath].contains(token))\n'''
new_assert = '''            && requiredDecisionOwnershipSkillTokens.every((token) -> documents[skillPath].contains(token))\n            && forbiddenDecisionSkillGateTokens.every((token) -> !documents[skillPath].contains(token))\n            && forbiddenDecisionSkillTemplateRestatementTokens.every((token) -> !documents[skillPath].contains(token))\n            && requiredDecisionTemplateTokens.every((token) -> documents[templatePath].contains(token))\n            && requiredDecisionOwnershipTemplateTokens.every((token) -> documents[templatePath].contains(token))\n'''
if s.count(old_assert) != 1:
    raise SystemExit(f'expected one ownership assertion token block, found {s.count(old_assert)}')
s = s.replace(old_assert, new_assert)
p.write_text(s)
