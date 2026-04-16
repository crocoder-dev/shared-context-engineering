# Plan: OpenCode agent-trace plugin generation

## Change Summary

Add canonical Pkl-owned registration and generation wiring for the new OpenCode Agent Trace plugin so both manual and automated OpenCode profiles emit the plugin artifact and register it in generated `opencode.json` manifests.

User-confirmed decisions:

- Scope is limited to Pkl + generated OpenCode outputs.
- Canonical plugin registration is `id = "sce-agent-trace"` with path `"./plugins/sce-agent-trace.ts"`.
- Success includes generation for both profiles plus generated/parity validation.

## Success Criteria

1. Canonical OpenCode plugin registration sources include the new `sce-agent-trace` entry with path `./plugins/sce-agent-trace.ts`.
2. Generated OpenCode manifests for both profiles include the new plugin registration (alongside existing SCE-managed plugin registrations).
3. The plugin source is generated into both profile plugin directories:
   - `config/.opencode/plugins/sce-agent-trace.ts`
   - `config/automated/.opencode/plugins/sce-agent-trace.ts`
4. Generation/parity checks confirm no drift after updates (`nix run .#pkl-check-generated`), and repository validation remains green (`nix flake check`).
5. Context documentation that defines the OpenCode plugin-registration contract is synced to current code truth.

## Constraints and Non-Goals

- In scope: Pkl source ownership, renderer/generation wiring, generated OpenCode outputs, and context updates tied to this contract.
- Out of scope: introducing new runtime behavior requirements for Agent Trace plugin internals beyond generating/packaging the current source file.
- Do not hand-edit generated outputs as a source of truth; implement through canonical `config/pkl/**` and library source files.
- Keep task slicing atomic: each executable task must be one coherent commit unit.

## Task Stack

- [x] T01: `Add canonical OpenCode registration for agent-trace plugin` (status:done)
  - Task ID: T01
  - Goal: Extend canonical plugin-registration sources so the shared OpenCode plugin list includes `sce-agent-trace` with path `./plugins/sce-agent-trace.ts`.
  - Boundaries (in/out of scope): In - `config/pkl/base/opencode.pkl` plugin registration model data and `config/pkl/renderers/common.pkl` shared registration list wiring. Out - generation output files, plugin source-file emission, or context docs.
  - Done when: canonical registration data defines `sce-agent-trace` and shared renderer inputs include it deterministically for both OpenCode profiles.
  - Verification notes (commands or checks): inspect updated Pkl sources; run `nix run .#pkl-check-generated` after regeneration-focused edits to confirm deterministic output expectations.
  - **Status:** done
  - **Completed:** 2026-04-16
  - **Files changed:** `config/pkl/base/opencode.pkl`, `config/pkl/renderers/common.pkl`
  - **Evidence:** `nix run .#pkl-check-generated` passes; canonical OpenCode registration now includes `sce-agent-trace` with path `./plugins/sce-agent-trace.ts`; shared renderer plugin list includes both `sce-bash-policy` and `sce-agent-trace` deterministically.

- [x] T02: `Generate agent-trace plugin file for manual and automated OpenCode profiles` (status:done)
  - Task ID: T02
  - Goal: Wire generator inputs so `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts` is emitted to both generated OpenCode plugin paths.
  - Boundaries (in/out of scope): In - `config/pkl/generate.pkl` source reads and file-output mappings for both manual + automated profiles. Out - unrelated plugin/runtime generation, Claude outputs, or changes to non-Agent-Trace plugin paths.
  - Done when: generation mapping writes `sce-agent-trace.ts` into both `config/.opencode/plugins/` and `config/automated/.opencode/plugins/` from the canonical library source.
  - Verification notes (commands or checks): regenerate outputs and confirm target plugin files are present and content-aligned with the source file; run `nix run .#pkl-check-generated`.
  - **Status:** done
  - **Completed:** 2026-04-16
  - **Files changed:** `config/pkl/generate.pkl`, `config/.opencode/plugins/sce-agent-trace.ts`, `config/automated/.opencode/plugins/sce-agent-trace.ts`
  - **Evidence:** `nix run .#pkl-check-generated` passes; `nix develop -c pkl eval -m . config/pkl/generate.pkl` emitted both generated plugin files from `config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts`; generated plugin files are content-aligned with the canonical library source.

- [x] T03: `Ensure both generated opencode manifests register the new plugin` (status:done)
  - Task ID: T03
  - Goal: Validate and land generated-manifest registration behavior for both manual and automated OpenCode `opencode.json` outputs.
  - Boundaries (in/out of scope): In - renderer-driven manifest output changes resulting from canonical registration updates, plus generated `opencode.json` artifacts for manual/automated profiles. Out - non-OpenCode targets and unrelated manifest fields.
  - Done when: both generated `opencode.json` files include `./plugins/sce-agent-trace.ts` in the `plugin` array with deterministic ordering per canonical list.
  - Verification notes (commands or checks): inspect `config/.opencode/opencode.json` and `config/automated/.opencode/opencode.json`; run `nix run .#pkl-check-generated`.
  - **Status:** done
  - **Completed:** 2026-04-16
  - **Files changed:** `config/.opencode/opencode.json`, `config/automated/.opencode/opencode.json`
  - **Evidence:** both generated manifests include `./plugins/sce-agent-trace.ts` after `./plugins/sce-bash-policy.ts`; `nix run .#pkl-check-generated` passes; no additional source edits were required because T01/T02 canonical wiring already drove the manifest output.

- [x] T04: `Validation, cleanup, and context sync for plugin-registration contract` (status:done)
  - Task ID: T04
  - Goal: Run full validation and sync context files that document generated OpenCode plugin-registration behavior.
  - Boundaries (in/out of scope): In - final validation commands, plan evidence updates, and context updates for `context/sce/generated-opencode-plugin-registration.md` plus any required root context/glossary references if contract semantics changed. Out - new functional behavior outside this plan.
  - Done when: validation commands pass, no temporary scaffolding remains, and context accurately reflects current generated OpenCode plugin-registration state including the new agent-trace plugin.
  - Verification notes (commands or checks): `nix run .#pkl-check-generated`; `nix flake check`; verify/update `context/overview.md`, `context/glossary.md`, and `context/sce/generated-opencode-plugin-registration.md` as needed.
  - **Status:** done
  - **Completed:** 2026-04-16
  - **Files changed:** `flake.nix`, `context/plans/opencode-agent-trace-plugin-generation.md`, `context/sce/generated-opencode-plugin-registration.md`, `context/overview.md`, `context/architecture.md`, `context/glossary.md`, `context/context-map.md`
  - **Evidence:** `nix run .#pkl-check-generated` passes; `nix flake check` passes after tracking `config/lib/agent-trace-plugin/` so flake evaluation includes the canonical Agent Trace plugin source; shared/root context and `context/sce/generated-opencode-plugin-registration.md` reflect the current generated OpenCode plugin-registration contract for `sce-bash-policy` and `sce-agent-trace`.

## Open Questions

None.

## Validation Report

### Commands run

- `nix run .#pkl-check-generated` -> exit 0 (`Generated outputs are up to date.`)
- `nix flake check` -> exit 0 (`all checks passed!`)

### Temporary scaffolding

- None identified or removed in this final task.

### Success-criteria verification

- [x] Canonical OpenCode plugin registration sources include `sce-agent-trace` with path `./plugins/sce-agent-trace.ts` -> confirmed in `config/pkl/base/opencode.pkl` and `config/pkl/renderers/common.pkl` as recorded by T01 evidence.
- [x] Generated OpenCode manifests for both profiles include the new plugin registration -> confirmed in `config/.opencode/opencode.json` and `config/automated/.opencode/opencode.json` with `./plugins/sce-agent-trace.ts` present after `./plugins/sce-bash-policy.ts`.
- [x] The plugin source is generated into both profile plugin directories -> confirmed in `config/.opencode/plugins/sce-agent-trace.ts` and `config/automated/.opencode/plugins/sce-agent-trace.ts` as recorded by T02 evidence.
- [x] Generation/parity checks confirm no drift and repository validation remains green -> confirmed by `nix run .#pkl-check-generated` exit 0 and `nix flake check` exit 0.
- [x] Context documentation that defines the OpenCode plugin-registration contract is synced to current code truth -> confirmed in `context/sce/generated-opencode-plugin-registration.md` plus aligned root references in `context/overview.md`, `context/architecture.md`, `context/glossary.md`, and `context/context-map.md`.

### Failed checks and follow-ups

- Initial `nix flake check` attempts failed while `config/lib/agent-trace-plugin/` was still untracked, because Nix flake evaluation cannot see untracked files. After user-approved index update for that canonical source path, `nix flake check` passed without further code changes.

### Residual risks

- None identified for this plan scope.
