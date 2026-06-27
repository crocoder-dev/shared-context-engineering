# Shared Context Engineering (SCE)

[![crates.io](https://img.shields.io/crates/v/shared-context-engineering?logo=rust)](https://crates.io/crates/shared-context-engineering)
[![npm](https://img.shields.io/npm/v/%40crocoder-dev%2Fsce?logo=npm)](https://www.npmjs.com/package/@crocoder-dev/sce)
[![Nix CI](https://github.com/crocoder-dev/shared-context-engineering/actions/workflows/pr-ci.yml/badge.svg?branch=main)](https://github.com/crocoder-dev/shared-context-engineering/actions/workflows/pr-ci.yml)

**AI made code generation fast. Team alignment didn't keep up.**

SCE treats the *why* behind your code, architecture, decisions, constraints, as a versioned, shared artifact that both your team and your AI agents work from.

- a repo-owned `context/` directory holding the architecture, decisions, and constraints AI agents otherwise re-derive every session
- generated configs that make OpenCode and Claude Code actually read it
- a Bash policy that keeps agents inside your repo's rules
- hooks that capture agent activity at the commit boundary
- a local Agent Trace SQLite database linking each commit to the session that produced it

## Quick start

Install the `sce` CLI through whichever channel fits your environment:

```bash
# npm
npm install -g @crocoder-dev/sce

# cargo
cargo install shared-context-engineering --locked

# nix
nix profile install github:crocoder-dev/shared-context-engineering
```

Then, from inside a git repository:

```bash
sce setup     # install generated assistant config, hooks, and bash policy
sce doctor    # verify the install is healthy
```

`sce setup` writes the OpenCode and/or Claude config into your repo, installs the required git hooks, and initializes the per-repo Agent Trace database. `sce doctor` is read-only by default; `sce doctor --fix` will repair the issues it knows how to repair (missing or stale hooks, missing canonical DB parent directories) and report the rest for manual follow-up.

## Bash policy

**Stop agents from running commands your repo does not allow.**

Configure `policies.bash` in `.sce/config.json` with built-in presets and/or your own deny rules. Examples of what teams use it for:

- block direct `git commit` so agents go through the SCE commit flow
- prevent package-manager drift (e.g. block `npm`/`pnpm` in a Bun repo)
- enforce any custom repo rule expressible as a command-prefix match

## Agent Trace

SCE writes a local, per-repo audit trail conforming to the [Agent Trace](https://agent-trace.dev/) spec. Each commit links back to the agent conversation, tool calls, and diff that produced it, all stored on your machine.

## Supported integrations

| Feature | OpenCode | Claude Code |
|---|---|---|
| Generated config | ✓ | ✓ |
| Hooks + Bash policy | ✓ | ✓ |
| Conversation + diff trace | ✓ | ✓ |
| Model / session attribution | full | full |
| Shared `context/` | ✓ | ✓ |

OpenCode and Claude Code are first-class. Other agents can read `context/` but don't get generated config, hooks, or Bash policy enforcement.

## Why this exists

AI sped up code generation, not team alignment. Without shared, durable context, every agent session starts cold and every reviewer re-derives the *why* from scratch, what we call **cognitive debt**. SCE is the infrastructure to pay it down.

Read the full argument → [Motivation](https://sce.crocoder.dev/docs/motivation)

## Documentation

- [Documentation site](https://sce.crocoder.dev/)
- [Getting started](https://sce.crocoder.dev/docs/getting-started)
---

Built by [CroCoder](https://www.crocoder.dev/).
