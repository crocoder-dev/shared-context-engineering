# Change Summary

Roll out `owo-colors` across the most user-facing text-mode CLI surfaces, and use `comfy-table` selectively where the output is naturally tabular. Keep JSON, completion-script, and MCP stdio outputs unchanged so machine-readable contracts and pipe safety remain stable.

# Success Criteria

- Human-facing text output for the highest-traffic commands has clearer visual hierarchy using `owo-colors`.
- Selective `comfy-table` usage is limited to outputs that are materially easier to scan as rows/columns.
- JSON output paths remain byte-for-byte compatible with the current contract.
- Non-interactive and machine-oriented outputs stay unstyled or are gated to TTY-safe behavior.
- Existing tests are updated or extended to cover the new text rendering behavior without weakening JSON/output-contract coverage.

# Constraints and Non-Goals

- In scope: text-mode output for root help, app-level error diagnostics, setup summaries, doctor text report, auth text flows, and one compact metadata/status surface if it improves consistency.
- In scope: introducing a small shared styling helper/policy so color/table use stays deterministic and reusable.
- Out of scope: JSON payload changes, completion script formatting, MCP stdio server output, and broad redesign of low-level hook/runtime/internal log messages.
- Out of scope: adding color to observability/file-sink logs unless a follow-up task explicitly expands the scope.
- `comfy-table` should be used only where the rendered content is structurally tabular; prose-style sections should remain plain text with color accents.
- Color usage must preserve readable output in non-TTY or `NO_COLOR` scenarios and must not break stable automation expectations for piped output.

# Task Stack

- [ ] T01: `Add shared text styling foundation` (status:todo)
  - Task ID: T01
  - Goal: Add the `owo-colors` and `comfy-table` dependencies plus a small shared CLI styling layer that centralizes color enablement and table rendering rules for text-mode output.
  - Boundaries (in/out of scope): In - dependency wiring, TTY/`NO_COLOR` policy, shared helper API, unit coverage for style gating. Out - command-specific output rewrites.
  - Done when: the CLI has a reusable styling module that can render plain or styled text deterministically, table helpers are available for tabular surfaces, and non-JSON paths can opt in without duplicating policy logic.
  - Verification notes (commands or checks): `nix flake check`; targeted Rust tests covering style enablement/disablement and any shared rendering helpers.

- [ ] T02: `Refresh help and error presentation` (status:todo)
  - Task ID: T02
  - Goal: Apply `owo-colors` to the root help and subcommand-help experiences and to top-level stderr diagnostics so the highest-frequency discovery/error paths gain clearer emphasis.
  - Boundaries (in/out of scope): In - `cli/src/command_surface.rs`, help rendering paths in `cli/src/cli_schema.rs` / `cli/src/app.rs`, and app-level `Error [CODE]: ...` formatting. Out - command-specific success/status bodies.
  - Done when: help output has clearer headings/command emphasis, error diagnostics use restrained semantic color, and plain-text fallback remains intact for non-colored environments.
  - Verification notes (commands or checks): targeted Rust tests for help/error rendering branches; `nix flake check`.

- [ ] T03: `Style setup and auth text flows` (status:todo)
  - Task ID: T03
  - Goal: Update setup and auth text-mode output to use `owo-colors` consistently, improving readability of statuses, labels, and next-step guidance without changing JSON behavior.
  - Boundaries (in/out of scope): In - setup install/hook summaries and auth login/renew/logout/status text rendering. Out - auth JSON payloads, network behavior, token storage behavior, and interactive device-flow mechanics beyond text presentation.
  - Done when: setup/auth text outputs highlight success states, labels, and key values cleanly; browser/code prompts remain obvious; and JSON branches are unchanged.
  - Verification notes (commands or checks): targeted Rust tests for setup/auth text renderers; `nix flake check`.

- [ ] T04: `Apply selective tables to doctor and help listings` (status:todo)
  - Task ID: T04
  - Goal: Introduce `comfy-table` only where it clearly improves scanability, starting with the command listing in root help and the row-oriented sections of the doctor text report.
  - Boundaries (in/out of scope): In - converting naturally tabular doctor/help sections to compact tables and pairing them with restrained `owo-colors`. Out - converting prose sections or prompt-style output into tables.
  - Done when: help command listings and doctor sections such as hooks/databases/problems render more cleanly as tables in text mode, while narrative/status lines remain non-tabular and readable.
  - Verification notes (commands or checks): targeted Rust tests for doctor/help text rendering; manual spot-checks of narrow-width readability if practical; `nix flake check`.

- [ ] T05: `Polish remaining top-tier text surfaces` (status:todo)
  - Task ID: T05
  - Goal: Bring one-line or metadata-heavy text outputs into the same presentation system for consistency across the most user-facing commands.
  - Boundaries (in/out of scope): In - config text output, trace prompt text headers, version text output, and sync placeholder text if they benefit from lightweight styling. Out - completion scripts, MCP stdio responses, and low-level hooks/observability logs.
  - Done when: the selected remaining top-tier text surfaces use the shared styling conventions consistently, and any surface that proves awkward for styling is explicitly left plain by choice rather than drift.
  - Verification notes (commands or checks): targeted Rust tests for each touched renderer; `nix flake check`.

- [ ] T06: `Run validation and cleanup` (status:todo)
  - Task ID: T06
  - Goal: Validate the rollout end-to-end, remove any temporary rendering scaffolding, and confirm current-state context/doc needs before closing the change.
  - Boundaries (in/out of scope): In - full repo verification, test cleanup, and context sync verification for any durable CLI contract changes. Out - new feature additions beyond the styling rollout.
  - Done when: all relevant checks pass, touched text outputs are covered by appropriate tests, temporary/debug scaffolding is removed, and any required `context/` updates are identified or completed.
  - Verification notes (commands or checks): `nix flake check`; any narrower Rust test commands added during implementation; verify whether CLI/context files need sync for durable output-contract changes.

# Open Questions

- None at plan time; the rollout is scoped to the most user-facing text endpoints already identified: help, app-level errors, setup, doctor, auth, and selective consistency passes on remaining top-tier text surfaces.
