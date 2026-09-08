from pathlib import Path

path = Path(".sce-validation-report-dedup.py")
text = path.read_text()
old = """Do not author this section while planning. Only `/validate` through the **Validation phase**
writes it."""
new = """Do not author this section while planning. Only `/validate` through `sce-validation`
writes it."""
count = text.count(old)
if count != 2:
    raise SystemExit(f"expected 2 rendered intro spellings, found {count}")
path.write_text(text.replace(old, new))
