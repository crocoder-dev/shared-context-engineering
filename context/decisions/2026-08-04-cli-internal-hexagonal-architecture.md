# Decision: Adopt an internal hexagonal architecture for the sce CLI

Date: 2026-08-04
Status: Accepted
Plan: `context/plans/cli-hexagonal-architecture-skeleton.md`
Task: T01, T02, T03, T04, T05

## Context

The `sce` CLI (`cli/`) is one Cargo package whose command, service, and
runtime logic all live under `cli/src/services/**`, `app.rs`,
`cli_schema.rs`, and `command_surface.rs`. As the CLI grows, there was no
enforced boundary preventing future domain or use-case logic from directly
coupling to infrastructure concerns (CLI parsing, filesystem, process,
environment, database, HTTP). Introducing such a boundary after the fact,
once more code exists, would be far more expensive than establishing it
before further growth.

## Decision

The CLI adopts hexagonal architecture as a permanent, enforced internal
module-boundary and dependency-direction discipline within its single Cargo
package: `cli/src/domain/` (pure business types/rules) and
`cli/src/application/` (`error.rs`, `ports/`, `use_cases/`) must never depend
on `crate::adapters`, `crate::composition`, `crate::services`, or
infrastructure crates/modules (`clap`, `turso`, `reqwest`, `inquire`,
`keyring_core`, `std::fs`, `std::env`, `std::process`); `cli/src/adapters/`
(`inbound/`, `outbound/`) implement application-owned ports and may
transitionally depend on `crate::services`; `cli/src/composition.rs` is the
sole composition root `main.rs` calls, currently delegating to the legacy
`app::run`. `cli/src/services/**` remains a temporary compatibility
namespace holding all current runtime behavior, to be migrated into the new
layers through future vertical slices, one command/capability at a time.

## Rationale

A deterministic, network-free shell script
(`scripts/check-cli-architecture.sh`) can mechanically enforce the
domain/application restriction without new crate dependencies, dynamic
dispatch, or a general-purpose rule engine, and wiring it into
`nix flake check` makes the restriction self-enforcing from day one rather
than relying on review discipline. Establishing the module skeleton and the
permanent restriction now, while `services/**` is left completely untouched
and behavior-identical, decouples the boundary decision from any migration
risk.

## Alternatives considered

- **Split the CLI into multiple crates** — rejected; the plan explicitly
  keeps the CLI a single Cargo package, since crate-count is orthogonal to
  achieving dependency-direction discipline and would add build/tooling
  overhead.
- **Migrate `services/**` logic into the new layers immediately (big-bang
  rewrite)** — rejected as materially riskier than an incremental
  vertical-slice migration; deferred to future plans.
- **A general-purpose architecture-rule engine** — rejected as
  disproportionate; a deterministic script covering exactly the permanent
  `domain`/`application` restrictions is sufficient and simpler to audit.

## Compatibility and risks

- No behavior, CLI output, exit codes, or command surface changes: this
  phase adds only doc-comment-only modules and a thin `composition::run`
  delegator to `app::run`.
- Risk: the architecture check only scans `domain`/`application`, not
  `adapters`/`composition`, which may transitionally depend on `services`;
  this is an intentional, narrow scope, not a gap to be closed reactively.
- Migration risk (moving logic out of `services/**`) is deferred entirely to
  future plans and is not taken on by this decision.

## Guardrails

- `domain` and `application` must never depend on `crate::services`, under
  any circumstance, permanently — this is not a transitional-phase-only
  rule.
- The check enforces only the domain/application restrictions; it does not
  attempt to constrain `adapters` or `composition`.
- No new crate dependencies, no dynamic dispatch, no boxed service
  registries were introduced to achieve this.

## Consequences

- Future CLI plans have an existing, CI-enforced module skeleton and
  dependency rule to build vertical-slice migrations against, instead of
  needing to invent one per plan.
- `scripts/check-cli-architecture.sh` and `scripts/test-check-cli-architecture.sh`
  are now permanent parts of the validation surface (`nix flake check`) that
  any future change touching `cli/src/domain/**` or `cli/src/application/**`
  must satisfy.
- `services/**` is now documented as explicitly temporary; new code should
  default to landing in the new layers rather than growing `services/**`
  further where avoidable.

## Follow-up

- None. This decision covers the skeleton only; specific vertical-slice
  migrations out of `services/**` are future plans, not committed follow-up
  work.

## References

- Plan: [`cli-hexagonal-architecture-skeleton`](../plans/cli-hexagonal-architecture-skeleton.md)
- Task: `T01`, `T02`, `T03`, `T04`, `T05`
- Current-state context: [`context/architecture.md`](../architecture.md) (`## CLI internal hexagonal architecture`)
- Evidence: [`context/plans/cli-hexagonal-architecture-skeleton.md`](../plans/cli-hexagonal-architecture-skeleton.md) (`## Validation Report`)
