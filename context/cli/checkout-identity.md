# Checkout Identity Service (removed)

The checkout identity service (`cli/src/services/checkout/`: `resolve_git_dir`, `read_checkout_id`, `get_or_create_checkout_id`) was removed by the `remove-checkout-id` plan. SCE no longer creates or reads `<git-dir>/sce/checkout-id`; there is no per-clone/worktree identity anywhere in the current code, and `agent_trace_storage::ResolvedAgentTraceStorage` no longer carries a `checkout_id` field. `sce setup` and `sce doctor` (text and JSON) no longer mention checkout identity.

Repository-scoped Agent Trace persistence never depended on checkout identity for correctness: it was diagnostic metadata only, never stored on Agent Trace rows and never used to select the active DB (see `context/cli/agent-trace-storage.md`). Any pre-existing `<git-dir>/sce/checkout-id` file left on disk from before this removal is inert and untouched by SCE, the same convention already applied to legacy `agent-trace-<checkout-id>.db` files (see `context/sce/agent-trace-db.md`).

See also: `context/glossary.md` (`checkout identity` entry), `context/cli/agent-trace-storage.md`, `context/sce/agent-trace-db.md`.
