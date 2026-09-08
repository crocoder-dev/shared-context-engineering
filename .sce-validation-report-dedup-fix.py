from pathlib import Path

path = Path('.sce-validation-report-dedup.py')
text = path.read_text()
old = 'through the **Validation phase**\\nwrites it.'
new = 'through `sce-validation`\\nwrites it.'
count = text.count(old)
if count != 2:
    raise SystemExit(f'expected 2 rendered intro spellings, found {count}')
path.write_text(text.replace(old, new))
