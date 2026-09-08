from pathlib import Path

path = Path("/tmp/sce-handover-schema-dedup.py")
text = path.read_text()

old_defs = 'new_defs = """local requiredHandoverSkillTokens = new Listing {'
new_defs = 'new_defs = r"""local requiredHandoverSkillTokens = new Listing {'
old_assertion = 'assertion = """hidden assertHandoverSchemaCompletenessOwnership = (documents: Mapping) ->'
new_assertion = 'assertion = r"""hidden assertHandoverSchemaCompletenessOwnership = (documents: Mapping) ->'

if text.count(old_defs) != 1:
    raise SystemExit("expected exactly one new_defs block")
if text.count(old_assertion) != 1:
    raise SystemExit("expected exactly one assertion block")

text = text.replace(old_defs, new_defs)
text = text.replace(old_assertion, new_assertion)
path.write_text(text)
