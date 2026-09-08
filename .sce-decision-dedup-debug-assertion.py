from pathlib import Path
p = Path('/tmp/sce-decision-dedup.py')
s = p.read_text()
start = 'assertion = r"""hidden assertDecisionGateTemplateOwnership = (documents: Mapping) ->\n'
end = '\n"""\nc = once(c, assertion_anchor, assertion + assertion_anchor, "insert decision ownership assertion")'
if s.count(start) != 1 or s.count(end) != 1:
    raise SystemExit(f'assertion variable markers wrong: start={s.count(start)} end={s.count(end)}')
left, rest = s.split(start, 1)
_, right = rest.split(end, 1)
replacement = r'''assertion = r"""hidden assertDecisionGateTemplateOwnership = (documents: Mapping) ->
  let (target = ".pi")
    let (skillPath = "config/\(target)/skills/sce-decision/SKILL.md")
      let (templatePath = "config/\(target)/skills/sce-decision/references/adr-template.md")
        let (syncPath = "config/\(target)/skills/sce-next-task/references/context-sync.md")
          if (!documents.containsKey(skillPath)) throw("T10 debug: Pi decision skill path missing")
          else if (!documents.containsKey(templatePath)) throw("T10 debug: Pi ADR template path missing")
          else if (!documents.containsKey(syncPath)) throw("T10 debug: Pi context-sync path missing")
          else if (!documents[syncPath].contains("sole owner of decision qualification")) throw("T10 debug: Pi context-sync owner marker missing")
          else if (!documents[skillPath].contains("## Qualification handoff")) throw("T10 debug: Pi qualification handoff missing")
          else if (!documents[skillPath].contains("Treat the caller's gate result as authoritative")) throw("T10 debug: Pi caller-authority pointer missing")
          else if (!documents[skillPath].contains("sole authority for\nthe persisted ADR schema and section semantics")) throw("T10 debug: Pi template-owner pointer missing")
          else if (documents[skillPath].contains("## Decision gate")) throw("T10 debug: Pi old decision gate remains")
          else if (documents[skillPath].contains("**Context** states")) throw("T10 debug: Pi section semantics remain")
          else if (!documents[templatePath].contains("Status: {validated decision status}")) throw("T10 debug: Pi validated-status placeholder missing")
          else if (documents[templatePath].contains("Proposed|Accepted|Rejected|Deprecated|Superseded")) throw("T10 debug: Pi template still owns status enum")
          else "sce-decision: Pi ownership seams present"
"""'''
p.write_text(left + replacement + end + right)
