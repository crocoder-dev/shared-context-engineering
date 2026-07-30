# Plan: workflow-eval-harness

## Change summary

Add a local evaluation harness under `evals/` that measures SCE workflow
conformance across three coding-agent harnesses (Claude Code, Pi, OpenCode).
Each run copies a small synthetic fixture repository into a temporary
directory, initializes a fresh Git repository, installs the generated SCE
payload for the selected harness, executes the harness as a one-shot
subprocess inside `nix run github:sin-ack/agent-run --`, captures the
transcript, Git diff, produced artifacts, timing, and token/cost usage, grades
the result deterministically, and destroys the temporary repository.

The runner is TypeScript built on Effect v4. Effect is not decoration here: the
workspace lifecycle is `Effect.acquireRelease` inside a `Scope`, so a temp repo
is destroyed on success, failure, and interrupt alike; harness stdout is a
`Stream` of JSONL events with real backpressure and a `Effect.timeout`
boundary; `expected.yaml` manifests and every harness's event stream are
decoded through `Schema` at the boundary rather than trusted; and the three
harness adapters plus the grader are `Context.Service` modules, so the suite
can be tested end to end against test layers without spawning a live agent.

This is new work: the repository currently has no behavioral testing of the
workflow prose it generates. `nix run .#pkl-check-generated` asserts generated
artifact paths, metadata, and forbidden-path contracts, and
`config/pkl/renderers/generation-contract-check.pkl` asserts structural
properties of the rendered Markdown, but nothing verifies that an agent reading
those documents actually follows the workflow's gates. This plan closes that
gap for one workflow — `/change-to-plan` — across all three targets, and leaves
the runner extensible to `/next-task`, `/validate`, `/commit`, and `/handover`.

Grading is layered and deterministic. Layer 1 is conformance: artifact paths,
required sections, task-line shape, allowed side effects, terminal handoff
patterns. Layer 2 is semantic coverage: per-case `required_facts` and
`forbidden_facts` checked against the produced artifact and terminal output.
Both layers gate pass/fail. An LLM-judge quality score is an explicit non-goal
of this plan.

The structural rules in layer 1 come from one checked-in contract profile
derived from `config/pkl/base/workflow-change-to-plan.pkl`, so the eval cannot
drift into defining a second contract. The current canonical contract emits
`## Acceptance criteria` and forbids a trailing validation-and-cleanup task
(`workflow-change-to-plan.pkl:1190`, `:1302`); the `## Success criteria`
heading and mandatory final validation task visible in 63 historical files
under `context/plans/` are the superseded contract and must not be encoded.

## Acceptance criteria

- [ ] AC1: The `evals/` Effect package builds, type-checks, tests, lints, and
      formats, and its checks run inside root `nix flake check`.
  - Validate: `nix flake check` succeeds and reports `evals-tests`,
    `evals-typecheck`, `evals-biome-check`, and `evals-biome-format`
    derivations.
- [ ] AC2: A single `/change-to-plan` case executes end to end on each of
      Claude Code, Pi, and OpenCode inside `agent-run`, producing one
      normalized result record per run containing exit status, timeout status,
      wall-clock duration, decoded event stream, token usage, reported cost,
      tool calls, final Git diff, and grader output.
  - Validate: `cd evals && bun run eval --case change-to-plan/precise-request --harness claude --harness pi --harness opencode --repetitions 1`
    writes three result JSON files under `evals/results/`, each of which
    round-trips through `Schema.decodeUnknownEffect(EvalResult)`.
- [ ] AC3: The grader is harness-agnostic: one `Grader` service produces
      pass/fail for all three harnesses from the normalized result alone.
  - Validate: `rg -n 'claude|opencode|\bpi\b' evals/src/grader/` returns no
    match, and `cd evals && bun run test src/grader` passes.
- [ ] AC4: Layer-1 structural rules are sourced from one checked-in contract
      profile that matches the canonical Pkl contract, not restated per case.
  - Validate: `cd evals && bun run test src/grader/contract-profile.test.ts`
    passes, asserting the profile requires `## Acceptance criteria` and rejects
    a trailing validation-and-cleanup task; cross-check against
    `config/pkl/base/workflow-change-to-plan.pkl:1190` and `:1302`.
- [ ] AC5: Three `/change-to-plan` cases exist — `precise-request`,
      `ambiguous-request`, `constrained-request` — and the suite runs
      3 cases × 3 harnesses × 3 repetitions.
  - Validate: `cd evals && bun run eval --workflow change-to-plan --repetitions 3`
    produces 27 result records and one summary report.
- [ ] AC6: The negative case is graded as a negative case: `ambiguous-request`
      passes only when no plan file was written and the terminal output carries
      categorized clarification questions.
  - Validate: `cd evals && bun run test src/grader/negative-case.test.ts`
    passes against recorded fixtures for both a correct refusal and an
    incorrect plan-writing run.
- [ ] AC7: Every run records provenance: harness version, model identifier,
      generated SCE payload hash, base fixture hash, prompt, and the loaded
      instruction files.
  - Validate: `jq -e '.provenance | .harnessVersion and .modelId and .payloadHash and .fixtureHash'`
    over any result file in `evals/results/` exits 0.
- [ ] AC8: A temp workspace is destroyed on success, failure, and interrupt.
  - Validate: `cd evals && bun run test src/workspace.test.ts` passes,
    including a case that interrupts the run fiber mid-execution and asserts
    the workspace directory no longer exists.

### Full validation

- `nix flake check`
- `nix run .#pkl-check-generated`
- `cd evals && bun run test`

Note: the eval suite itself is not part of `nix flake check`. It requires
network access and provider credentials and spends money, so flake checks cover
only the package's own unit tests, typecheck, lint, and format.

### Context sync

- New domain file `context/sce/workflow-eval-harness.md` describing the eval
  runner's Effect service graph, the fixture/case format, the layered grader,
  the normalized result schema, and the sandbox contract.
- `context/context-map.md` entry for that file.
- `context/overview.md`: note the `evals/` surface and its flake checks.
- `context/patterns.md`: eval authoring conventions (case layout, contract
  profile ownership, harness-agnostic grader rule, Effect service/layer
  conventions for this package).
- `context/patterns.md` root Biome scoping currently records the approved scope
  as `npm/**` and `config/lib/**` only; update it for `evals/**`.

## Constraints and non-goals

- **In scope:** a new top-level `evals/` Effect package; `biome.json` scope
  extension; new flake checks; `evals/.agent-run/config.toml`.
- **Out of scope:** `/next-task`, `/validate`, `/commit`, and `/handover`
  fixtures and graders; a smoke suite against the real SCE repository; CI
  execution of the eval suite; any change to `config/pkl/**`, the generated
  workflow documents, or the Rust CLI.
- **Constraints:**
  - Effect v4 with the conventions in `.claude/skills/effect`: services as
    `Context.Service` with module-namespace projection, implementations as
    `Layer.effect(Service, Effect.gen(...))` returning `Service.of({...})`,
    public methods as `Effect.fn("Domain.operation")`, typed failures as
    `Schema.TaggedErrorClass`, records as `Schema.Struct` plus a same-name
    `interface`, runtime settings through `Config`.
  - Subprocess execution goes through `@effect/platform` `Command` with the
    `@effect/platform-bun` runtime layer, not raw `node:child_process`.
    Filesystem work goes through `@effect/platform` `FileSystem`.
  - Sandboxing is `nix run github:sin-ack/agent-run -- <harness> ...` with a
    checked-in `evals/.agent-run/config.toml`. `agent-run` uses `bwrap` and is
    Linux-only with unprivileged user namespaces enabled.
  - Claude Code runs with `--dangerously-skip-permissions`, which is acceptable
    only because every run is inside `agent-run` against a throwaway temp repo.
  - All three harnesses run cold, one-shot, subprocess mode:
    `pi --mode json --print --no-session`,
    `claude --print --output-format stream-json --verbose`,
    `opencode --pure run --format json --auto`. No warm/server/RPC mode.
  - Pi must not receive `--no-skills` or `--no-prompt-templates`: the installed
    SCE payload is exactly what is under test.
  - Models are configured per harness, not globally. Pi and OpenCode both run
    `gpt-5.6-sol`; Claude Code runs a Claude model. The resolved identifier is
    recorded per run.
  - The generated SCE payload is produced by `nix run .#pkl-generate -- <dir>`;
    the eval never reads committed target trees, which do not exist.
- **Non-goal:** an LLM-judge quality score. The result schema leaves room for
  it, but no judge is implemented, invoked, or recorded in this plan.
- **Non-goal:** a general-purpose agent benchmark. This measures SCE workflow
  conformance, not agent capability.
- **Non-goal:** cross-harness latency or cost comparison tables. Runs are
  uniformly cold, so the numbers are recorded, but this plan draws no
  conclusions from them.
- **Non-goal:** introducing Effect anywhere else in the repository. `config/lib`
  and `npm/` stay as they are.

## Assumptions

- Package manager is Bun (`bun add effect`), matching the existing `config/lib`
  package and `bun.lock` convention, rather than the literal `npm install`
  wording in the request. This is the only deviation from that wording; say so
  if the eval package should be npm-managed instead.
- Dependencies are `effect`, `@effect/platform`, `@effect/platform-bun`,
  `@effect/vitest` plus `vitest` for `it.effect`, and a YAML parser for
  `expected.yaml`. `@effect/vitest` requires the vitest runner rather than
  `bun test`, so the eval package's flake test derivation runs vitest; the
  existing `config-lib-bun-tests` derivation is untouched.
- Eval checks follow the existing split-derivation pattern
  (`config-lib-bun-tests`, `config-lib-biome-check`, `config-lib-biome-format`)
  rather than one combined derivation, per `context/patterns.md`.
- Case manifests are YAML (`expected.yaml`), as specified in the request.
- Model identifiers are read through `Config` as a per-harness map, defaulting
  to `gpt-5.6-sol` for Pi and OpenCode. Claude Code has no model named in the
  request, so it runs its own configured default; the resolved identifier is
  captured from the run's own event stream rather than assumed. Name the Claude
  model if you want it pinned.
- `evals/results/` is Git-ignored; only cases, fixtures, harness adapters,
  graders, and the contract profile are checked in.

## Task stack

- [ ] T01: `Scaffold the evals Effect package and wire its checks` (status:todo)
  - Task ID: T01
  - Goal: An `evals/` Bun/TypeScript package with Effect v4 installed, the
    module conventions established, shared schemas defined, and four flake
    check derivations green, with no runtime behavior implemented.
  - Boundaries (in/out of scope): In — `evals/package.json`, `evals/bun.lock`,
    `evals/tsconfig.json`, `evals/vitest.config.ts`; `evals/src/domain.ts`
    holding `Harness` (`Schema.Literal("claude", "pi", "opencode")`),
    `EvalInvocation`, `HarnessEvent` as a `Schema.TaggedUnion` over
    `Assistant`, `ToolStart`, `ToolResult`, `Usage`, `Error`, `Raw`,
    `Provenance`, `GradeResult`, and `EvalResult`, each `Schema.Struct` with a
    same-name `interface`; directory skeleton (`cases/`, `src/harness/`,
    `src/grader/`, `results/`); `evals/.gitignore` for `results/`; `biome.json`
    scope extension to `evals/**`; flake `evals-tests`, `evals-typecheck`,
    `evals-biome-check`, `evals-biome-format` derivations with vendored
    dependencies so the Nix sandbox needs no network. Out — any service, any
    harness invocation, any grader logic, any fixture.
  - Dependencies: none
  - Done when: `nix flake check` passes and reports the four new derivations;
    `cd evals && bun run test` passes on a schema round-trip test for
    `EvalResult`; `bun run typecheck` is clean; `nix develop -c biome check evals`
    is clean.
  - Verification notes (commands or checks): `nix flake check`;
    `cd evals && bun run test && bun run typecheck`;
    `nix develop -c biome check evals`.

- [ ] T02: `Implement the scoped Workspace service` (status:todo)
  - Task ID: T02
  - Goal: A `Workspace` service acquires a disposable fixture workspace inside
    a `Scope` and releases it unconditionally — copy fixture `repo/` to a temp
    dir, `git init`, commit a baseline, install the generated SCE payload for a
    named harness — and exposes capture of the final `git diff --binary` and
    the produced artifact file set.
  - Boundaries (in/out of scope): In — `evals/src/workspace.ts` with
    `Workspace.Service`, `Workspace.layer` built on `@effect/platform`
    `FileSystem` and `Command`, an `Effect.acquireRelease` temp-dir lifecycle,
    payload installation via `nix run .#pkl-generate -- <tmp>` plus the
    per-harness subtree copy (`config/.claude` → `.claude`, `config/.pi` →
    `.pi`, `config/.opencode` → `.opencode`), baseline-commit hash and fixture
    content hash computation, and `Workspace.SetupError` /
    `Workspace.CaptureError` as `Schema.TaggedErrorClass`. Out — spawning any
    harness, `agent-run`, event parsing, grading.
  - Dependencies: T01
  - Done when: an `it.effect` test acquires a workspace from a stub fixture,
    asserts the payload subtree is present and the repo has exactly one
    baseline commit, writes a file, asserts the captured diff contains it, and
    asserts the directory is gone after scope close; a second test interrupts
    the fiber mid-acquisition and asserts the same.
  - Verification notes (commands or checks): `cd evals && bun run test src/workspace.test.ts`;
    `nix flake check`.

- [ ] T03: `Add the sandboxed harness adapter services` (status:todo)
  - Task ID: T03
  - Goal: A `Harness` adapter interface with `claude`, `pi`, and `opencode`
    implementations builds the exact argv wrapped in
    `nix run github:sin-ack/agent-run --`, runs it with the workspace as cwd
    through `@effect/platform` `Command`, applies `Effect.timeout`, and yields
    exit code, timeout status, duration, and raw stdout/stderr.
  - Boundaries (in/out of scope): In —
    `evals/src/harness/{claude,pi,opencode}.ts` each exporting a
    `HarnessAdapter`, `evals/src/harness/registry.ts` exposing
    `HarnessRegistry.Service` with `adapter(name)` and a `layer` composing all
    three, `Harness.SpawnError` and `Harness.Timeout` typed errors,
    `evals/.agent-run/config.toml` with the per-tool mounts the README requires
    (`~/.claude` plus `~/.claude.json` for Claude, `~/.pi` and `/tmp` for Pi,
    `TMPDIR=/tmp/opencode` plus the OpenCode state dirs). Out — event decoding,
    usage extraction, grading.
  - Dependencies: T02
  - Done when: argv construction for all three harnesses is asserted under
    `it.effect`; a real sandboxed trivial command runs end to end; a
    deliberately hanging command fails with `Harness.Timeout` and leaves no
    orphan process.
  - Verification notes (commands or checks): `cd evals && bun run test src/harness`;
    manual `nix run github:sin-ack/agent-run -- claude --version` inside a
    scratch workspace.

- [ ] T04: `Decode harness output into one normalized result` (status:todo)
  - Task ID: T04
  - Goal: Each harness's stdout is consumed as a `Stream` of JSONL lines,
    decoded into `HarnessEvent` values through `Schema.decodeUnknownEffect`,
    and folded into normalized totals for input and output tokens, reported
    cost, tool-call count, and the assistant's terminal text.
  - Boundaries (in/out of scope): In — per-harness decoders under
    `evals/src/harness/`, a shared `Stream.runFold` reducer in
    `evals/src/normalize.ts`, checked-in recorded stream samples per harness
    under `evals/src/harness/__fixtures__/`, and tolerance for undecodable
    lines (emitted as the `Raw` variant, never fatal). Out — grading,
    provenance capture, running the suite.
  - Dependencies: T03
  - Done when: recorded samples for all three harnesses fold into the same
    normalized shape; terminal assistant text is extracted correctly for each;
    a malformed line becomes a `Raw` event without failing the stream.
  - Verification notes (commands or checks): `cd evals && bun run test src/normalize.test.ts src/harness`.

- [ ] T05: `Implement the contract profile and layered Grader service` (status:todo)
  - Task ID: T05
  - Goal: One checked-in contract profile encodes the canonical
    `/change-to-plan` structural rules, and a harness-agnostic `Grader` service
    applies layer 1 (conformance) and layer 2 (semantic coverage) from that
    profile plus a case's decoded `expected.yaml`, returning a per-layer
    pass/fail naming the exact assertion that failed.
  - Boundaries (in/out of scope): In —
    `evals/src/grader/contract-profile.ts` (required sections including
    `## Acceptance criteria`, task-line pattern `- [ ] T0N: ... (status:todo)`,
    sequential IDs, the no-trailing-validation-task rule),
    `evals/src/grader/structural.ts`, `evals/src/grader/semantic.ts`,
    `evals/src/grader/index.ts` with `Grader.Service` and `Grader.layer`, an
    `Expected` schema plus `Schema.decodeUnknownEffect` YAML loader,
    `Grader.ManifestError`, support for artifact cases and no-artifact negative
    cases, allowed-side-effect checking against the captured Git diff, and
    terminal-output pattern matching. Out — any harness-specific branch; any
    LLM judge; the case fixtures themselves.
  - Dependencies: T04
  - Done when: the profile asserts `## Acceptance criteria` and rejects a
    trailing validation-and-cleanup task, cross-checked against
    `config/pkl/base/workflow-change-to-plan.pkl:1190` and `:1302`; the grader
    passes a hand-written conforming plan, fails one missing a required
    section, fails one writing a file outside `side_effects.allowed`, fails one
    whose artifact omits a `required_fact`, fails one containing a
    `forbidden_fact`, and correctly grades both a conforming refusal and a
    non-conforming plan-writing run for a no-artifact case;
    `rg -n 'claude|opencode|\bpi\b' evals/src/grader/` returns no match.
  - Verification notes (commands or checks): `cd evals && bun run test src/grader`;
    `rg -n 'claude|opencode|\bpi\b' evals/src/grader/`.

- [ ] T06: `Add the eval runner CLI with repetitions, provenance, and reporting` (status:todo)
  - Task ID: T06
  - Goal: `bun run eval` selects cases by workflow or path, harnesses, and
    repetition count; runs each `(case, harness, repetition)` through the
    workspace scope, sandboxed spawn, normalization, and grader; writes one
    result JSON per run under `evals/results/`; and prints a summary matrix of
    pass rate and run-to-run variance per case per harness.
  - Boundaries (in/out of scope): In — `evals/src/cli.ts` and
    `evals/src/runner.ts` with `Runner.Service`, argument parsing, `Config`
    reads for model identifier and timeout, per-run provenance capture (harness
    version via the harness's own version command, resolved model identifier,
    generated payload hash, fixture hash, base commit, prompt, instruction
    files present in the workspace), result-file naming
    `{case}-{harness}-{repetition}.json`, and the summary report. A harness run
    that fails or times out is recorded as a failed run and does not abort the
    suite. Out — case fixtures; CI wiring; concurrent execution across runs.
  - Dependencies: T05
  - Done when: the runner executes a stub case against a `HarnessRegistry` test
    layer under `it.effect` and writes conforming result files with complete
    provenance; the summary matrix renders pass rate and variance; a stubbed
    non-zero exit is recorded as a failed run while the remaining runs
    complete.
  - Verification notes (commands or checks): `cd evals && bun run test src/runner.test.ts`;
    `jq -e '.provenance | .harnessVersion and .modelId and .payloadHash and .fixtureHash' evals/results/*.json`.

- [ ] T07: `Author the precise-request and constrained-request cases` (status:todo)
  - Task ID: T07
  - Goal: Two positive `/change-to-plan` cases exist, each with a minimal
    fixture repository carrying its own small `context/`, a `prompt.md`, and an
    `expected.yaml`, and both pass on at least one harness.
  - Boundaries (in/out of scope): In —
    `evals/cases/change-to-plan/precise-request/{repo,prompt.md,expected.yaml}`
    and `.../constrained-request/{repo,prompt.md,expected.yaml}`. The precise
    case asserts a conforming plan artifact plus a runnable `/next-task`
    handoff in the terminal output. The constrained case asserts the plan
    preserves the request's stated scope boundaries and that `forbidden_facts`
    covering invented adjacent work do not appear. Out — the ambiguous case;
    fixtures for other workflows.
  - Dependencies: T06
  - Done when: `bun run eval --case change-to-plan/precise-request --harness claude --repetitions 1`
    and the same for `constrained-request` both grade as pass, and each
    fixture's `context/` is small enough that the run stays well inside the
    configured timeout.
  - Verification notes (commands or checks): `cd evals && bun run eval --workflow change-to-plan --harness claude --repetitions 1`;
    inspect the written result JSON for the graded artifact path.

- [ ] T08: `Author the ambiguous-request negative case` (status:todo)
  - Task ID: T08
  - Goal: A `/change-to-plan` case whose prompt is genuinely undecidable
    passes only when the harness writes no plan file and emits categorized
    clarification questions, and fails when it writes a plan anyway.
  - Boundaries (in/out of scope): In —
    `evals/cases/change-to-plan/ambiguous-request/{repo,prompt.md,expected.yaml}`
    with an empty `side_effects.allowed`, and terminal-output patterns matching
    the clarification gate's `{question-id} · {category}` heading shape and its
    "No plan was written." line. Out — grader changes; the two positive cases.
  - Dependencies: T07
  - Done when: the case grades as pass against a run that correctly refuses,
    and grades as fail against a recorded run that writes a plan; the full
    suite (`--workflow change-to-plan --repetitions 3`) executes 27 runs and
    produces a summary matrix.
  - Verification notes (commands or checks): `cd evals && bun run eval --workflow change-to-plan --repetitions 3`;
    `ls evals/results/*.json | wc -l` reports 27.

## Open questions

- Effect plus `@effect/platform`, `@effect/platform-bun`, `@effect/vitest`,
  vitest, and a YAML parser is a meaningful dependency surface for a package
  whose job is to shell out to three binaries and diff text. The scoped
  workspace lifecycle and the boundary decoding genuinely earn it; the CLI and
  reporting probably do not. Worth naming now: if you would rather Effect stop
  at the workspace/stream/schema layer and keep the CLI plain TypeScript, that
  is a smaller T06.
- The full suite is 27 live agent runs of a planning workflow. That is real
  money on every invocation and no budget gate is planned. Worth adding a
  `--max-cost` abort to T06, or is a manually run suite acceptable?
- `agent-run` uses `bwrap` and is Linux-only. Any macOS contributor cannot run
  the suite at all, and `context/patterns.md` records an `ubuntu-latest` +
  `macos-latest` CI matrix. The plan accepts Linux-only; say so if that is
  wrong, because the alternative is a second unsandboxed execution path and
  that is a materially different design.
- Pinning Pi and OpenCode to `gpt-5.6-sol` while Claude Code runs a Claude
  model splits the matrix into two different comparisons, and it is worth being
  deliberate about that. Pi vs OpenCode is a clean harness-only comparison:
  same model, same prompt, so a divergence is attributable to how each harness
  presents the SCE payload. Claude Code vs either is harness *and* model, so a
  divergence there is not attributable to anything on its own. That is fine if
  the Claude column is read as "does the contract hold on its home target"
  rather than as a third data point in the same ranking — but the summary
  matrix will render all three side by side and invite the wrong reading. Say
  if you want the report to separate them.
- Three repetitions detects gross instability, not a rate. If a harness
  follows the clarification gate 80% of the time, three runs will not
  distinguish that from 100% or 67%. That is probably the right trade at this
  cost, but it is worth naming before you read the first summary matrix as a
  verdict.
