# Shared Context Engineering

AI-assisted software delivery with explicit, versioned context.

---

## Run the CLI

Use the [WIP] sce cli at `cli/`. See `cli/README.md` for current behavior and usage.

- [Docs](https://sce.crocoder.dev/docs)
- [Getting Started](https://sce.crocoder.dev/docs/getting-started)
- [Motivation](https://sce.crocoder.dev/docs/motivation)

## Tessl skills

The publish workflow at `.github/workflows/publish-tiles.yml` treats each skill directory under `config/.opencode/skills/` and `config/.claude/skills/` as its own Tessl tile.

Before GitHub Actions can publish a tile, generate a `tile.json` inside each skill directory you want to publish:

```sh
tessl skill import ./config/.opencode/skills/sce-plan-authoring --workspace <myworkspace>
tessl skill import ./config/.claude/skills/sce-plan-authoring --workspace <myworkspace>
```

Repeat that for any other skill folders you want Tessl to manage, then add the `TESSL_API_TOKEN` repository secret described in the Tessl publishing docs.

Built by [CroCoder](https://www.crocoder.dev/)
