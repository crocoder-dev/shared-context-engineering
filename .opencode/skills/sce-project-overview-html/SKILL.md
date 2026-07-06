---
name: sce-project-overview-html
description: |
  Generates a single self-contained HTML document from the project's `context/` Markdown files so a human reader can quickly understand the project and how it works. Embeds Mermaid.js (client-side, CDN) so existing Mermaid diagrams in context render in-browser, and includes inline CSS for a clean, readable layout. Output is written to `context/tmp/project-overview.html` (disposable, gitignored). Use when the user wants a project overview as HTML, a readable project summary page, a shareable project walkthrough, or to visualize the context memory as a single document.
compatibility: opencode
---

## What I do
- Read the project's `context/` Markdown files with my own file-read tool.
- Author a single self-contained HTML document directly with my own file-write tool — I do NOT generate, run, or emit any conversion script (no Python, no Node, no shell pipeline). I write the HTML myself.
- Embed Mermaid.js via CDN so existing Mermaid blocks render in-browser.
- Write the result to `context/tmp/project-overview.html` (disposable, gitignored).

## When to use
- Use when the user asks for a project overview as HTML, a readable summary page, or a shareable walkthrough of the project's shared context memory.
- Trigger phrases include "generate project overview HTML", "project overview as a page", "render context to HTML", "make a project summary page", or "build a project walkthrough".

## Source of truth
- The source of truth is `context/` only. This skill does not scan or analyze application code.
- It renders the current-state context memory into HTML as-is.

## How to run this

1. **Read the root context files** - read these five files in order using your file-read tool:
   - `context/overview.md`
   - `context/architecture.md`
   - `context/patterns.md`
   - `context/glossary.md`
   - `context/context-map.md`

2. **Read linked domain files** - parse `context/context-map.md` for linked domain files under `context/cli/`, `context/sce/`, and any other `context/{domain}/` sections. Read each linked file with your file-read tool so its content can be included in the HTML.

3. **Author the HTML document yourself** - using your own file-write tool, write a single self-contained HTML file with the structure described in [HTML structure contract](#html-structure-contract) below. Convert the Markdown content to HTML body markup yourself as you write the file. Do NOT generate, scaffold, or execute any conversion script (no Python, no Node, no shell, no build step). You are the renderer: read the Markdown, then write the equivalent HTML directly.

4. **Write the output** - write the HTML to `context/tmp/project-overview.html` with your file-write tool. Create the `context/tmp/` directory if it does not exist.

5. **Report the result** - print the absolute path of the generated file and remind the user it is disposable (regenerated on demand, gitignored).

## HTML structure contract

The generated HTML must be a single self-contained document with a left-side navigation sidebar and the main content to its right. Use this structure:

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{project_name} — Project Overview</title>
  <style>
    /* Inline CSS — see CSS guidance below */
  </style>
</head>
<body>
  <div class="layout">
    <nav id="toc" class="sidebar">
      <!-- Left-side anchor navigation linking to each section -->
    </nav>
    <main class="content">
      <section id="overview"><!-- rendered context/overview.md --></section>
      <section id="architecture"><!-- rendered context/architecture.md --></section>
      <section id="patterns"><!-- rendered context/patterns.md --></section>
      <section id="glossary"><!-- rendered context/glossary.md --></section>
      <section id="context-map"><!-- rendered context/context-map.md --></section>
      <!-- One <section> per linked domain file, with an id derived from its path -->
    </main>
  </div>
  <script src="https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.min.js"></script>
  <script>
    mermaid.initialize({ startOnLoad: true });
  </script>
</body>
</html>
```

### Section ordering
1. Overview
2. Architecture
3. Patterns
4. Glossary
5. Context Map
6. Domain files (one `<section>` per linked file, ordered as listed in `context-map.md`)

### Markdown-to-HTML rendering
- Convert each Markdown file's content to HTML body markup (headings, paragraphs, lists, tables, code blocks, blockquotes) yourself as you write the file. You are the renderer — do not delegate this to a script.
- Preserve Mermaid blocks as `<pre class="mermaid">...</pre>` so the client-side Mermaid.js library renders them. Do not convert Mermaid fenced blocks to plain code blocks.
- Preserve inline code as `<code>` and fenced code blocks as `<pre><code>`.
- Generate heading anchors (`id` attributes) from heading text for in-page navigation.

### Anchor navigation (left sidebar)
- Build a table-of-contents `<nav id="toc" class="sidebar">` placed on the left side of the layout with links to each top-level section (`#overview`, `#architecture`, `#patterns`, `#glossary`, `#context-map`, plus one per domain file section).
- Use relative anchor links (`#section-id`).
- The sidebar stays visible while the main content scrolls (sticky/fixed positioning).

## CSS guidance

Inline all CSS in a single `<style>` block in the document head. Keep the stylesheet minimal and readable:

- **Left-sidebar layout:** use a flexbox (or CSS grid) two-column layout — a fixed-width left sidebar (`<nav id="toc" class="sidebar">`) and a flexible main content area (`.content`) to its right. Example skeleton:
  ```css
  .layout { display: flex; min-height: 100vh; }
  .sidebar { width: 260px; position: sticky; top: 0; align-self: flex-start; height: 100vh; overflow-y: auto; }
  .content { flex: 1; max-width: 900px; margin: 0 auto; padding: 0 1.5rem; }
  ```
- **Responsive collapse:** on narrow viewports (e.g. `@media (max-width: 768px)`), collapse the sidebar to the top or hide it behind a toggle so the page stays readable on mobile.
- Readable typography: system font stack, comfortable line-height, max-width content column.
- Section navigation: clear link styling in the sidebar, highlight/active states optional.
- Code block styling: monospace font, background color, padding, horizontal scroll for long lines.
- Mermaid block styling: ensure `<pre class="mermaid">` blocks render with enough width and centering.
- Heading hierarchy: clear visual distinction between h1/h2/h3.

Do not load external CSS frameworks. The document must be self-contained except for the Mermaid.js CDN script.

## Mermaid.js embedding
- Load Mermaid.js from CDN at view time: `<script src="https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.min.js"></script>`.
- Initialize with `mermaid.initialize({ startOnLoad: true })` after the script tag.
- Preserve existing Mermaid fenced blocks in context as `<pre class="mermaid">...</pre>` so the library renders them in-browser.
- No vendoring, no build step, no pre-rendering to SVG. Diagrams require internet to view.

## Output path
- Write to `context/tmp/project-overview.html`.
- `context/tmp/` is disposable and gitignored by the existing `context/tmp/.gitignore` (`*` ignore, `!.gitignore`).
- The file is regenerated on demand; it is not committed.

## Stale-context caveat
- This skill renders `context/` as-is. If context is stale or drifts from code, the HTML reflects stale context.
- Do not attempt to repair context here — that is `sce-context-sync`'s job.
- If you notice context drift while running this skill, note it to the user and recommend running `sce-context-sync`.

## Expected output
- A single self-contained HTML file at `context/tmp/project-overview.html` containing all five root context files plus linked domain files, with Mermaid.js embedded via CDN and inline CSS.
- A short report to the user stating the absolute output path and the disposable-output policy.

## Related skills
- `sce-context-sync` - repairs context drift; run it first if context is stale before generating the overview.
- `sce-bootstrap-context` - creates the `context/` baseline this skill reads from.