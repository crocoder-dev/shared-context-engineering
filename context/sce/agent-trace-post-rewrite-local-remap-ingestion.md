# Agent Trace Post-Rewrite Local Remap Ingestion (T08)

## Current status
- This contract is no longer active in runtime.
- The current `cli/src/services/hooks.rs` keeps `sce hooks post-rewrite` as a deterministic no-op.

## Status
- Plan: `agent-trace-attribution-no-git-wrapper`
- Task: `T08`
- Scope: local `post-rewrite` ingestion pipeline only (no hosted webhook processing)

## Implemented surface
- Code: `cli/src/services/hooks.rs`
- Primary entrypoint: `finalize_post_rewrite_remap`
- Hook intent: consume local git `post-rewrite` input (`<old_sha> <new_sha>` pairs) and emit deterministic remap-ingestion requests.

## Runtime gating

`finalize_post_rewrite_remap` returns `NoOp` and performs no ingestion when any of these guards apply:

- `sce_disabled = true`
- `attribution_hooks_enabled = false`
- `trace_side_effects_enabled = false`
- `cli_available = false`
- `is_bare_repo = true`

## Pair parsing contract

- Input is a newline-delimited payload where each non-empty line must contain exactly two whitespace-separated fields: `<old_sha> <new_sha>`.
- Empty lines are ignored.
- Self-mapping lines (`old_sha == new_sha`) are ignored as no-op rewrites.
- Any non-empty malformed line fails the call with an error; no partial best-effort parsing for that invocation.

## Rewrite-method normalization

- Hook argument values are normalized to lowercase.
- Recognized values map to typed methods:
  - `amend` -> `RewriteMethod::Amend`
  - `rebase` -> `RewriteMethod::Rebase`
- All other values are preserved as lowercase in `RewriteMethod::Other(String)`.

## Idempotency and dispatch

- For each parsed pair, the ingestion request derives one deterministic key:
  - `post-rewrite:<method>:<old_sha>:<new_sha>`
- The method token uses normalized labels (`amend`, `rebase`, or lowercase passthrough).
- Requests are dispatched through `RewriteRemapIngestion::ingest`.
- The ingestion response is interpreted as:
  - `true`: pair accepted as a new ingestion
  - `false`: pair skipped as replay/duplicate
- Finalization returns aggregate counters: total pairs, ingested pairs, and skipped pairs.

## Current boundaries

- In scope: local hook-side normalization, strict parsing, deterministic per-pair replay keys, and ingestion dispatch seam.
- Out of scope: rewrite trace transformation semantics (`T09`), hosted intake (`T12`), and mapping engine heuristics (`T13`).

## Verification evidence

- `nix flake check`

## Tests added

- No-op behavior when SCE is disabled, attribution hooks are disabled, or attribution-only mode is active.
- Amend-pair ingestion with deterministic idempotency-key derivation.
- Rebase duplicate replay behavior (second identical pair skipped).
- Strict malformed-line rejection (`<old_sha> <new_sha>` required).
