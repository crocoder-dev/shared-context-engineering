# `sce-project-overview-html` skill

## What this skill is for

The `sce-project-overview-html` skill renders the project's current-state `context/` Markdown memory into a single self-contained HTML document so a human reader can quickly understand the project and how it works. It is owned by the Shared Context Code agent and is invocable through the Shared Context Code agent's `skill:` allowlist in both manual and automated OpenCode profiles.

Use this skill when the user asks for a project overview as HTML, a readable summary page, a shareable walkthrough, or a single-document view of the shared context memory.

## Source of truth

- The source of truth is `context/` only. The skill does not scan or analyze application code.
- It renders the current-state context memory into HTML as-is. If context is stale, the HTML reflects stale context; the skill does not repair context (that is `sce-context-sync`'s job).

## Files read

The skill reads, in order:

1. `context/overview.md`
2. `context/architecture.md`
3. `context/patterns.md`
4. `context/glossary.md`
5. `context/context-map.md`
6. Every linked domain file discovered by parsing `context/context-map.md` (under `context/cli/`, `context/sce/`, and any other `context/{domain}/` sections).

## Rendering contract

- The agent authors the HTML directly with its own file-write tool. It does NOT generate, run, or emit any conversion script (no Python, no Node, no shell pipeline, no build step). The agent is the renderer.
- Markdown is converted to HTML body markup (headings, paragraphs, lists, tables, code blocks, blockquotes) by the agent as it writes the file.
- Mermaid fenced blocks in context are preserved as `<pre class="mermaid">...</pre>` so the client-side library renders them in-browser. They are not converted to plain code blocks.
- Inline code is preserved as `<code>`; fenced code blocks as `<pre><code>`.
- Heading anchors (`id` attributes) are generated from heading text for in-page navigation.

## HTML structure

- Single self-contained document with a left-side navigation sidebar (`<nav id="toc" class="sidebar">`) and main content (`.content`) to its right.
- Section ordering: Overview, Architecture, Patterns, Glossary, Context Map, then one `<section>` per linked domain file ordered as listed in `context-map.md`.
- The sidebar stays visible while the main content scrolls (sticky positioning) and collapses on narrow viewports.
- All CSS is inlined in a single `<style>` block in the document head. No external CSS frameworks are loaded.

## Mermaid.js dependency

- Mermaid.js is loaded from CDN at view time: `<script src="https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.min.js"></script>`.
- Initialized with `mermaid.initialize({ startOnLoad: true })`.
- No vendoring, no build step, no pre-rendering to SVG. Diagrams require internet to view. Offline support is a non-goal for this iteration.

## Output path and disposable-output policy

- Output is written to `context/tmp/project-overview.html`.
- `context/tmp/` is disposable and gitignored by the existing `context/tmp/.gitignore` (`*` ignore, `!.gitignore`).
- The file is regenerated on demand and is not committed.

## Canonical authoring and generation

- The skill body is authored canonically in `config/pkl/base/shared-content-code.pkl` and `config/pkl/base/shared-content-automated-code.pkl` (shared `UnitSpec`), aggregated in `config/pkl/base/shared-content.pkl` and `config/pkl/base/shared-content-automated.pkl`, and rendered into all three target trees:
  - `config/.opencode/skills/sce-project-overview-html/SKILL.md` (manual OpenCode)
  - `config/automated/.opencode/skills/sce-project-overview-html/SKILL.md` (automated OpenCode)
  - `config/.claude/skills/sce-project-overview-html/SKILL.md` (Claude)
- The repo-root `.opencode/skills/sce-project-overview-html/SKILL.md` is the active runtime copy, kept in sync with the generated `config/.opencode/` output.
- Skill descriptions and the Shared Context Code `skill:` allowlist entry are owned by the four metadata/renderer files: `config/pkl/renderers/common.pkl`, `config/pkl/renderers/opencode-metadata.pkl`, `config/pkl/renderers/opencode-automated-metadata.pkl`, and `config/pkl/renderers/claude-metadata.pkl`.

## Related skills

- `sce-context-sync` — repairs context drift; run it first if context is stale before generating the overview.
- `sce-bootstrap-context` — creates the `context/` baseline this skill reads from.