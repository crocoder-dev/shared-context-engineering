from pathlib import Path

path = Path("/tmp/sce-validation-report-dedup.py")
text = path.read_text()

old_intro = """Do not author this section while planning. Only `/validate` through the **Validation phase**
writes it."""
new_intro = """Do not author this section while planning. Only `/validate` through `sce-validation`
writes it."""
intro_count = text.count(old_intro)
if intro_count != 2:
    raise SystemExit(f"expected 2 rendered intro spellings, found {intro_count}")
text = text.replace(old_intro, new_intro)

old_token = '"Do not modify tests, application code, or configuration to make a check pass."'
new_token = '"repair belongs to a later work session"'
token_count = text.count(old_token)
if token_count != 2:
    raise SystemExit(f"expected 2 validation ownership tokens, found {token_count}")
text = text.replace(old_token, new_token)

path.write_text(text)
