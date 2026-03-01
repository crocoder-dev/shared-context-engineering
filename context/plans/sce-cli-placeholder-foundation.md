# Plan: sce-cli-placeholder-foundation

## 1) Change summary
Create a new Rust CLI named `sce` under `cli/` as a placeholder foundation with minimal dependencies, using `tokio` and the `turso` crate for local database capability. The initial slice focuses on deterministic scaffolding and extension points for future features: repository setup, MCP tooling, git-hook awareness for SCE-generated code regions, and cloud sync.

## 2) Success criteria
- A new compilable Rust CLI crate exists at `cli/` and exposes an `sce` executable.
- Dependency surface remains minimal and explicit: `anyhow`, `tokio`, `turso`, and `lexopt` (confirmed interpretation of "lexpot").
- CLI includes placeholder command flow and help text that clearly marks future subcommands without claiming implemented behavior.
- Local Turso integration uses `turso::Builder::new_local(...)` with async execution under `tokio`.
- Architecture stubs (traits/modules/contracts) exist for future MCP tooling, git hook listener integration, generated-region tracking, and cloud sync.
- Basic verification passes (`cargo check`, `cargo test` or equivalent smoke tests) for the new CLI crate.
- Context updates for any new operational contracts are captured and synchronized after implementation.

## 3) Constraints and non-goals
- In scope: plan and future implementation guidance for `cli/` placeholder scaffolding only.
- In scope: local Turso usage through the `turso` crate (not `libsql`) and async runtime via `tokio`.
- In scope: minimal-dependency posture for an early-stage CLI.
- Out of scope: full production implementation of setup automation, full MCP feature set, full git-hook daemon/listener behavior, and real cloud sync.
- Out of scope: remote Turso/cloud authentication and production secret management.
- Non-goal: shipping stable UX/flags for all future commands in this placeholder phase.

## Assumptions
- "lexpot" is interpreted as the Rust crate `lexopt` for minimal argument parsing.
- Turso API usage follows the crate pattern already provided (`Builder::new_local`, `connect`, async `execute/query`) and stays local-only in this phase.
- If repository-level Rust workspace wiring is needed, the implementation will follow existing repo conventions rather than introducing broad cross-repo refactors.

## 4) Task stack (T01..T07)
- [x] T01: Define CLI foundation boundaries and crate layout (status:done)
  - Task ID: T01
  - Goal: Finalize the `cli/` crate structure, command surface contract, and module boundaries for placeholder-first delivery.
  - Boundaries (in/out of scope):
    - In: crate skeleton, `main.rs` entrypoint contract, module map for commands/services, and explicit placeholder markers.
    - Out: feature-complete command implementations.
  - Done when:
    - `cli/` layout is documented in-code and reflects near-term extensibility.
    - Placeholder vs implemented surfaces are explicitly distinguishable.
  - Verification notes (commands or checks):
    - Confirm file/module structure and compile-time entrypoint wiring.

- [ ] T02: Bootstrap Rust crate with minimal dependency contract (status:todo)
  - Task ID: T02
  - Goal: Create crate metadata and dependency configuration with only required early-phase crates.
  - Boundaries (in/out of scope):
    - In: `Cargo.toml` setup for `anyhow`, `tokio`, `turso`, and `lexopt`; minimal profiles/features needed for placeholder behavior.
    - Out: adding convenience frameworks (for example `clap`) unless blockers emerge and are explicitly approved.
  - Done when:
    - Crate compiles with the agreed dependency set.
    - Dependency rationale is clear from manifest structure and code usage.
  - Verification notes (commands or checks):
    - `cargo check -p sce` (or crate-local `cargo check`) passes.

- [ ] T03: Implement placeholder command loop and error model (status:todo)
  - Task ID: T03
  - Goal: Add minimal CLI parsing and dispatch scaffolding using `lexopt` with consistent `anyhow`-based error handling.
  - Boundaries (in/out of scope):
    - In: top-level command parser, `--help` output, placeholder subcommands (`setup`, `mcp`, `hooks`, `sync`) returning intentional TODO messaging.
    - Out: deep option matrices and backward-compatibility guarantees.
  - Done when:
    - CLI can parse and route to placeholder handlers without panics.
    - Error exits and user-facing messages are deterministic and actionable.
  - Verification notes (commands or checks):
    - Run help and representative placeholder invocations.
    - Add parser-focused unit tests for key command routes.

- [ ] T04: Add local Turso integration adapter with tokio runtime (status:todo)
  - Task ID: T04
  - Goal: Introduce a small data-layer module that initializes a local Turso database using `Builder::new_local(...)` and validates connectivity with a smoke operation.
  - Boundaries (in/out of scope):
    - In: local-only path/in-memory modes, async DB bootstrap utility, and a basic query/execute smoke path.
    - Out: remote Turso endpoints, auth tokens, replication, and production migration framework.
  - Done when:
    - Adapter can create/open a local DB and run at least one successful SQL operation in async flow.
    - CLI placeholder command(s) can invoke the adapter without exposing low-level DB details.
  - Verification notes (commands or checks):
    - Add async tests around local DB init and simple round-trip query.
    - Confirm behavior against Turso crate API from `/tursodatabase/turso` docs.

- [ ] T05: Scaffold future feature contracts for setup, MCP, hooks, and cloud sync (status:todo)
  - Task ID: T05
  - Goal: Define stable interfaces and internal module seams for future capabilities without implementing full behavior.
  - Boundaries (in/out of scope):
    - In: trait/interface definitions, placeholder service structs, event model placeholders for git hooks and generated-region tracking, and cloud sync abstraction points.
    - Out: actual MCP transport implementation, live hook registration daemon, or real cloud API calls.
  - Done when:
    - Future capabilities have explicit extension points and ownership boundaries in code.
    - No placeholder path implies production readiness.
  - Verification notes (commands or checks):
    - Compile-time interface checks.
    - Basic tests for placeholder service wiring where applicable.

- [ ] T06: Add documentation and onboarding notes for placeholder CLI (status:todo)
  - Task ID: T06
  - Goal: Document current command behavior, near-term roadmap, and safe usage limitations.
  - Boundaries (in/out of scope):
    - In: crate-local README/usage docs and concise repo-level pointer if needed.
    - Out: long-form product documentation for unimplemented features.
  - Done when:
    - Developers can run the placeholder CLI and understand what is intentionally not implemented.
    - Future work items are mapped to module contracts created in T05.
  - Verification notes (commands or checks):
    - Manual docs sanity pass aligned to actual command output.

- [ ] T07: Validation and cleanup (status:todo)
  - Task ID: T07
  - Goal: Execute final quality gates, remove temporary scaffolding artifacts, and synchronize context to reflect the post-implementation current state.
  - Boundaries (in/out of scope):
    - In: compile/tests, lint/format checks used by repo norms, context updates (`context/overview.md`, `context/architecture.md`, `context/glossary.md`, and plan status updates as needed).
    - Out: net-new feature additions beyond accepted scope.
  - Done when:
    - All success criteria have verification evidence.
    - Temporary/debug artifacts are removed or intentionally retained with rationale.
    - Context reflects the implemented current state and no known code-context drift remains for this change.
  - Verification notes (commands or checks):
    - `cargo check` and `cargo test` for the new crate/workspace path.
    - Any repo-required format/lint checks relevant to Rust changes.
    - Manual context sync verification against implemented code paths.

## 5) Open questions
- None.
