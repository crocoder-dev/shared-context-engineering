---
name: sce-context-generate-html
description: |
  Generates static, human-readable HTML documentation from refreshed Shared Context Engineering files. Use when a user wants browsable project documentation, an HTML overview, or a human-friendly context site under `context/html/`; always runs `sce-context-sync` first and writes `context/html/index.html` as the default entrypoint.
compatibility: claude
---

## What I do
- Generate static, human-readable HTML documentation from the current `context/` state.
- Produce files under `context/html/`, with `context/html/index.html` as the default entrypoint.

## Required pre-generation sync
1. Load and run `sce-context-sync` before writing HTML.
2. Treat the refreshed `context/` files as the source of truth for the generated documentation.
3. If `sce-context-sync` finds unresolved drift, repair or ask for direction before generating HTML.

## Inputs to read
- Start with `context/context-map.md`, `context/overview.md`, `context/architecture.md`, `context/patterns.md`, and `context/glossary.md`.
- Follow relevant domain links from `context/context-map.md` for the requested audience or topic.
- Do not invent behavior that is not represented in current context or verified code.

## Output contract
- Write static documentation under `context/html/`.
- Make `context/html/index.html` the human-readable entrypoint.
- Use local assets only: inline CSS or local CSS files under `context/html/`.
- Keep output deterministic and reviewable; avoid opaque build steps or new dependencies by default.
- Include project overview content derived from existing context files.
- Prefer concise pages with clear headings, navigation, and links back to source context files.

## Diagram requirements
- Include at least one diagram when it helps explain project structure, boundaries, or flows.
- Diagrams must be visibly rendered in a browser, not left as raw unsupported Mermaid or code-fence text.
- If using Mermaid, use only an already-available or explicitly approved local renderer/runtime, initialize it on page load, and verify the rendered browser output.
- If Mermaid rendering is not available, convert the diagram to plain HTML/CSS or SVG so the browser displays it directly.

## Verification checklist
- Open or inspect `context/html/index.html` and confirm the page links, CSS, and navigation are usable.
- Confirm diagrams render visibly in browser-compatible HTML rather than appearing as raw source syntax.
- Confirm generated files stay under `context/html/` and no unrelated context files are changed except as required by the pre-generation `sce-context-sync` pass.

## Quality constraints
- Keep generated HTML static, readable, and deterministic.
- Prefer semantic HTML and accessible contrast.
- Keep CSS small and local to the generated docs.
- Record any skipped or degraded diagram rendering checks in the final response.
