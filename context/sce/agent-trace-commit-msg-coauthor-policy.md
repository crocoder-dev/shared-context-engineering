# Agent Trace commit-msg co-author policy

## Status
- Plan: `agent-trace-attribution-no-git-wrapper`
- Task: `T05`
- Implementation state: done

## Canonical contract
- Policy entrypoint: `cli/src/services/hooks.rs` -> `apply_commit_msg_coauthor_policy`.
- Canonical trailer string: `Co-authored-by: SCE <sce@crocoder.dev>`.
- Runtime gating conditions:
  - `sce_disabled = false`
  - `sce_coauthor_enabled = true`
  - `has_staged_sce_attribution = true`
- When all gate conditions pass, output commit message MUST contain exactly one canonical SCE trailer.
- When any gate condition fails, commit message is returned unchanged.

## Behavior details
- Canonical trailer dedupe removes duplicate canonical lines before final insertion.
- Trailer insertion is idempotent: applying the policy repeatedly yields the same message.
- Existing trailing newline is preserved when present.
- Human author/committer identity is not rewritten; only commit message trailer content is affected.

## Verification evidence
- `cargo fmt --manifest-path cli/Cargo.toml -- --check`
- `cargo test --manifest-path cli/Cargo.toml commit_msg_policy`
- `cargo build --manifest-path cli/Cargo.toml`
