# Plan: structured-cli-diagnostics

## Change summary

Replaces string-encoded remediation (`"... Try: ..."` concatenated directly into
`ClassifiedError` messages, detected downstream via `message().contains("Try:")`)
with an explicit `hint: Option<String>` field owned by `ClassifiedError` in
`cli/src/services/error.rs`. The shared stderr renderer in
`cli/src/services/app_support.rs` becomes the sole owner of the literal `Try:`
presentation: it uses an explicit hint when set, and falls back to
`FailureClass::default_try_guidance()` otherwise, with no message-text
inspection. This extends existing behavior; it does not replace the error
classes, `SCE-ERR-*` codes, or exit codes, and preserves current rendered text
for every parser, invocation-validation, and runtime diagnostic in the CLI.

Beyond the renderer and the explicitly-named parser/invocation call sites, the
codebase has roughly a dozen other places (`auth.rs`, `token_storage.rs`,
`db/mod.rs`, `resilience.rs`, `encryption_key.rs`, `security.rs`,
`observability.rs`, `agent_trace_sync/control_plane.rs`, and
`auth_command/mod.rs`) that already bake `Try:` remediation into plain
`anyhow::Error`/`String` text, which later becomes `ClassifiedError::runtime(...)`
through seven generic `.map_err(...)` conversion points. Once the renderer stops
inspecting message text, any of those left alone would get the class-default
guidance appended on top of their existing text — a real double-`Try:`
regression. Per user decision (2026-08-12 clarification), this plan adds one
small shared conversion helper at that anyhow boundary instead of migrating
every one of those dozen files individually, keeping the patch narrow while
still preserving exact current output.

## Acceptance criteria

- [ ] AC1: `ClassifiedError` exposes an explicit `hint: Option<String>` via
      `with_hint()`/`hint()`, defaulting to `None` on `parse`/`validation`/
      `runtime`/`dependency`, and the stderr renderer in `app_support.rs`
      selects an explicit hint when present or `FailureClass::default_try_guidance()`
      otherwise, with no `message().contains("Try:")` inspection remaining in
      the renderer.
  - Validate: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml error:: app_support::`; `grep -n 'contains("Try:")' cli/src/services/app_support.rs` returns nothing.
- [x] AC2: Every parser/invocation diagnostic named in the PR's priority list
      (missing subcommand, unknown command, unavailable command, unknown
      option, missing required argument, conflicting arguments, unregistered
      command, setup invocation validation, bash-policy STDIN validation) now
      attaches its remediation via `.with_hint()` instead of message
      concatenation, and renders byte-identical final stderr text to current
      behavior.
  - Validate: targeted tests reproducing each case's exact prior stderr text (e.g. `sce`, `sce frobnicate`, `sce --frobnicate`, `sce setup --repo x`, `sce policy bash` with empty STDIN).
- [x] AC3: Every anyhow-originated error reaching `ClassifiedError::runtime`
      through the shared conversion boundary (`auth_command/command.rs`,
      `config/command.rs`, `version/command.rs`, `hooks/command.rs`,
      `setup/command.rs`, `trace/command.rs`, `doctor/command.rs`, and
      `command_runtime.rs::registry_command`) that already carries a trailing
      `Try: ...` suffix is split into an explicit hint by one shared helper at
      construction time, so it is never doubled with the class-default
      guidance.
  - Validate: helper unit tests (message with/without a trailing `Try:` suffix); an app-boundary test reproducing one deep call site's previous exact output unchanged (e.g. `sce auth renew` with no stored credentials).
- [x] AC4: `context/sce/cli-error-code-taxonomy.md` documents the new
      ownership split (`ClassifiedError` owns optional hint data, `FailureClass`
      owns default remediation, the renderer owns `Try:` presentation) and
      names the anyhow-boundary helper as the one intentional,
      construction-time exception to "remediation presence is no longer
      decided by inspecting message text."
  - Validate: manual review of the updated doc against the implemented renderer and helper.
- [x] AC5: A repository-wide search for `contains("Try:")` and literal `"Try:"`
      string concatenation outside test code finds no unexplained occurrence;
      the ones that remain (the anyhow-boundary helper itself, and
      `auth_command/mod.rs`'s `with_try_guidance`, which operates on plain
      `anyhow::Error`/`String` values upstream of any `ClassifiedError` and
      whose own de-dup logic must stay to avoid a double `Try:` inside that
      pre-boundary text) are named in the taxonomy doc or task evidence.
  - Validate: `grep -rn 'Try:' cli/src --include=*.rs` reviewed by hand against the documented exception list.

### Full validation

- `nix flake check`

### Context sync

- `context/sce/cli-error-code-taxonomy.md`

## Constraints and non-goals

- **In scope:** `cli/src/services/error.rs`; `cli/src/services/app_support.rs`
  (renderer only); `cli/src/services/parse/command_runtime.rs` (parser/clap
  cleanup); `cli/src/services/bash_policy.rs` (directly-constructed validation
  errors); `cli/src/services/setup/mod.rs` (invocation-validation errors); the
  seven anyhow-boundary `.map_err` sites in `auth_command/command.rs`,
  `config/command.rs`, `version/command.rs`, `hooks/command.rs`,
  `setup/command.rs`, `trace/command.rs`, `doctor/command.rs`; and
  `context/sce/cli-error-code-taxonomy.md`.
- **Out of scope:** typed `CommandOutput` or any refactor of successful command
  payloads (explicitly deferred to a later PR); rewriting `auth.rs`'s
  per-variant `AuthError` messages or `auth_command/mod.rs`'s
  `with_try_guidance` de-dup helper; the other anyhow-message-construction
  files that embed `Try:` text but never need structural change beyond the
  shared boundary helper (`token_storage.rs`, `db/mod.rs`, `resilience.rs`,
  `encryption_key.rs`, `security.rs`, `observability.rs`,
  `agent_trace_sync/control_plane.rs`); `SCE-ERR-*` codes; exit codes;
  `--format` semantics; new dependencies; styling; logging event IDs.
- **Constraints:** No new crate dependencies. `ClassifiedError` must not depend
  on `clap` types. Preserve existing rendered diagnostic text, punctuation,
  error code, stdout/stderr routing, exit code, and redaction for every
  migrated call site.
- **Non-goal:** A general-purpose diagnostic/hint framework beyond
  `ClassifiedError`'s single optional `hint` field.

## Assumptions

- The "shared boundary-conversion helper" approach was chosen by the user over
  full per-site migration of every `Try:`-bearing anyhow message, and over
  accepting the double-`Try:` regression as a known gap (2026-08-12
  clarification).
- `auth_command/mod.rs`'s `with_try_guidance`/`contains("Try:")` de-dup logic
  is left unchanged: it composes plain `anyhow::Error`/`String` text upstream
  of any `ClassifiedError` construction, and its own de-dup check prevents a
  genuine double-`Try:` clause inside that pre-boundary text (e.g. when
  `AuthError`'s own `Display` already embeds `Try:` guidance). Removing it
  would be a real regression, not a simplification, so it stays as one of the
  intentional, explainable exceptions AC5 records.
- The anyhow-boundary conversion sites needing the new helper are exactly:
  `auth_command/command.rs:11`, `config/command.rs:11`, `version/command.rs:11`,
  `hooks/command.rs:12`, `setup/command.rs` (six `map_err` sites),
  `trace/command.rs` (multiple sites), `doctor/command.rs:12`, plus
  `command_runtime.rs::registry_command`'s two direct
  `ClassifiedError::runtime(...)` sites. `cli/src/app.rs`'s direct
  `ClassifiedError::dependency`/`runtime` constructions do not embed `Try:`
  text today and need no change, confirmed by inspection during planning.
- Durable context drift: `context/context-map.md`, `context/overview.md`, and
  `context/sce/cli-error-code-taxonomy.md` currently describe
  `ClassifiedError`/`FailureClass` as living in `cli/src/app.rs`; the code
  already moved them to `cli/src/services/error.rs` (rendering in
  `cli/src/services/app_support.rs`) before this plan. This plan corrects the
  taxonomy doc's ownership section as part of its own change but does not
  audit every other stale `app.rs` reference across `context/`, since that
  drift predates and is unrelated to this change.

## Task stack

- [x] T01: `Add an explicit hint field to ClassifiedError` (status:done)
  - Task ID: T01
  - Goal: `ClassifiedError` carries `hint: Option<String>` with a `#[must_use] with_hint(...)` builder and `hint()` accessor; `parse`/`validation`/`runtime`/`dependency` constructors default it to `None`. The renderer in `app_support.rs` prefers an explicit hint when present, and otherwise falls back to today's exact logic (`message().contains("Try:")` then class-default) unchanged, so this task is purely additive and non-regressing.
  - Boundaries (in/out of scope): In — `cli/src/services/error.rs`; the renderer function in `cli/src/services/app_support.rs` gains a hint-first branch but keeps its existing fallback behavior for messages with no explicit hint. Out — migrating any call site to set a hint; removing the legacy `contains("Try:")` fallback (T06).
  - Dependencies: none
  - Done when: `ClassifiedError::parse("x").with_hint("y")` renders as `Error [SCE-ERR-PARSE]: x Try: y`; every existing call site (none of which set a hint yet) renders identically to current output; `code()`/`class()`/exit-code mapping are unchanged.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml error:: app_support::` covering: hint-present rendering, hint-absent class-default rendering, no double guidance when a hint is set, error code stable, exit code stable, redaction still applied to a hinted message.
  - Implementation evidence: Added `hint: Option<String>` to `ClassifiedError` in
    `cli/src/services/error.rs`, defaulted to `None` in `parse`/`validation`/
    `runtime`/`dependency`, plus a `#[must_use] with_hint(...)` builder and
    `hint()` accessor. `write_error_diagnostic` in
    `cli/src/services/app_support.rs` now branches on `error.hint()` first,
    falling back unchanged to the existing `contains("Try:")`/class-default
    logic when no hint is set. No call site sets a hint yet.
  - Verification outcome: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::error::`
    (3 passed) and `./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml services::app_support::` (5 passed) — hint-present
    rendering, hint-absent class-default fallback, no double `Try:` when a
    hint is set, class/code/exit-code stability, and redaction of a hinted
    message all verified. `cargo fmt --manifest-path cli/Cargo.toml` applied.
  - Deviations/assumptions: None beyond the reviewed task scope.

- [x] T02: `Structure clap-derived parser diagnostics with explicit hints` (status:done)
  - Task ID: T02
  - Goal: Replace `clean_clap_error_message`'s string-concatenated `Try:` output in `cli/src/services/parse/command_runtime.rs` with a small `CleanedClapError { message, hint }` value; `classify_clap_error` attaches the hint via `.with_hint()`. `handle_clap_error`'s literal `"Missing required subcommand. Try: run 'sce --help'..."` and `registry_command`'s two `"...is not registered. Try: run 'sce --help'..."` messages are split into `.with_hint(...)` calls the same way. `ClassifiedError` must not gain a dependency on `clap` types.
  - Boundaries (in/out of scope): In — `cli/src/services/parse/command_runtime.rs` only. Out — the anyhow-boundary helper (T05); non-clap invocation errors (T03, T04).
  - Dependencies: T01
  - Done when: `sce`, `sce frobnicate`, `sce --frobnicate`, a missing-required-argument invocation, and a conflicting-argument invocation each render exactly the same final stderr text as before this task; `clean_clap_error_message` no longer contains `message.contains("Try:")` or string-concatenates `Try:`.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml command_runtime::` covering missing-subcommand, unknown-command, unavailable-command, unknown-option, missing-required-argument, and conflicting-argument cases with exact-text assertions.
  - Implementation evidence: Introduced a private `CleanedClapError { message, hint }`
    struct in `cli/src/services/parse/command_runtime.rs`. `clean_clap_error_message`
    now returns it, splitting every branch's former `. Try: ...` suffix into the
    `hint` field instead of concatenating it into `message`; the previously dead
    `message.contains("Try:")` guard in the fallback arm was removed since it never
    matched a real clap-produced message. `classify_clap_error` builds the
    `ClassifiedError` from `cleaned.message` and appends `.with_hint(hint)` when
    present. `handle_clap_error`'s missing-subcommand literal and both
    `registry_command` "not registered" literals were split the same way. No `clap`
    type was added to `ClassifiedError` or `error.rs`.
  - Verification outcome: `./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml command_runtime::` (10 passed) — added exact-text tests for
    missing-subcommand (`sce hooks`), unknown-command (`sce frobnicate`),
    unknown-option (`sce --frobnicate`), missing-required-argument
    (`sce completion`), argument-conflict (`sce setup --opencode --claude`), and
    unregistered-command cases, each asserting the combined message+hint renders
    byte-identical text to the prior concatenated string.
    `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` (348 passed,
    full suite, no regressions). `./scripts/run-cli-cargo.sh clippy --manifest-path
    cli/Cargo.toml --bins --tests -- -D warnings` clean. `cargo fmt --manifest-path
    cli/Cargo.toml -- --check` clean.
  - Deviations/assumptions: The "unavailable in this build" `InvalidSubcommand`
    branch (a known command name that clap itself failed to parse) is not
    reachable through direct integration testing in the current build, since
    every name in `TOP_LEVEL_COMMANDS` is always present in the derived `Commands`
    enum; its exact-text preservation was verified by code inspection of the
    branch's `message`/`hint` split instead of a runtime assertion.

- [x] T03: `Structure bash-policy STDIN validation diagnostics with explicit hints` (status:done)
  - Task ID: T03
  - Goal: `read_stdin_payload` and `parse_json_payload` in `cli/src/services/bash_policy.rs` stop concatenating `Try:` text into their `ClassifiedError::validation(...)` messages and attach it via `.with_hint(...)` instead.
  - Boundaries (in/out of scope): In — the two named functions in `cli/src/services/bash_policy.rs`. Out — any other `bash_policy.rs` error path that does not already embed `Try:` text.
  - Dependencies: T01
  - Done when: a failed STDIN read and an invalid-JSON STDIN payload each render exactly the same final stderr text as before this task.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml bash_policy::` covering STDIN-read failure and invalid-JSON cases with exact-text assertions.
  - Implementation evidence: `read_stdin_payload`'s IO-failure and empty-payload messages,
    and `parse_json_payload`'s invalid-JSON message, in `cli/src/services/bash_policy.rs`
    no longer concatenate `. Try: ...` text into the message string; each now calls
    `.with_hint(...)` with the same guidance text instead, so the renderer's
    `"{message} Try: {hint}"` format reproduces byte-identical prior output. The
    empty-payload check was extracted from `read_stdin_payload` into a private
    `require_non_empty_stdin_payload(payload: String)` helper so its message/hint pair
    is directly unit-testable without mocking real process STDIN.
  - Verification outcome: `./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml bash_policy::` (39 passed) — added exact message/hint assertions for
    the empty-STDIN case (via the new `require_non_empty_stdin_payload` helper), a
    whitespace-only-STDIN case, and the invalid-JSON case (via `parse_json_payload`
    called directly). `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`
    (351 passed, full suite, no regressions). `./scripts/run-cli-cargo.sh clippy
    --manifest-path cli/Cargo.toml --bins --tests -- -D warnings` clean. `cargo fmt
    --manifest-path cli/Cargo.toml -- --check` clean.
  - Deviations/assumptions: The STDIN IO-read-failure branch (`io::stdin().read_to_string`
    returning `Err`) is not reachable through a deterministic unit test without mocking
    real process STDIN, consistent with T02's precedent for an untestable clap branch;
    its exact message/hint split was verified by code inspection instead of a runtime
    assertion. Extracting `require_non_empty_stdin_payload` as a private helper is a
    reversible, local testability change with identical external behavior, not a scope
    expansion beyond the two named functions.

- [x] T04: `Structure setup invocation-validation diagnostics with explicit hints` (status:done)
  - Task ID: T04
  - Goal: Every setup invocation-validation error in `cli/src/services/setup/mod.rs` that concatenates `Try:` text (`--repo` without `--hooks`, `--workflow` conflicting with `--bootstrap-context`, `--bootstrap-context` not used alone, mutually exclusive target flags, `--non-interactive` requiring a target, `--workflow` requiring a target, unknown `--workflow` value, empty/unresolvable/non-directory/non-git `--repo`) attaches its remediation via `.with_hint(...)` instead.
  - Boundaries (in/out of scope): In — the invocation-validation error sites in `cli/src/services/setup/mod.rs` listed above. Out — setup's other runtime-failure error paths that do not embed `Try:` text; the asset-destination-is-a-directory message at line ~1378 stays as-is unless it also embeds `Try:` (verify and migrate consistently if so).
  - Dependencies: T01
  - Done when: `sce setup --hooks --repo` misuse, mutually-exclusive target flags, and an unknown `--workflow` value each render exactly the same final stderr text as before this task.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml setup::` covering the `--repo`-without-`--hooks` case, the mutually-exclusive-target-flags case, and the unknown-`--workflow` case with exact-text assertions.
  - Implementation evidence: `resolve_setup_request` and `validate_optional_workflow_slugs` in
    `cli/src/services/setup/mod.rs` now return `Result<_, ClassifiedError>` instead of
    `anyhow::Result`; each of the seven `bail!` sites they own (`--repo` without `--hooks`,
    `--workflow` with `--bootstrap-context`, `--bootstrap-context` not alone, mutually
    exclusive target flags, `--non-interactive` without a target, `--workflow` without a
    target, unknown `--workflow` slug) now returns
    `ClassifiedError::validation(message).with_hint(hint)` with the same text split at the
    former `. Try: ` boundary, so the rendered `"{message} Try: {hint}"` output is
    byte-identical to the prior concatenated string. `convert_setup_command` in
    `cli/src/services/parse/command_runtime.rs` no longer converts the anyhow error via
    `.map_err(|error| ClassifiedError::validation(error.to_string()))` (which would have
    discarded the hint); it now propagates the already-typed `ClassifiedError` directly
    with `?`.
  - Verification outcome: `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml
    setup::` (58 passed) — added exact-text assertions (via a `rendered_text` helper
    matching the renderer's `"{message} Try: {hint}"` format, mirroring T02's precedent) for
    the `--repo`-without-`--hooks` case, the mutually-exclusive-target-flags case, and the
    unknown-`--workflow` case, plus updated the three pre-existing `resolve_setup_request`
    rejection tests to assert on `.message()`/`.hint()` instead of `.to_string()`.
    `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` (354 passed, full
    suite, no regressions). `./scripts/run-cli-cargo.sh clippy --manifest-path
    cli/Cargo.toml --bins --tests -- -D warnings` clean. `cargo fmt --manifest-path
    cli/Cargo.toml -- --check` clean.
  - Deviations/assumptions: The Goal's parenthetical also names the empty/unresolvable/
    non-directory/non-git `--repo` validation (`normalize_user_repository_path` and
    `map_setup_non_git_repository_error`, both inside the private `install` submodule of
    this same file). Those checks run during actual repository installation, not CLI
    invocation parsing: they're reached only through `prepare_setup_hooks_repository` /
    `ensure_git_repository`, whose errors already flow through
    `setup/command.rs`'s generic anyhow-boundary `.map_err(|error|
    ClassifiedError::runtime(format!("{error:#}")))` — one of T05's seven listed boundary
    sites. T05's shared helper splits any trailing `" Try: ..."` suffix into a hint
    generically at that boundary, so migrating these deep, still-anyhow-typed runtime
    functions here would duplicate T05's work and pull "other runtime-failure error paths"
    (explicitly out of scope for this task) into scope. Left them unconverted for T05 to
    cover, consistent with this task's own Done-when checks and Verification notes, which
    name only the three `resolve_setup_request`/`validate_optional_workflow_slugs` cases.
    Confirmed by inspection that `resolve_setup_request` and `validate_optional_workflow_slugs`
    are the only invocation-validation call sites reached through
    `command_runtime.rs::convert_setup_command`, a boundary T05's site list does not cover
    (T05 lists `setup/command.rs`, not `command_runtime.rs::convert_setup_command`) —
    leaving these two functions unmigrated would have caused a real double-`Try:` regression
    once T06 removes the renderer's `contains("Try:")` fallback.

- [x] T05: `Add a shared anyhow-boundary hint-extraction helper` (status:done)
  - Task ID: T05
  - Goal: Add one small helper in `cli/src/services/error.rs` (e.g. `ClassifiedError::from_anyhow` or a free function) that takes the converted `anyhow::Error` text, splits a trailing `" Try: ..."` suffix into an explicit hint when present, and constructs `ClassifiedError::runtime(...)` with that hint attached. Replace the seven duplicated `.map_err(|error| ClassifiedError::runtime(format!("{error:#}")))` closures in `auth_command/command.rs`, `config/command.rs`, `version/command.rs`, `hooks/command.rs`, `setup/command.rs` (all its `map_err` sites), `trace/command.rs` (all its sites), and `doctor/command.rs` with calls to the shared helper. Apply the same helper (or its splitting logic) to `command_runtime.rs::registry_command`'s two direct `ClassifiedError::runtime(...)` constructions.
  - Boundaries (in/out of scope): In — the new helper in `cli/src/services/error.rs`; the seven `command.rs` conversion sites; `command_runtime.rs::registry_command`. Out — any change to the anyhow/String message construction inside `auth.rs`, `token_storage.rs`, `db/mod.rs`, `resilience.rs`, `encryption_key.rs`, `security.rs`, `observability.rs`, `agent_trace_sync/control_plane.rs`, or `auth_command/mod.rs`.
  - Dependencies: T01
  - Done when: a helper unit test confirms a message with a trailing `Try: ...` suffix is split into `(message, Some(hint))` and a message without one is left as `(message, None)`; `sce auth renew` with no stored credentials renders exactly the same final stderr text as before this task; every listed conversion site uses the shared helper instead of a duplicated inline closure.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml error:: auth_command:: trace::` covering the helper's split behavior and at least one deep call site's exact-text output.
  - Implementation evidence: Added `ClassifiedError::from_anyhow_text(message)` in
    `cli/src/services/error.rs`, which `rsplit_once(" Try: ")`s the converted anyhow
    text and either builds `runtime(body).with_hint(hint)` when a trailing `Try:`
    suffix is present or plain `runtime(message)` when it is not, splitting on the
    *last* occurrence so text with an inner `Try:` clause (e.g. `auth_command/mod.rs`'s
    already-de-duped pre-boundary text) still yields the correct outer hint. Replaced
    all seven originally-listed `.map_err(|error| ClassifiedError::runtime(format!("{error:#}")))`
    closures — `auth_command/command.rs:11`, `config/command.rs:11`,
    `version/command.rs:11`, `hooks/command.rs:12`, `doctor/command.rs:12`, all six
    `map_err` sites in `setup/command.rs`, and all eight matching sites in
    `trace/command.rs` — with `ClassifiedError::from_anyhow_text(format!("{error:#}"))`.
    `command_runtime.rs::registry_command`'s two direct constructions needed no change:
    they already build `ClassifiedError::runtime(...).with_hint(...)` directly (done in
    T02, since that code path never routes through anyhow text), so there was nothing
    left for a text-splitting helper to do there. `trace/command.rs`'s three other
    direct `ClassifiedError::runtime(...)` constructions (`current_repo_root`,
    `classify_status_error`, `classify_sync_error`) were left unchanged: they are not
    among the originally-listed `.map_err` closures, none of their source error types
    currently embed `Try:` text (confirmed by inspection — no `Try:` string exists
    anywhere in `cli/src/services/trace/`), and converting them was out of this task's
    named scope.
  - Verification outcome: `./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml error::` (6 passed, including three new `from_anyhow_text` unit
    tests: split-on-trailing-`Try:`, no-split-when-absent, split-on-the-last-of-two
    `Try:` occurrences). `./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml` (357 passed, full suite, no regressions).
    `./scripts/run-cli-cargo.sh clippy --manifest-path cli/Cargo.toml --bins --tests
    -- -D warnings` clean. `cargo fmt --manifest-path cli/Cargo.toml -- --check`
    clean. `sce auth renew` with no stored credentials was verified by code
    inspection rather than a runtime assertion: `auth_command/mod.rs:106` still
    returns the exact literal `"No stored WorkOS credentials were found. Try: run
    'sce auth login' before running 'sce auth renew'."`, and `from_anyhow_text`
    splits that unchanged string into
    `(message: "No stored WorkOS credentials were found.", hint: Some("run 'sce auth
    login' before running 'sce auth renew'."))`, which the renderer's `"{message} Try:
    {hint}"` format reproduces byte-identical to the prior concatenated string.
    `token_storage::load_tokens` resolves its DB path from the shared XDG-style
    state-root catalog with no per-test override, so it is not reachable through a
    deterministic unit test without mutating real process-wide state, consistent
    with T02/T03's precedent for untestable branches.
  - Deviations/assumptions: `command_runtime.rs::registry_command` required no
    change (already migrated in T02, as noted above) — a narrowing of this task's
    literal wording, not a scope expansion, since the two sites the plan named
    already satisfy the Done-when check ("every listed conversion site uses the
    shared helper instead of a duplicated inline closure": these sites use
    `.with_hint(...)` directly and have no duplicated inline closure to replace).
    `trace/command.rs`'s three non-`map_err` direct constructions were left
    unmigrated as out of this task's named scope; this creates no double-`Try:` risk
    today since none of their source text ever embeds `Try:` guidance.

- [x] T06: `Remove the renderer's legacy message-text remediation detection` (status:done)
  - Task ID: T06
  - Goal: With every `ClassifiedError`-reaching construction site now attaching remediation through an explicit hint (T02-T05) or never embedding `Try:` text at all, remove the `message().contains("Try:")` fallback branch from the renderer in `cli/src/services/app_support.rs` added in T01, leaving only: explicit hint present -> use it; otherwise -> `FailureClass::default_try_guidance()`.
  - Boundaries (in/out of scope): In — the renderer function in `cli/src/services/app_support.rs` only. Out — any call site change (already complete by T05).
  - Dependencies: T02, T03, T04, T05
  - Done when: `cli/src/services/app_support.rs` contains no `contains("Try:")`; the full existing plus newly-added test suite for every previously-Try:-bearing diagnostic (parser, bash-policy, setup, and the anyhow-boundary cases) still renders byte-identical stderr text.
  - Verification notes (commands or checks): `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml`; `grep -rn 'contains("Try:")' cli/src/services/app_support.rs` returns nothing.
  - Implementation evidence: `write_error_diagnostic` in `cli/src/services/app_support.rs`
    now branches only on `error.hint()`: `Some(hint)` renders the explicit hint,
    `None` falls back directly to `error.class().default_try_guidance()`. The
    `else if error.message().contains("Try:")` branch (and its
    `message_already_containing_try_is_left_unchanged_without_a_hint` test, which
    asserted the now-deleted fallback behavior) were removed. No call site changed.
  - Verification outcome: `./scripts/run-cli-cargo.sh test --manifest-path
    cli/Cargo.toml` (356 passed, full suite, no regressions — one prior test removed
    since it exercised the deleted fallback branch). `grep -n 'contains("Try:")'
    cli/src/services/app_support.rs` returns nothing. `./scripts/run-cli-cargo.sh
    clippy --manifest-path cli/Cargo.toml --bins --tests -- -D warnings` clean.
    `cargo fmt --manifest-path cli/Cargo.toml -- --check` clean.
  - Deviations/assumptions: None beyond the reviewed task scope.

- [x] T07: `Document the hint-owned remediation model in the error-code taxonomy` (status:done)
  - Task ID: T07
  - Goal: Update `context/sce/cli-error-code-taxonomy.md` to describe `ClassifiedError` owning optional hint data, `FailureClass` owning default remediation, the renderer owning `Try:` presentation, and remediation presence no longer being decided by inspecting rendered message text at the render layer — naming the anyhow-boundary helper (T05) as the one intentional, construction-time exception, and `auth_command/mod.rs`'s `with_try_guidance` as an out-of-scope pre-boundary de-dup left unchanged.
  - Boundaries (in/out of scope): In — `context/sce/cli-error-code-taxonomy.md` only. Out — auditing or correcting other stale `app.rs`-ownership references in `context/overview.md` or `context/context-map.md` (pre-existing, unrelated drift).
  - Dependencies: T06
  - Done when: the taxonomy doc's rendering and ownership sections describe the implemented model accurately, with no remaining reference to message-text `contains("Try:")` detection except the documented boundary-helper exception.
  - Verification notes (commands or checks): manual review of the updated doc against `cli/src/services/error.rs` and `cli/src/services/app_support.rs`.
  - Implementation evidence: Rewrote `context/sce/cli-error-code-taxonomy.md`'s
    **Scope**, **Rendering contract**, **Actionable parser/invocation guidance
    contract**, **Ownership**, and **Determinism and testing** sections to
    reflect the implemented model: `ClassifiedError` (`cli/src/services/error.rs`)
    owns code/class/message/optional hint; `FailureClass` owns
    `default_try_guidance()`; `write_error_diagnostic` in
    `cli/src/services/app_support.rs` owns final `Try:` presentation, selecting
    an explicit hint when set or the class default otherwise, with no
    message-text inspection. Corrected stale ownership references from
    `cli/src/app.rs` to `cli/src/services/error.rs` /
    `cli/src/services/app_support.rs` (`app.rs` now only invokes
    `app_support::render_run_outcome`). Added a new **The anyhow-boundary
    exception** subsection under **Ownership** documenting
    `ClassifiedError::from_anyhow_text` (T05) as the one intentional,
    construction-time exception, and `auth_command/mod.rs`'s `with_try_guidance`
    as a separate, out-of-scope, pre-boundary de-dup helper operating on plain
    `anyhow::Error`/`String` text upstream of any `ClassifiedError`. Updated the
    actionable-guidance ownership line to name
    `cli/src/services/parse/command_runtime.rs`, `cli/src/services/bash_policy.rs`,
    and `cli/src/services/setup/mod.rs` instead of the stale `app.rs`/`hooks/mod.rs`
    references.
  - Verification outcome: Manual review of the updated doc against
    `cli/src/services/error.rs` (`FailureClass`, `ClassifiedError`,
    `with_hint`/`hint`, `from_anyhow_text`) and
    `cli/src/services/app_support.rs` (`write_error_diagnostic`,
    `exit_with_error`, `render_run_outcome`) confirms the described ownership
    and rendering behavior matches the implemented code exactly.
    `grep -n 'contains("Try:")' context/sce/cli-error-code-taxonomy.md` finds
    nothing; the doc's only remaining `Try:`-detection mention is
    `auth_command/mod.rs`'s pre-boundary `with_try_guidance`, explicitly named
    as the documented out-of-scope exception.
  - Deviations/assumptions: None beyond the reviewed task scope.

## Open questions

None. The one material scope ambiguity (how to handle the anyhow-routed messages that already embed `Try:` text) was resolved via the 2026-08-12 clarification gate; the chosen shared-helper approach is recorded above and reflected in the task stack.

## Validation Report

**Status:** failed
**Date:** 2026-08-12

### Commands run

- `nix flake check` -> exit 1 (fails on `checks.x86_64-linux.cli-tests`: 6 test failures — 2 in `services::app_support::tests` and 4 in `services::agent_trace_db`/`services::agent_trace_export` unrelated to this plan)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml` -> exit 0 (356 passed, 0 failed, outside the nix sandbox)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::error::` -> exit 0 (6 passed)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::app_support::` -> exit 0 (4 passed, non-tty stderr)
- `script -qec "./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::app_support::" /dev/null` -> exit 101 (2 of 4 failed: `explicit_hint_is_rendered`, `absent_hint_falls_back_to_class_default_guidance` — reproduces the nix failure under a real tty)
- `./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml services::trace::` -> exit 0 (29 passed)
- `grep -n 'contains("Try:")' cli/src/services/app_support.rs` -> exit 1, no output (AC1's second check passes)
- `grep -rn 'Try:' cli/src --include=*.rs` -> reviewed by hand; every remaining occurrence outside `error.rs`'s `from_anyhow_text` and `auth_command/mod.rs`'s `with_try_guidance` is a construction-time message/hint literal or test assertion, consistent with the taxonomy doc and prior task evidence (AC5)

### Scaffolding removed

- None.

### Success-criteria verification

- [ ] AC1: `ClassifiedError` exposes an explicit `hint`, and the renderer prefers it over class-default guidance with no message-text inspection -> the structural change and its own direct-shell test run pass, but the two `app_support::tests` added for this criterion (`explicit_hint_is_rendered`, `absent_hint_falls_back_to_class_default_guidance`) assert an exact plain-text stderr string while `write_error_diagnostic` styles output via `services::style::error_text`/`heading`/`error_code`, whose color decision reads the real process `io::stderr().is_terminal()` rather than any test-controlled policy. When the test binary's stderr is a real tty (reproduced locally via `script`) or under `nix flake check`'s sandboxed build, styling activates and the assertions fail with ANSI-wrapped output. This is a genuine, environment-dependent test defect introduced by this criterion's own tests, not a flake in an unrelated area.
- [x] AC2: parser/invocation diagnostics attach remediation via `.with_hint()` and render byte-identical stderr -> `command_runtime::`, `bash_policy::`, and `setup::` targeted exact-text tests pass in the full 356-test run; these tests build expected text directly from `.message()`/`.hint()` without going through the styled renderer, so they are unaffected by the AC1 tty issue.
- [x] AC3: anyhow-boundary conversions split a trailing `Try:` suffix via one shared helper -> `services::error::` helper unit tests pass (6/6, including `from_anyhow_text` split/no-split/last-occurrence cases); the `sce auth renew` deep call-site case has no automated test (`auth_command` has no test module) and remains verified by code inspection only, per T05's recorded evidence.
- [x] AC4: taxonomy doc documents the ownership split and the anyhow-boundary exception -> manual review of `context/sce/cli-error-code-taxonomy.md` against `cli/src/services/error.rs` and `cli/src/services/app_support.rs` confirms accuracy (done in T07).
- [x] AC5: no unexplained `Try:`/`contains("Try:")` occurrence remains -> `grep -rn 'Try:' cli/src --include=*.rs` reviewed; all remaining sites are either the documented `from_anyhow_text`/`with_try_guidance` exceptions or plain anyhow-message-construction files named out-of-scope in this plan's Change summary/Constraints (e.g. `auth.rs`, `token_storage.rs`, `db/mod.rs`, `resilience.rs`, `security.rs`, `observability.rs`, `agent_trace_sync/control_plane.rs`).

### Failed checks and follow-ups

- `nix flake check` / `checks.x86_64-linux.cli-tests`: fails deterministically under a real tty or the nix sandbox on `services::app_support::tests::explicit_hint_is_rendered` and `absent_hint_falls_back_to_class_default_guidance`; evidence: ANSI-wrapped left-hand values in both nix's build log and a local `script`-wrapped rerun, e.g. `"\u{1b}[1m\u{1b}[36mError\u{1b}[39m\u{1b}[0m [...]: \u{1b}[33mx Try: y\u{1b}[39m\n"` vs expected `"Error [SCE-ERR-PARSE]: x Try: y\n"`; required: make these two tests deterministic regardless of the real process's terminal state — e.g. inject/override the color policy for the test's writer instead of `write_error_diagnostic` reading real `io::stderr()`, or assert on hint/message content only, ignoring styling, the way `command_runtime::`/`setup::`'s `rendered_text` test helpers already do.
- `nix flake check` / `checks.x86_64-linux.cli-tests`: 4 additional failures in `services::agent_trace_db::tests::recent_diff_trace_patches_applies_bounded_window_ordering_and_parse_accounting` and three `services::agent_trace_export::tests::*` cases, all `UNIQUE constraint failed: agent_traces.id` / DB-lock-retry errors; evidence: nix build log for `sce-cli-tests-test-0.4.0-pre-alpha-v2.drv`; not reproduced locally with `--test-threads=16` (356/356 passed) and not touched by this plan's in-scope files (`agent_trace_db`, `agent_trace_export` are out of this plan's scope entirely); required: a decision on whether this is pre-existing nix-sandbox test-isolation flakiness (needs its own investigation/fix outside this plan) or whether it blocks this plan's validation — recommend re-running `nix flake check` once to confirm reproducibility before treating it as this plan's blocker.

### Residual risks

- The AC1 test defect could mask a real production behavior gap only if `write_error_diagnostic`'s styling were itself incorrect; code inspection shows the styling logic is unchanged, correct production behavior — the defect is confined to test assertions, not the shipped renderer.

### Retry

After repairs, rerun:

`/validate context/plans/structured-cli-diagnostics.md`
