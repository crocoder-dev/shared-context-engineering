# Shared Context Engineering (SCE)

[![crates.io](https://img.shields.io/crates/v/shared-context-engineering?logo=rust)](https://crates.io/crates/shared-context-engineering)
[![npm](https://img.shields.io/npm/v/%40crocoder-dev%2Fsce?logo=npm)](https://www.npmjs.com/package/@crocoder-dev/sce)
[![Nix CI](https://github.com/crocoder-dev/shared-context-engineering/actions/workflows/pr-ci.yml/badge.svg?branch=main)](https://github.com/crocoder-dev/shared-context-engineering/actions/workflows/pr-ci.yml)

**AI made code generation fast. Team alignment didn't keep up.**

SCE treats the *why* behind your code, architecture, decisions, constraints, as a versioned, shared artifact that both your team and your AI agents work from.

- a repo-owned `context/` directory holding the architecture, decisions, and constraints AI agents otherwise re-derive every session
- generated configs that make OpenCode, Claude Code, and Pi actually read it
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

`sce setup` writes OpenCode, Claude Code, and/or Pi config into your repo, installs the required git hooks, and initializes the per-repo Agent Trace database. Use `sce setup --pi` for Pi only, or `sce setup --all` for OpenCode + Claude Code + Pi. `sce doctor` is read-only by default; `sce doctor --fix` will repair the issues it knows how to repair (missing or stale hooks, missing canonical DB parent directories) and report the rest for manual follow-up.

## Upgrading existing installations: managed telemetry

When upgrading from a version that did not create `telemetry-ready.json`, run `sce auth login` again if you use managed OpenTelemetry (OTLP) export, even if you are already signed in. An existing `auth.db` and a previously valid telemetry credential are not enough: without the readiness file, managed trace and log export stays disabled while SCE and its local Agent Trace capture continue working. This is intentional so unauthenticated hook processes do not open the encrypted auth DB or access the OS credential store.

After updating the CLI through your installation channel, run these commands in an interactive terminal inside the repository:

```bash
sce auth login
sce doctor
sce doctor --format json
```

Login attempts to issue and save a telemetry credential and create the readiness file, including when it reuses or renews an existing login. Authentication can succeed while telemetry provisioning fails. If you see ``Telemetry credential provisioning was unavailable. Retry with `sce auth login`.``, retry login once the service and your OS credential store are available, then run doctor again. For JSON login output (`sce auth login --format json`), check `telemetry_provisioning.status`: `issued` means provisioning succeeded; `unavailable` means it did not.

In doctor's `Environment` section, check the **Managed telemetry** row, or the top-level `managed_telemetry` object in JSON:

| Doctor output | JSON `managed_telemetry.state` | What to do |
|---|---|---|
| `[PASS] Managed telemetry` | `healthy` | The stored credential is usable for managed export. |
| `[PASS] Managed telemetry (not configured)` | `not_configured` | Managed export is off. If you expect telemetry after upgrading, run `sce auth login` again. This state produces no warning. |
| `[WARN] Managed telemetry` | `disabled` | Export is off. Follow the reported `reason` and remediation; `next_action: auth_login` calls for an interactive login followed by another doctor check. |

An overall healthy doctor result or successful login alone does not confirm telemetry is enabled. Doctor checks credential health without starting an exporter or contacting the collector, so `healthy` does not prove delivery. `sce doctor --fix` does not provision telemetry or repair readiness. Let login manage `telemetry-ready.json`; do not create it manually or delete the auth DB or encryption key to work around this check. Installations that do not use managed export can remain `not_configured`.

Doctor's telemetry diagnostics contain no credentials. Use them to diagnose disabled export instead of running `sce auth telemetry-headers` in a terminal: that hidden Claude Code helper prints a bearer credential on success. If inspecting local logs, `sce.telemetry.fallback` with reason `not_configured` indicates skipped managed export, but it is a debug event and may be absent at the default log level.

Claude Code's native OTLP export also needs its generated settings and repository enrollment. To enable it, run `sce setup --claude` (or `sce setup --all`) and then `sce auth login` inside each intended repository. Setup alone does not authenticate or enroll it, and doctor's `healthy` state does not verify Claude settings or repository enrollment. Login enrolls an unbound repository without moving an existing binding to another workspace. See the [telemetry reference](context/sce/telemetry-credential.md) for lifecycle and security details.

## Bash policy

**Stop agents from running commands your repo does not allow.**

Configure `policies.bash` in `.sce/config.json` with built-in presets and/or your own deny rules. Examples of what teams use it for:

- block direct `git commit` so agents go through the SCE commit flow
- prevent package-manager drift (e.g. block `npm`/`pnpm` in a Bun repo)
- enforce any custom repo rule expressible as a command-prefix match

## Agent Trace

SCE writes a local, per-repo audit trail conforming to the [Agent Trace](https://agent-trace.dev/) spec. Each commit links back to the agent conversation, tool calls, and diff that produced it, all stored on your machine.

## Supported integrations

| Feature | OpenCode | Claude Code | Pi |
|---|---|---|---|
| Generated config | ✓ | ✓ | ✓ |
| Hooks + Bash policy | ✓ | ✓ | ✓ |
| Conversation + diff trace | ✓ | ✓ | ✓ |
| Model / session attribution | full | full | full |
| Shared `context/` | ✓ | ✓ | ✓ |

## Why this exists

AI sped up code generation, not team alignment. Without shared, durable context, every agent session starts cold and every reviewer re-derives the *why* from scratch, what we call **cognitive debt**. SCE is the infrastructure to pay it down.

Read the full argument → [Motivation](https://sce.crocoder.dev/docs/motivation)

## Documentation

- [Documentation site](https://sce.crocoder.dev/)
- [Getting started](https://sce.crocoder.dev/docs/getting-started)
---

Built by [CroCoder](https://www.crocoder.dev/).
