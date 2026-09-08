from pathlib import Path
p = Path('/tmp/sce-decision-dedup.py')
s = p.read_text()
start = 'hidden assertDecisionGateTemplateOwnership = (documents: Mapping) ->\n'
end = 'local assertHandoverContent = (documents: Mapping) ->\n'
if s.count(start) != 1 or s.count(end) != 1:
    raise SystemExit('decision ownership assertion markers missing')
left, rest = s.split(start, 1)
_, right = rest.split(end, 1)
replacement = r'''hidden assertDecisionGateTemplateOwnership = (documents: Mapping) ->
  let (target = ".pi")
    let (skillPath = "config/\(target)/skills/sce-decision/SKILL.md")
      let (templatePath = "config/\(target)/skills/sce-decision/references/adr-template.md")
        let (syncPath = "config/\(target)/skills/sce-next-task/references/context-sync.md")
          if (
            documents.containsKey(skillPath)
            && documents.containsKey(templatePath)
            && documents.containsKey(syncPath)
            && documents[syncPath].contains("sole owner of decision qualification")
            && documents[skillPath].contains("## Qualification handoff")
            && documents[skillPath].contains("sole authority for")
            && documents[templatePath].contains("Status: {validated decision status}")
          ) "sce-decision: Pi ownership seams present"
          else throw("sce-decision Pi ownership seam missing")

'''
p.write_text(left + replacement + end + right)
