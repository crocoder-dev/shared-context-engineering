# Plan: skip-sce-coauthor-for-sce-author

## Change summary

The commit-msg hook must not add `Co-authored-by: SCE <sce@crocoder.dev>` when the commit author is the SCE identity itself (`sce@crocoder.dev`, the `shared-context-engineering` GitHub account). This exemption applies even when attribution hooks are enabled and the staged-diff AI-overlap preflight reports `Overlap`. A co-author trailer that names the commit author is redundant.

This change extends the existing co-author policy in `cli/src/services/hooks/commit_hooks.rs` (`run_commit_msg_subcommand_in_repo` and `apply_commit_msg_coauthor_policy`). All other gates, trailer placement, dedupe, and idempotency stay unchanged. The change also repairs recorded context that names `cli/src/services/hooks/mod.rs` as the policy location; the code lives in `commit_hooks.rs`.

## Acceptance criteria

- [x] AC1: When the resolved commit author email is `sce@crocoder.dev` (case-insensitive), the commit-msg hook leaves the commit message unchanged, even when attribution hooks are enabled, SCE is not disabled, and AI overlap is present.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml coauthor_policy_tests`
- [x] AC2: When the resolved commit author email is any other address, trailer behavior is identical to the current policy.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml coauthor_policy_tests`
- [x] AC3: When the author is the SCE identity, the hook does not run the staged-diff AI-overlap preflight.
  - Validate: inspect `run_commit_msg_subcommand_in_repo` in `cli/src/services/hooks/commit_hooks.rs`; the author check must short-circuit before `staged_diff_has_ai_overlap` is called.
- [x] AC4: The author is resolved the same way Git resolves it for the commit, so `git commit --author`, `GIT_AUTHOR_EMAIL`, and `user.email` are all honored.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml commit_msg` (includes a repository-backed test that sets `GIT_AUTHOR_EMAIL=sce@crocoder.dev` and asserts no trailer).

### Full validation

- `nix flake check`
- `git diff --check`

### Context sync

- `context/sce/agent-trace-commit-msg-coauthor-policy.md`: add the author-exemption gate and correct the policy and runtime entrypoint locations to `cli/src/services/hooks/commit_hooks.rs`.
- `context/sce/agent-trace-hooks-command-routing.md`: note the author exemption beside the canonical trailer rule.
- `context/glossary.md`: update the `agent trace commit-msg co-author policy` entry with the author exemption and correct file location.

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/services/hooks/commit_hooks.rs`, `cli/src/services/hooks/runtime.rs` (author resolution helper if needed), `cli/src/services/hooks/mod.rs` (identity constant if needed), commit-msg hook tests.
- **Out of scope:** post-commit Agent Trace persistence, `SCE-*` execution trailers, setup prompts, and config schema.
- **Constraints:** Resolve the author with Git (`git var GIT_AUTHOR_IDENT`) from the repository root. Use a typed error or explicit result for resolution failure, not a new `anyhow` surface where a typed result is practical. Keep the policy function pure and testable.
- **Non-goal:** A configurable list of exempt authors. A new config key or environment variable for this exemption.

## Assumptions

- "The author" means the Git commit author, not the committer. The email `sce@crocoder.dev` is the identity match; the GitHub URL identifies the same account and is not matched separately.
- Email comparison is exact after trimming and ASCII case-folding. The author name is ignored.
- If the author cannot be resolved, the hook keeps the current policy behavior. Git itself fails the commit when no author identity exists.
- The exempt email derives from the existing canonical trailer identity, so the two cannot diverge.

## Task stack

- [x] T01: `Skip SCE co-author trailer when the commit author is SCE` (status:done)
  - Task ID: T01
  - Scope: In — resolve the commit author email in the commit-msg runtime; add an author-exemption input to the co-author policy; short-circuit before the AI-overlap preflight; unit tests for exempt, non-exempt, and case-variant emails; a repository-backed commit-msg test with `GIT_AUTHOR_EMAIL=sce@crocoder.dev`. Out — config, setup, post-commit, and any other trailer.
  - Dependencies: none
  - Done when: the policy returns the message unchanged for the SCE author under all other passing gates; non-SCE authors keep current behavior; the preflight is skipped for the SCE author; targeted tests pass.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml coauthor_policy_tests`; `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml commit_msg`.
  - Completed: 2026-11-27
  - Files changed: `cli/src/services/hooks/mod.rs`, `cli/src/services/hooks/runtime.rs`, `cli/src/services/hooks/commit_hooks.rs`, `cli/tests/commit_msg_author_exemption.rs`
  - Result: `SCE_IDENTITY_EMAIL` and `CANONICAL_SCE_COAUTHOR_TRAILER` share one email literal through a `sce_identity_email!` macro in `mod.rs`. `resolve_commit_author_email` in `runtime.rs` runs `git var GIT_AUTHOR_IDENT` from the repository root and returns a typed `CommitAuthorResolutionError` (`GitUnavailable`, `GitFailed`, `MalformedIdent`). `run_commit_msg_subcommand_in_repo` resolves the author after the policy gate passes and skips `staged_diff_has_ai_overlap` for the SCE author. A resolution failure logs debug event `sce.hooks.commit_msg.author_unresolved` and keeps the current policy. `apply_commit_msg_coauthor_policy` takes a new `author_email: Option<&str>` input and returns the message unchanged when the trimmed, ASCII case-folded email equals `sce@crocoder.dev`. The repository-backed integration test runs a real `git commit` through the `sce` binary as the commit-msg hook. It covers `GIT_AUTHOR_EMAIL`, `--author`, and `user.email`. A sandbox `HOME` that stays empty proves the preflight was skipped. For a non-SCE author, the test asserts the preflight's `sce.hooks.commit_msg.ai_overlap_error` log event.
  - Verify outcomes: `coauthor_policy_tests` passed (6 passed); `commit_msg` passed (2 passed in `tests/commit_msg_author_exemption.rs`); `cargo clippy --all-targets -- -D warnings` passed; `cargo fmt` applied; `git diff --check` passed.
  - Deviations: the repository-backed test is a Cargo integration test (`cli/tests/commit_msg_author_exemption.rs`) and not a unit test, so the real Git hook environment exercises author resolution without process-global environment mutation. At the user's request, no explanatory code comments were added.
  - Context impact: localized — the commit-msg co-author policy gains an author-exemption gate (`context/sce/agent-trace-commit-msg-coauthor-policy.md`, `context/sce/agent-trace-hooks-command-routing.md`, `context/glossary.md`), and the recorded policy location must be corrected to `commit_hooks.rs`.
  - Context synchronization: synced

## Open questions

None. The request names the identity and the expected behavior, and the current code has no author check.

## Validation Report

**Status:** validated  
**Date:** 2026-11-27

### Commands run

- `nix flake check` -> exit 0 (all 5 flake checks passed: build, clippy, fmt, tests, setup-schema-upgrade-check)
- `git diff --check` -> exit 0 (no whitespace errors)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml coauthor_policy_tests` -> exit 0 (6 passed)
- `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml commit_msg` -> exit 0 (2 passed in `tests/commit_msg_author_exemption.rs`)

### Success-criteria verification

- [x] AC1: SCE author leaves the message unchanged under all other passing gates -> `coauthor_policy_tests::sce_author_leaves_the_message_unchanged` passed; `commit_msg_hook_skips_coauthor_and_preflight_for_sce_author` passed
- [x] AC2: Other authors keep current trailer behavior -> `coauthor_policy_tests::other_authors_keep_the_current_policy` plus existing trailer-placement tests passed
- [x] AC3: Preflight is skipped for the SCE author -> inspected `run_commit_msg_subcommand_in_repo` (`cli/src/services/hooks/commit_hooks.rs:76-89`); `staged_diff_has_ai_overlap` is only called when `!author_is_sce_identity(author_email)`
- [x] AC4: Author resolved as Git resolves it (`--author`, `GIT_AUTHOR_EMAIL`, `user.email`) -> repository-backed `commit_msg_hook_skips_coauthor_and_preflight_for_sce_author` passed via real `git commit` using `git var GIT_AUTHOR_IDENT`

### Failed checks and follow-ups

- None.

### Residual risks

- `cli/tests/commit_msg_author_exemption.rs` is untracked, so the git-source-filtered `nix flake check` did not include it; it was run directly via cargo and passed. It must be added to Git to be covered by flake checks.
