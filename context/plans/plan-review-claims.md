# Plan: plan-review-claims

## Change summary

This plan renames the hidden `sce plan decision-brief` command to the visible `sce plan review` command and adds a boolean `--claims` flag. Without the flag, `sce plan review` prints the existing Decision Brief Markdown. With the flag, it prints one `sce.plan.claims.v1` JSON document on stdout. The document holds one claim candidate per acceptance criterion in the supplied plans, with its linked decisions, declared verification, covering tasks, and source locations pinned to the `--head` commit.

The change extends the plan service from PR #342 and the completed `plan-decision-brief` plan. It reuses the `markdown-rs` extractor, the `validate` rules, the `parse_arguments` grammar, and the commit-backed Git reader in `cli/src/services/plan/command.rs`. A new pure module `cli/src/services/plan/claims.rs` maps validated `Plan` values to typed `serde` structures. The JSON states authored intent only. It contains no evidence, verdict, score, or priority, so a separate assessor such as Jev can attach judgments by `repository.slug`, `repository.head_sha`, and `claim.id`. The command never writes files, never runs `Validate:` or `Check:` content, and never calls a model or the network. The `decision-brief` name is removed without an alias. `sce plan pr-description` does not change.

## Design decisions

- D1: `sce plan review` replaces `sce plan decision-brief` as a visible subcommand with no alias. `PlanRequest::Review(PlanReviewRequest)` carries `repo`, `head`, `paths`, and a `ReviewOutput::{Brief, Claims}` enum. `services/parse/command_runtime.rs` maps the boolean `--claims` Clap flag to that enum, and `command.rs` dispatches on it with a static `match`.
  - Boundary: `cli/src/cli_schema.rs` owns the `review` subcommand and its `--claims` flag. `cli/src/command_surface.rs` owns the `Plan Usage:` line. `command.rs` owns dispatch. `pr_description.rs` keeps the Decision Brief layout, its three-question limit, and its empty-statement check, and the `Brief` output uses them unchanged.
  - Rationale: The user asked for one command with two output modes and a boolean selector. An enum inside the request follows the repository's static enum-dispatch convention and makes the mode explicit in tests.
  - Rejected: A `--format json` option or a `--claims <path>` file argument — the request forbids both. A kept `decision-brief` alias — the command is pre-alpha and the request removes the name. A second subcommand such as `sce plan claims` — it splits one review surface into two commands. A trait or `dyn` output renderer — the repository uses enums for static dispatch.
  - Trade-offs: Scripts that call `sce plan decision-brief` break: the existing unknown-subcommand routing in `command_runtime.rs` prints the `sce plan --help` text on stdout with exit `0` and generates no Decision Brief, so a script cannot detect the removal from the exit code. The boolean flag cannot select a third output later without a new flag or a breaking change.
- D2: `cli/src/services/plan/claims.rs` is a pure module with no I/O. It owns the typed `Serialize` structures in contract field order, the mapping from validated `PlanInput` values to claims, the reference resolution inside each plan, the rejection of repeated consumed fields described in `## Claims JSON v1 contract`, and `serde_json::to_string_pretty` serialization.
  - Boundary: `claims.rs` reads only `Plan.entries`, `PlanEntry.statement`, `PlanEntry.checked`, `PlanEntry.line`, `PlanField.key`, `PlanField.value`, and `PlanField.line`. It never parses Markdown, never calls `render_format`, and never applies `escape_markdown`. It never picks one occurrence of a repeated consumed field; it returns `CliError::validation` instead, also when called directly with a `Plan` that `validate` accepts. `command.rs` owns Git I/O and passes validated plans to `claims.rs`. `validate.rs` and `sce plan check` keep their current rules.
  - Rationale: The extractor and validator already own plan structure. A separate pure module keeps JSON concerns out of the Markdown renderer, so the three-question limit and the Markdown escaping cannot leak into the claim manifest. Typed structures fix the field order without a map.
  - Rejected: Adding a `BriefFormat::Claims` variant to `render_format` — that pass applies the question limit and the Markdown escaping, which the JSON contract forbids. Building `serde_json::Value` maps by hand — the field order is then not enforced by types. A second regex or line scanner for fields — it duplicates `extract.rs`. Taking the first occurrence of a repeated field — a second `Covers:` or `Scenarios:` line would silently vanish from the manifest. Adding the repetition rule to `validate.rs` — it would change `sce plan check` for every plan, which this change does not intend.
  - Trade-offs: The plan service now has two output modules that both read decisions. A plan that `sce plan check` accepts can still fail `--claims` when it repeats a consumed field. Any new plan field that claims need requires a change to the typed structures and a new schema version when the change is incompatible.
- D3: `--claims` reads every plan from the `--head` commit through the existing `verify_commit` and `read_committed_plan` path. Every plan passes `parse_valid_plan` before `claims.rs` runs, and the command returns the complete serialized document only after all plans are validated and mapped.
  - Boundary: `command.rs` reuses `parse_arguments`, `resolve_repository_root`, `verify_commit`, `read_committed_plan`, and `parse_valid_plan` for both outputs. The shared stdout writer appends the single trailing newline. No other code path writes to stdout.
  - Rationale: Source links name `--head`, so the parsed text and line numbers must come from `--head`. Building the full document before return means a failure leaves stdout empty, because the runtime prints a payload only on success.
  - Rejected: Reading the working tree for `--claims` — line anchors then disagree with the commit. Streaming claims to stdout per plan — a later failure leaves a partial document.
  - Trade-offs: The commit must exist locally, and uncommitted plan edits are invisible to the manifest. Each plan costs Git process starts. As with the Decision Brief, the command does not check that the commit exists in the named GitHub repository, so links can be well-formed but unresolvable.
- D4: A claim is one acceptance criterion, identified as `<plan-path>#<AC-id>`, and every v1 claim records authored intent only. `kind` is `acceptance_criterion`, `modality` is `intended`, and `author_checked` copies `PlanEntry.checked`. The schema has no evidence, verdict, confidence, risk, or priority field.
  - Boundary: `claims.rs` owns the identity rule and the `sce.plan.claims.v1` literal. External assessors own every judgment and join on `repository.slug`, `repository.head_sha`, and `claim.id`. Checkbox state, `(status:done)`, and validation reports never change `modality`.
  - Rationale: The path and AC ID stay stable across text edits and commits, because plan rules never renumber or reuse AC IDs. Keeping judgments outside the manifest keeps the authored source authoritative and lets an assessor change without a schema change.
  - Rejected: An ID that includes the commit SHA or line number — every commit or edit changes it. A content-hash ID — any wording fix changes it. A `verified` modality from checked boxes — a checkbox is not execution evidence.
  - Trade-offs: Moving a criterion to another plan or renaming a plan file changes its ID. An AC that holds several assertions stays one claim until a future atomization step exists.
  - Review: required
  - Review question: Is `<plan-path>#<AC-id>` an acceptable stable claim identity when a plan rename or a moved criterion changes it?

## Claims JSON v1 contract

`sce plan review --claims` prints exactly one JSON document on stdout. This section is the complete `sce.plan.claims.v1` contract. Every object has exactly the listed properties, in the listed order, and no other property. No property is optional, and no property is omitted when its value is empty.

The document separates two things. **Authored claims and declared verification procedures** are what the plan author wrote: acceptance criteria, design decisions, `Validate:` commands, scenarios, and `Check:` commands. **Observed execution evidence** is what happens when someone runs those procedures: exit codes, logs, test results, model judgments. v1 carries only the first kind. `validate` and `check` are declared procedures copied as text. The command never runs them, and their presence never means that they passed.

### Types

Root:

- `schema_version`: string, always `"sce.plan.claims.v1"`
- `repository`: Repository
- `plans`: Plan[]
- `claims`: Claim[]

Repository:

- `slug`: string, the validated `--repo` value `<owner/name>`
- `head_sha`: string, the validated `--head` value

Plan:

- `path`: string, the repository-relative plan path as validated by `parse_arguments`
- `url`: string, `https://github.com/{slug}/blob/{head_sha}/{path}`

Claim:

- `id`: string, `<plan-path>#<AC-id>`
- `kind`: string, always `"acceptance_criterion"`
- `statement`: string
- `modality`: string, always `"intended"`
- `author_checked`: boolean or null
- `source`: Source
- `decisions`: Decision[]
- `verification`: Verification
- `tasks`: Task[]

Source:

- `path`: string, the plan path
- `entry_id`: string, the ID of the entry the source points at
- `line`: positive integer, the 1-based `PlanEntry.line` of that entry in the `--head` content
- `url`: string, `https://github.com/{slug}/blob/{head_sha}/{path}#L{line}`

Decision:

- `id`: string
- `statement`: string
- `boundary`: string, the `Boundary:` value
- `rationale`: string, the `Rationale:` value
- `rejected`: string, the `Rejected:` value
- `trade_offs`: string, the `Trade-offs:` value
- `review`: Review
- `source`: Source

Review:

- `required`: boolean, `true` when the decision carries `Review: required`, otherwise `false`
- `question`: string or null, the `Review question:` value when `required` is `true`, otherwise `null`

Verification:

- `mode`: string, `"scenarios"` or `"inspection"`
- `validate`: string, the `Validate:` value
- `scenarios`: Scenario[]

Scenario:

- `id`: string
- `statement`: string
- `given`: string, the `Given:` value
- `when`: string, the `When:` value
- `then`: string, the `Then:` value
- `check`: string, the `Check:` value
- `source`: Source

Task:

- `id`: string
- `statement`: string
- `author_checked`: boolean or null
- `source`: Source

### Mapping rules

- Each acceptance criterion of each supplied plan gives exactly one claim. Nothing else gives a claim.
- A claim `id` is the plan path, `#`, and the AC ID, for example `context/plans/token-refresh.md#AC1`. It contains no SHA and no line number.
- Every `statement` is the whitespace-folded `PlanEntry.statement`, copied without Markdown escaping. It keeps inline backticks and a task's `(status:...)` suffix as authored. A marked decision with an empty statement gives `"statement": ""`.
- Every other string value is the authored `PlanField.value`, copied without Markdown escaping, folding, or trimming beyond what the extractor applies. A wrapped value keeps its embedded newline, and `serde_json` escapes it as `\n`.
- `author_checked` is `true` for `[x]`, `false` for `[ ]`, and `null` when the entry has no checkbox, copied from `PlanEntry.checked`. It never counts as verification evidence and never changes `modality`.
- Every `line` comes from the plan content committed at `--head`, and every `url` names `head_sha`, never a branch or the working tree.
- `decisions` follows the order of the IDs in the claim's `Decisions:` value. A claim without `Decisions:` gives `"decisions": []`.
- `Scenarios: none (inspection)` gives `"mode": "inspection"` and `"scenarios": []`. Any other `Scenarios:` value gives `"mode": "scenarios"` and the referenced scenarios in reference order.
- `tasks` lists every task whose `Covers:` value names the acceptance criterion, in task source order. A criterion that no task covers gives `"tasks": []`.
- `plans` is sorted by path in byte order. `claims` is sorted by plan path, then by acceptance-criterion source order inside the plan. Input path order never changes the output.
- References resolve only inside their own plan.
- The document has no evidence, assessment, verdict, confidence, risk, score, or priority property. External assessors join their own data on `repository.slug`, `repository.head_sha`, and `claims[].id`.
- `#[derive(Serialize)]` structs in `claims.rs` fix the property order. No `serde_json::Value` map and no `skip_serializing_if` attribute is used.
- `serde_json::to_string_pretty` serializes the document with two-space indentation. The shared stdout writer appends one `\n`, so stdout ends with exactly one newline.
- The command creates no file. `claims.json` names the stdout document only, for example after a shell redirection.

### Ambiguous field rejection

A structurally valid plan can repeat a field on one entry, for example two `Covers:` lines on one task. Choosing one occurrence would silently drop a relationship or authored text from the manifest, so `claims.rs` rejects these repetitions instead.

- The consumed fields are `Boundary`, `Rationale`, `Rejected`, `Trade-offs`, `Review`, and `Review question` on decisions, `Validate`, `Decisions`, and `Scenarios` on acceptance criteria, `Given`, `When`, `Then`, and `Check` on scenarios, and `Covers` on tasks.
- `validate` already rejects a repeated `Review:` or `Review question:` with `` `<key>:` appears more than once on `<id>` ``. `--claims` keeps that diagnostic unchanged, because every plan passes `parse_valid_plan` before `claims.rs` runs.
- `claims.rs` checks the remaining consumed fields itself, on every plan it receives, also when a caller passes it a `Plan` directly. For each entry and each key that occurs more than once, it reports one diagnostic at the line of the second occurrence with the message `` `<key>:` appears more than once on `<id>` ``, for example `` `Covers:` appears more than once on `T01` ``.
- `claims.rs` checks plans in sorted path order and stops at the first plan with a repetition. It returns `CliError::validation` (exit `3`) with this text, listing that plan's diagnostics in line order:

  ```text
  Plan claims check failed for '<path>':
  <path>:<line>: `<key>:` appears more than once on `<id>`
  Try: merge each repeated field into one line, then rerun 'sce plan review --claims'.
  ```

- On this failure stdout is empty, no partial JSON is printed, and no file is written.
- `sce plan check`, `validate.rs`, and the Decision Brief output keep accepting these repetitions. The rejection applies only to claims generation.

### Golden example

The plan `context/plans/token-refresh.md`, committed at `4f2a9c1e8b7d6053a1c2e3f4a5b6c7d8e9f01234`, passes `sce plan check`. Its D1, AC1, S1, and T01 definitions start on lines 9, 19, 26, and 34:

```markdown
# Plan: token-refresh

## Change summary

Add a refresh endpoint.

## Design decisions

- D1: The session store validates refresh tokens.
  - Boundary: `src/auth/` owns token validation.
  - Rationale: The store already tracks revocation.
  - Rejected: Stateless tokens — they cannot be revoked.
  - Trade-offs: One store read per refresh.
  - Review: required
  - Review question: Is one store read per refresh acceptable?

## Acceptance criteria

- [ ] AC1: `POST /auth/refresh` rejects a revoked token with 401.
  - Validate: `pnpm test src/auth/refresh.test.ts`
  - Decisions: D1
  - Scenarios: S1

### Verification scenarios

- S1: Revoked refresh token is rejected
  - Given: A revoked refresh token.
  - When: The client calls `POST /auth/refresh`.
  - Then: The response is 401.
  - Check: `pnpm test src/auth/refresh.test.ts -t "rejects a revoked token"`

## Task stack

- [x] T01: `Add /auth/refresh endpoint` (status:done)
  - Task ID: T01
  - Covers: AC1
  - Scope: In — route handler. Out — rotation policy.
  - Dependencies: none
  - Done when: The endpoint returns 401 for a revoked token.
  - Verify: `pnpm test src/auth/refresh.test.ts`
  - Context synchronization: synced

## Open questions

None.
```

`sce plan review --claims --repo crocoder-dev/example --head 4f2a9c1e8b7d6053a1c2e3f4a5b6c7d8e9f01234 context/plans/token-refresh.md` prints this document, followed by one newline:

```json
{
  "schema_version": "sce.plan.claims.v1",
  "repository": {
    "slug": "crocoder-dev/example",
    "head_sha": "4f2a9c1e8b7d6053a1c2e3f4a5b6c7d8e9f01234"
  },
  "plans": [
    {
      "path": "context/plans/token-refresh.md",
      "url": "https://github.com/crocoder-dev/example/blob/4f2a9c1e8b7d6053a1c2e3f4a5b6c7d8e9f01234/context/plans/token-refresh.md"
    }
  ],
  "claims": [
    {
      "id": "context/plans/token-refresh.md#AC1",
      "kind": "acceptance_criterion",
      "statement": "`POST /auth/refresh` rejects a revoked token with 401.",
      "modality": "intended",
      "author_checked": false,
      "source": {
        "path": "context/plans/token-refresh.md",
        "entry_id": "AC1",
        "line": 19,
        "url": "https://github.com/crocoder-dev/example/blob/4f2a9c1e8b7d6053a1c2e3f4a5b6c7d8e9f01234/context/plans/token-refresh.md#L19"
      },
      "decisions": [
        {
          "id": "D1",
          "statement": "The session store validates refresh tokens.",
          "boundary": "`src/auth/` owns token validation.",
          "rationale": "The store already tracks revocation.",
          "rejected": "Stateless tokens — they cannot be revoked.",
          "trade_offs": "One store read per refresh.",
          "review": {
            "required": true,
            "question": "Is one store read per refresh acceptable?"
          },
          "source": {
            "path": "context/plans/token-refresh.md",
            "entry_id": "D1",
            "line": 9,
            "url": "https://github.com/crocoder-dev/example/blob/4f2a9c1e8b7d6053a1c2e3f4a5b6c7d8e9f01234/context/plans/token-refresh.md#L9"
          }
        }
      ],
      "verification": {
        "mode": "scenarios",
        "validate": "`pnpm test src/auth/refresh.test.ts`",
        "scenarios": [
          {
            "id": "S1",
            "statement": "Revoked refresh token is rejected",
            "given": "A revoked refresh token.",
            "when": "The client calls `POST /auth/refresh`.",
            "then": "The response is 401.",
            "check": "`pnpm test src/auth/refresh.test.ts -t \"rejects a revoked token\"`",
            "source": {
              "path": "context/plans/token-refresh.md",
              "entry_id": "S1",
              "line": 26,
              "url": "https://github.com/crocoder-dev/example/blob/4f2a9c1e8b7d6053a1c2e3f4a5b6c7d8e9f01234/context/plans/token-refresh.md#L26"
            }
          }
        ]
      },
      "tasks": [
        {
          "id": "T01",
          "statement": "`Add /auth/refresh endpoint` (status:done)",
          "author_checked": true,
          "source": {
            "path": "context/plans/token-refresh.md",
            "entry_id": "T01",
            "line": 34,
            "url": "https://github.com/crocoder-dev/example/blob/4f2a9c1e8b7d6053a1c2e3f4a5b6c7d8e9f01234/context/plans/token-refresh.md#L34"
          }
        }
      ]
    }
  ]
}
```

## Acceptance criteria

How this plan is proven complete. Each criterion is observable and names the
check that proves it. `/validate` runs these checks; no task in the stack
performs final validation.

- [ ] AC1: `sce plan review --repo <owner/name> --head <sha> <path>...` prints the Decision Brief Markdown that `sce plan decision-brief` printed for the same commit and plans, with stdout ending in exactly one newline. `decision-brief` is no longer a recognized `plan` subcommand: `sce plan decision-brief --repo <owner/name> --head <sha> <path>...` follows the existing unknown-subcommand routing in `command_runtime.rs`, exits `0`, prints on stdout exactly the bytes that `sce plan --help` prints, leaves stderr empty, and generates no Decision Brief. The three-question limit and the empty-statement check still apply to the `review` output, so `sce plan review` over plans with more than three marked decisions exits `3` with empty stdout.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::review_` and `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::decision_brief_`
  - Decisions: D1
  - Scenarios: S1, S2
- [ ] AC2: `sce --help`, `sce plan --help`, and the Bash, Zsh, and Fish completion scripts expose `review` and its `--claims` flag, and none of them contains `decision-brief`. The `Plan Usage:` section lists `sce plan review [--claims] --repo <owner/name> --head <sha> <path>...`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::review_appears_in_help_and_completions`
  - Decisions: D1
  - Scenarios: S3
- [ ] AC3: `sce plan review --claims` prints one `sce.plan.claims.v1` JSON document that follows the `## Claims JSON v1 contract` section exactly: the Root, Repository, Plan, Claim, Source, Decision, Review, Verification, Scenario, and Task objects have exactly the listed properties, types, and order, and the document is pretty-printed with two-space indentation. For the golden example plan and arguments in that section, the output is byte-identical to the golden JSON document. Statements and field values keep their authored text without Markdown escaping.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::claims_`
  - Decisions: D2, D4
  - Scenarios: S4
- [ ] AC4: Each claim resolves its `Decisions:` and `Scenarios:` references in reference order and lists its covering tasks in task source order. A claim without `Decisions:` has `"decisions": []`. A decision without a review pair has `"review": {"required": false, "question": null}`. `Scenarios: none (inspection)` gives `"mode": "inspection"` and `"scenarios": []`. `validate` is always the authored `Validate:` value. When an entry repeats a consumed field listed in `### Ambiguous field rejection`, the `claims.rs` render function returns a validation error naming the entry ID, the repeated field, and the line of the second occurrence, and returns no JSON, even when it is called directly with a plan that `validate` accepts.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::claims_`
  - Decisions: D2
  - Scenarios: S5, S6, S7, S14
- [ ] AC5: The manifest records intent only. Every claim has `"kind": "acceptance_criterion"` and `"modality": "intended"`. `author_checked` is `true`, `false`, or `null` exactly as the checkbox is checked, unchecked, or absent, for claims and tasks alike. The document contains no evidence, verdict, confidence, risk, or priority field, whatever the checkbox or task status.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::claims_`
  - Decisions: D4
  - Scenarios: S8
- [ ] AC6: `--claims` includes every acceptance criterion of every supplied plan, also when the plans hold more than three marked decisions or a marked decision with an empty statement. Valid plans without acceptance criteria give `"claims": []`.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::claims_`
  - Decisions: D2
  - Scenarios: S9
- [ ] AC7: `--claims` reads plans only from the `--head` commit. The same repository, SHA, and plan set give byte-identical stdout for every path order and after an uncommitted working-tree edit. On success, stdout is one valid JSON document followed by exactly one newline, stderr is empty, and `git status --porcelain` is unchanged.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::review_claims_`
  - Decisions: D3
  - Scenarios: S10, S11
- [ ] AC8: `--claims` fails with exit `3` for invalid `--repo`, `--head`, or plan-path values, a duplicate path, an invalid plan, and a committed plan that `sce plan check` accepts but that repeats a consumed field, and with exit `4` for a missing commit, a missing path, a symlink, non-UTF-8 content, and a failed `git`. Every failure leaves stdout empty, prints a diagnostic on stderr, and writes no file.
  - Validate: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::review_claims_rejects_invalid_inputs`
  - Decisions: D3
  - Scenarios: S12
- [ ] AC9: `sce plan review --claims` over the committed plans `context/plans/sce-plan-init.md` and `context/plans/plan-pr-description.md` at the `--head` commit exits `0` and prints valid JSON with one claim per acceptance criterion of those plans.
  - Validate: `nix run .#sce -- plan review --claims --repo crocoder-dev/shared-context-engineering --head "$(git rev-parse HEAD)" context/plans/sce-plan-init.md context/plans/plan-pr-description.md | nix shell nixpkgs#jq -c jq -e '.schema_version == "sce.plan.claims.v1" and (.claims | length) > 0'`
  - Decisions: D3, D4
  - Scenarios: S13
- [ ] AC10: `context/cli/plan-service.md` and `context/cli/cli-command-surface.md` document `sce plan review`, `sce plan review --claims`, the exact v1 JSON contract, the stable claim identity, commit-backed provenance, and the difference between intended claims, declared checks, and observed evidence. They state that Jev and other semantic evaluators are separate consumers and that `claims.json` names the stdout document, not a file the command creates.
  - Validate: inspection of the `## Review command contract` and `## Claims manifest contract` sections in `context/cli/plan-service.md` and the `plan` paragraph in `context/cli/cli-command-surface.md`.
  - Scenarios: none (inspection)

### Verification scenarios

- S1: Review prints the Decision Brief from the commit
  - Given: A temporary Git repository with a committed valid plan that marks one decision.
  - When: The test runs `sce plan review --repo o/r --head <sha> <path>` and then edits the plan in the working tree and runs it again.
  - Then: Both runs exit `0` with the exact Decision Brief stdout, ending in one newline, and the outputs are byte-identical.
  - Check: `plan::review_reads_committed_plan`, renamed from `decision_brief_reads_committed_plan` by T01.
- S2: The removed name routes to plan help and the brief limit stays
  - Given: A temporary Git repository with committed plans holding four marked decisions.
  - When: The test runs `sce plan --help`, then `sce plan decision-brief --repo o/r --head <sha> <paths>`, then `sce plan review --repo o/r --head <sha> <paths>` without `--claims`, all through the same argument parser and `render_run_outcome` path.
  - Then: `decision-brief` exits `0` with stdout byte-identical to the `sce plan --help` stdout, empty stderr, and no `## Decision Brief` text, and `review` exits `3` with the shared limit error on stderr and empty stdout.
  - Check: `plan::review_removed_name_shows_plan_help_and_keeps_question_limit`, created by T01.
- S3: Help and completions expose review and the claims flag
  - Given: The built CLI.
  - When: The test renders `sce --help`, `sce plan --help`, `sce plan review --help`, and the Bash, Zsh, and Fish completion scripts.
  - Then: Each output contains `review`, the review help and every completion script contain `--claims`, the root help contains the new `Plan Usage:` line, and no output contains `decision-brief`.
  - Check: `plan::review_appears_in_help_and_completions`, created by T01 and extended with `--claims` by T03.
- S4: Golden JSON for one of each definition
  - Given: The `context/plans/token-refresh.md` plan source from `### Golden example`, extracted in memory.
  - When: The test calls the `claims.rs` render function with `--repo crocoder-dev/example`, `--head 4f2a9c1e8b7d6053a1c2e3f4a5b6c7d8e9f01234`, and that plan path.
  - Then: The output equals the golden JSON document from `### Golden example` byte for byte, without the trailing newline that the stdout writer adds.
  - Check: `plan::claims_golden_one_of_each`, created by T02.
- S5: Multiple references keep reference order
  - Given: An acceptance criterion with `Decisions: D2, D1` and `Scenarios: S2, S1`, and tasks T02 and T01 that both cover it.
  - When: The test renders the claims.
  - Then: The claim lists decisions D2, D1 and scenarios S2, S1 with their own statements, fields, and lines, and lists tasks T01, T02 in source order.
  - Check: `plan::claims_preserve_reference_order`, created by T02.
- S6: Inspection-only criterion
  - Given: An acceptance criterion with `Scenarios: none (inspection)` and an authored `Validate:` value.
  - When: The test renders the claims.
  - Then: Its `verification` is `mode` `inspection`, `validate` equal to the authored value, and an empty `scenarios` array.
  - Check: `plan::claims_inspection_only_criterion`, created by T02.
- S7: Unmarked and absent decisions
  - Given: One acceptance criterion that references a decision without a review pair, and one acceptance criterion without `Decisions:`.
  - When: The test renders the claims.
  - Then: The first claim's decision has `"required": false` and `"question": null`, and the second claim has `"decisions": []`.
  - Check: `plan::claims_unmarked_and_absent_decisions`, created by T02.
- S8: Checked entries stay intended
  - Given: Acceptance criteria and tasks that are checked, unchecked, and without a checkbox, with a task marked `(status:done)`.
  - When: The test renders the claims.
  - Then: `author_checked` is `true`, `false`, and `null` respectively, every claim has `"modality": "intended"`, and no key outside the v1 contract appears in the parsed document.
  - Check: `plan::claims_checked_entries_stay_intended`, created by T02.
- S9: No limit and no criteria
  - Given: Plans with four marked decisions and five acceptance criteria, and a separate valid plan without acceptance criteria.
  - When: The test renders the claims for each input.
  - Then: The first output holds all five claims, and the second output holds `"claims": []`.
  - Check: `plan::claims_include_every_criterion`, created by T02.
- S10: Commit-backed and order-independent output
  - Given: A temporary Git repository with two committed valid plans.
  - When: The test runs `sce plan review --claims` with both path orders, then edits one plan in the working tree and runs it again.
  - Then: All runs give byte-identical stdout whose `plans` are sorted by path and whose source lines match the committed content.
  - Check: `plan::review_claims_reads_committed_plans`, created by T03.
- S11: Output purity
  - Given: A temporary Git repository with a committed valid plan and a recorded `git status --porcelain`.
  - When: The test runs `sce plan review --claims`.
  - Then: Stdout parses as one JSON value and ends in `\n` but not `\n\n`, stderr is empty, and `git status --porcelain` is unchanged.
  - Check: `plan::review_claims_output_is_pure`, created by T03.
- S12: Invalid inputs fail with empty stdout
  - Given: Temporary Git repositories with invalid arguments, a duplicate path, a malformed plan, a committed plan that `sce plan check` accepts but whose `T01` has `Covers: AC1` on one line and `Covers: AC2` on the next, a missing commit, a missing path, a symlink, and a non-UTF-8 blob.
  - When: The test runs `sce plan review --claims` for each case.
  - Then: Argument, duplicate-path, plan, and repeated-field defects exit `3`, Git and content defects exit `4`, stdout is empty, stderr names the defect, and `git status --porcelain` is unchanged. For the repeated `Covers:` case, stderr is exactly the `Plan claims check failed` text from `### Ambiguous field rejection` with `` `Covers:` appears more than once on `T01` `` at the line of the second `Covers:`.
  - Check: `plan::review_claims_rejects_invalid_inputs`, created by T03.
- S13: Dogfood over committed repository plans
  - Given: The repository at a commit that contains `context/plans/sce-plan-init.md` and `context/plans/plan-pr-description.md`.
  - When: A user runs the AC9 command.
  - Then: It exits `0` and `jq -e` confirms the schema version and a nonempty `claims` array.
  - Check: the AC9 `Validate:` command.
- S14: Repeated consumed fields are rejected, not truncated
  - Given: Plans that `validate` accepts with no diagnostics: one whose task has `- [ ] T01: Implement the feature`, `Task ID: T01`, `Covers: AC1`, and `Covers: AC2` on separate lines; one whose acceptance criterion repeats `Scenarios:`; one whose acceptance criterion repeats `Decisions:`; and one whose decision repeats `Rationale:`.
  - When: The test calls the `claims.rs` render function directly for each plan.
  - Then: Each call returns a validation error with exit class `3` and the exact `Plan claims check failed` text, naming `` `Covers:` appears more than once on `T01` ``, `` `Scenarios:` appears more than once on `AC1` ``, `` `Decisions:` appears more than once on `AC1` ``, and `` `Rationale:` appears more than once on `D1` `` with the line of each second occurrence, and no call returns JSON or a `tasks` array that omits `AC2` coverage.
  - Check: `plan::claims_reject_repeated_consumed_fields`, created by T02.

### Full validation

Repository-wide checks `/validate` runs after the last task, regardless of
which criterion they map to.

- `nix flake check`
- `nix run .#pkl-check-generated`

### Context sync

- `context/cli/plan-service.md`
- `context/cli/cli-command-surface.md`
- `context/context-map.md` (the `plan-service.md` annotation)
- `context/glossary.md`, when it defines `decision-brief` terms

## Task context synchronization lifecycle

Persist this field in every plan; this is durable plan state, not chat state:

- **Task context synchronization:** every task carries `pending | synced | blocked`.
  A completed task must be `synced` before another task can start or the plan can
  finish.
- For `blocked`, record **Blocker**, **Required action**, and **Retry condition**
  beside the status. Never infer `synced` from conversation history; write every
  lifecycle transition to the plan file.

## Constraints and non-goals

- **In scope:** `cli/src/cli_schema.rs`, `cli/src/command_surface.rs`, `cli/src/services/parse/command_runtime.rs`, `cli/src/services/plan/command.rs`, a new `cli/src/services/plan/claims.rs`, `cli/src/services/plan/mod.rs` tests, `cli/src/services/completion/mod.rs` when its tests or pruning need a change, and the plan-service and command-surface context files.
- **Out of scope:** `sce plan pr-description` behavior, the `extract.rs` grammar, the `validate.rs` rules, the Decision Brief layout text, Jev or any other assessor, and any execution of `Validate:` or `Check:` content.
- **Constraints:** Use `serde` and `serde_json` already in `cli/Cargo.toml`. Add no dependency, async runtime, database access, network call, model call, trait object, or file-writing code. Run Cargo only through `nix develop -c ./scripts/run-cli-cargo.sh`.
- **Non-goal:** Semantic atomization of acceptance criteria, claim assessment, risk or priority ranking, a `--format` option, an output-file option, and a `decision-brief` alias.

## Assumptions

- `validate`, `given`, `when`, `then`, `check`, `boundary`, `rationale`, `rejected`, `trade_offs`, and `review.question` use the verbatim `PlanField.value` source slice, the same "exact authored content" that the brief reads. A wrapped value keeps its line break, and `serde_json` escapes it.
- `--claims` emits a marked decision with an empty statement as `"statement": ""`, because the empty-statement check belongs to the Decision Brief layout only.
- The Decision Brief keeps its `## Decision Brief` heading and its `decision_brief_*` render-level test names. Only the command-level tests move to `review_*` names.
- The `plan` completion pruning of hidden subcommands stays in place, although no `plan` subcommand stays hidden after the rename.

## Task stack

- [x] T01: `Rename plan decision-brief to the visible plan review command` (status:done)
  - Task ID: T01
  - Covers: AC1, AC2
  - Scope: In — rename `PlanSubcommand::DecisionBrief` to a visible `Review` subcommand named `review`, rename `PlanRequest::DecisionBrief` to `PlanRequest::Review(PlanReviewRequest)` with `ReviewOutput::Brief` only, update the command name constant and its remediation text to `sce plan review`, add the `Plan Usage:` line without `[--claims]`, rename the command-level `decision_brief_*` tests to `review_*`, replace the hidden-command tests with `review_appears_in_help_and_completions`, and add `review_removed_name_shows_plan_help_and_keeps_question_limit`, which compares the stdout, stderr, and exit code of `sce plan decision-brief ...` with those of `sce plan --help`. Out — the `--claims` flag, `claims.rs`, context files, and any change to the unknown-subcommand routing in `command_runtime.rs`.
  - Dependencies: none
  - Done when: `sce plan review` prints the unchanged Decision Brief, `sce plan decision-brief ...` exits `0` with the exact `sce plan --help` stdout, empty stderr, and no Decision Brief, `review` still exits `3` with empty stdout above three marked decisions, `review` appears in help and all three completion scripts, and the renamed and new tests pass.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::`.
  - Completed: 2026-10-25
  - Files changed: `cli/src/cli_schema.rs`, `cli/src/command_surface.rs`, `cli/src/services/parse/command_runtime.rs`, `cli/src/services/plan/command.rs`, `cli/src/services/plan/mod.rs`
  - Result: `PlanSubcommand::Review` (`name = "review"`, visible) replaces the hidden `DecisionBrief` variant. `PlanRequest::Review(PlanReviewRequest)` carries `repo`, `head`, `paths`, and `output: ReviewOutput`, where `ReviewOutput::Brief` is the only variant, and `command.rs` dispatches on it with a static `match`. `REVIEW_COMMAND = "sce plan review"` names the command in the remediation text. `Plan Usage:` lists `sce plan review --repo <owner/name> --head <sha> <path>...`. The command-level tests are renamed to `review_reads_committed_plan`, `review_question_limit`, and `review_rejects_invalid_git_inputs`. `review_appears_in_help_and_completions` replaces the two hidden-command tests. The new `review_removed_name_shows_plan_help_and_keeps_question_limit` uses a new `run_cli_args` test helper that renders help-text and plan commands through `render_run_outcome`. The unknown-subcommand routing and the completion pruning are unchanged.
  - Verify result: passed — `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::` (50 passed, 0 failed). Additional: the full CLI test suite (660 passed) and `clippy --all-targets -D warnings` (clean).
  - Context impact: the `plan` command surface changed — `sce plan decision-brief` is now the visible `sce plan review`, without an alias. `context/cli/plan-service.md` and `context/cli/cli-command-surface.md` still describe the hidden `decision-brief`, and T04 is planned to rewrite those sections.
  - Context synchronization: synced

- [ ] T02: `Add the pure sce.plan.claims.v1 claims module` (status:todo)
  - Task ID: T02
  - Covers: AC3, AC4, AC5, AC6
  - Scope: In — `cli/src/services/plan/claims.rs` with the typed `Serialize` structures of `## Claims JSON v1 contract`, the mapping from `PrDescriptionArguments` and `PlanInput` values under its mapping rules, in-plan reference and task resolution, sorted plans, the repeated-consumed-field rejection of `### Ambiguous field rejection`, `to_string_pretty` serialization, its registration in `mod.rs`, and the in-memory `claims_*` tests for S4 to S9 and S14, with the `### Golden example` plan and JSON as the S4 fixture and expected output. Out — CLI flag, Git I/O, dispatch, context files, and any change to `validate.rs`.
  - Dependencies: T01
  - Done when: The `claims.rs` render function returns the `### Golden example` JSON byte for byte for S4, the reference-order, inspection, unmarked-decision, intent-only, and no-limit tests pass without any call to `render_format` or `escape_markdown`, and repeated `Covers:`, `Scenarios:`, `Decisions:`, and `Rationale:` fields return the exact validation error instead of JSON.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::claims_`.
  - Context synchronization: pending

- [ ] T03: `Wire the --claims flag into sce plan review` (status:todo)
  - Task ID: T03
  - Covers: AC2, AC7, AC8, AC9
  - Scope: In — the boolean `--claims` Clap flag, its mapping to `ReviewOutput::Claims` in `command_runtime.rs`, the `command.rs` dispatch that reads plans from `--head`, validates all of them, and returns the `claims.rs` document, the `[--claims]` `Plan Usage:` suffix, the `--claims` help and completion assertions, and the `review_claims_*` Git repository tests for S10 to S12, including the committed plan with repeated `Covers:` lines on `T01`. Out — changes to the Decision Brief output, `validate.rs`, `sce plan check`, and context files.
  - Dependencies: T02
  - Done when: `sce plan review --claims` prints the manifest from the commit with one trailing newline, failures leave stdout empty with the expected exit class, a committed plan with repeated `Covers:` lines exits `3` with the exact `Plan claims check failed` stderr and no stdout while `sce plan check` still accepts it, help and completions show `--claims`, and the AC9 dogfood command succeeds.
  - Verify: `nix develop -c ./scripts/run-cli-cargo.sh test --manifest-path cli/Cargo.toml plan::review_`; the AC9 `Validate:` command.
  - Context synchronization: pending

- [ ] T04: `Document the review command and the claims manifest contract` (status:todo)
  - Task ID: T04
  - Covers: AC10
  - Scope: In — replace the decision brief command contract in `context/cli/plan-service.md` with a `## Review command contract` section, add a `## Claims manifest contract` section with the exact v1 fields, identity, provenance, intent-versus-evidence distinction, separate assessors, and the stdout-only `claims.json` meaning, update the `plan-service.md` scope, tests, and terms, update the `plan` paragraph in `context/cli/cli-command-surface.md`, and update the `context/context-map.md` annotation and any `context/glossary.md` entry that names `decision-brief`. Out — code changes.
  - Dependencies: T03
  - Done when: No durable context file outside `context/plans/` and `context/decisions/` names `sce plan decision-brief` as a current command, and the two new sections state every AC10 item.
  - Verify: `nix shell nixpkgs#ripgrep -c rg -n "decision-brief" context --glob '!plans/**' --glob '!decisions/**' --glob '!handovers/**'` returns only historical references; inspection of the two new sections.
  - Context synchronization: pending

## Open questions

None. The change request fixes the command shape, the schema, the identity rule, the error classes, and the test coverage. D4 carries the one choice a reviewer is asked to judge.
