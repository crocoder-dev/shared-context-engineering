# CLI Refactor: Open Architecture Decisions

**Status:** Draft — awaiting decision before implementation begins  
**Affected plans:** `cli-observability-di`, `cli-command-registry`, `cli-service-lifecycle`  
**Context:** Pre-implementation decision gate for the three-phase CLI refactor.

---

## Decision 1: Does `AppContext` include filesystem and git abstractions?

### Background
`cli-observability-di` defines `AppContext` as a DI container carrying `Arc<dyn Logger>` and `Arc<dyn Telemetry>`. However, `cli-service-lifecycle` will need to inject filesystem and git operations for testability (e.g., `hooks` health checks need to read hook files and run `git rev-parse`).

### Options

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| **A — Minimal AppContext** | Keep `AppContext` small (logger + telemetry only). Pass `FsOps`/`GitOps` as separate parameters to `ServiceLifecycle` methods. | `AppContext` remains focused. No bloat for commands that don't need fs/git. | Verbose signatures: `fn diagnose(&self, ctx: &AppContext, fs: &dyn FsOps, git: &dyn GitOps)`. |
| **B — Capabilities AppContext** | `AppContext` carries all capabilities as optional fields: `logger`, `telemetry`, `fs: Option<Arc<dyn FsOps>>`, `git: Option<Arc<dyn GitOps>>`. | Single context object everywhere. Easy to extend. | Can grow into a god object if unchecked. Callers must handle `None`. |
| **C — Trait-per-concern context** | `AppContext` is a trait: `trait AppContext { fn logger(&self) -> &dyn Logger; ... }`. Concrete impls choose which capabilities to expose. | Maximum flexibility. Test impls can stub only what's needed. | More boilerplate. More trait objects. |

### Recommendation
**Option B (Capabilities AppContext)** with a discipline rule: `AppContext` fields are capabilities consumed by *multiple* services. One-off dependencies stay as method parameters.

```rust
pub struct AppContext {
    pub logger: Arc<dyn Logger>,
    pub telemetry: Arc<dyn Telemetry>,
    pub fs: Arc<dyn FsOps>,
    pub git: Arc<dyn GitOps>,
}
```

**Rationale:** The current `doctor/mod.rs` already uses a `DoctorDependencies` struct with 6 function pointers. Consolidating these into `AppContext` is a natural evolution. It also means `ServiceLifecycle` only needs `&AppContext`.

---

## Decision 2: How does clap error handling / help rendering bridge to the registry?

### Background
`app.rs` currently does three things: (1) parse clap args, (2) handle clap errors (DisplayHelp, DisplayVersion, missing subcommand), (3) build `RuntimeCommand` structs. `cli-command-registry` wants to move (3) into a registry, but (1) and (2) still need `cli_schema` access.

### Options

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| **A — Central bridge in app.rs** | Keep `convert_clap_command` in `app.rs` as the clap→registry bridge. `app.rs` stays ~150 lines with parsing + dispatch. | Simple. No new modules. | `app.rs` still owns clap-specific logic. |
| **B — Extract to `services/parse/command_runtime.rs`** | Move `parse_runtime_command`, `convert_clap_command`, and all error handling into a dedicated parse module. `app.rs` calls `command_runtime::parse_and_resolve(args, &registry)`. | `app.rs` is truly thin (~50 lines). Parsing is testable in isolation. | One more module. Slight indirection. |
| **C — Registry owns conversion** | Give registry entries knowledge of clap variants. Registry maps `cli_schema::Commands` directly. | Tight coupling between clap schema and registry. Harder to test. |

### Recommendation
**Option B — extract to `services/parse/command_runtime.rs`**.

```rust
// services/parse/command_runtime.rs
pub fn parse_and_resolve<I>(
    args: I,
    registry: &CommandRegistry,
    logger: Option<&dyn Logger>,
) -> Result<Box<dyn RuntimeCommand>, ClassifiedError> { ... }
```

**Rationale:** `app.rs` should be *startup + render*, not parsing logic. The existing `command_runtime` module inside `app.rs` (lines 338-938) is already a natural extraction target. This also gives us a clean place for `app.rs` integration tests (see Decision 4).

---

## Decision 3: Where do `app.rs` integration tests move?

### Background
`app.rs` lines 941-1040 contain a substantial test block that exercises the full startup flow: invalid discovered config → degraded defaults → warning log. If `app.rs` is thinned, these tests need a home.

### Options

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| **A — Keep in app.rs** | Leave the test block in `app.rs` even after thinning. | No movement required. Tests stay close to the function they exercise. | `app.rs` grows by ~100 lines of tests. |
| **B — Move to `services/parse/tests.rs`** | Move to the new `command_runtime` parse module (Decision 2, Option B). | Tests the parse+startup flow where it belongs. | Requires Decision 2, Option B to be accepted. |
| **C — Move to integration tests (`cli/tests/`)** | Convert to a proper integration test that invokes `app::run` as a black box. | True end-to-end coverage. No compile-time coupling to internals. | Slower. More setup (temp dirs, env var mutation). |

### Recommendation
**Option B — move to `services/parse/tests.rs`** (assuming Decision 2, Option B is accepted). If not, fallback to **Option A**.

**Rationale:** The existing test exercises `run_with_dependency_check_and_streams`, which is the full parse→startup→dispatch→render pipeline. This is effectively a test for the `command_runtime` parser, not `app.rs` itself.

---

## Decision 4: How do we convert single-file services to directories?

### Background
`hooks.rs`, `config.rs`, `setup.rs`, and `local_db.rs` are single files. Both `cli-command-registry` and `cli-service-lifecycle` want multiple files per service (`command.rs`, `lifecycle.rs`, etc.).

### Options

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| **A — Lazily convert per-task** | Each plan task that needs a new file converts the module: `hooks.rs` → `hooks/mod.rs` + new files. | Minimal upfront churn. Natural evolution. | Multiple commits touch the same rename. Risk of merge conflicts if plans run in parallel. |
| **B — Pre-convert all services first** | Create a standalone task that converts all four services to directories upfront. | Clean slate. No rename churn in later tasks. Parallel plan execution becomes safer. | One-time large diff. Slightly more upfront work. |

### Recommendation
**Option B — pre-convert as a standalone task in `cli-command-registry` T00 or `cli-observability-di` T00**.

**Rationale:** If we don't pre-convert, `cli-command-registry` T02-T04 and `cli-service-lifecycle` T02-T04 will all fight over the same file renames. Pre-converting avoids this entirely. The diff is mechanical (rename + add `mod.rs`).

**Suggested placement:** Add a `T00` to `cli-command-registry`: "Pre-convert single-file services to directory modules."

---

## Decision 5: `DoctorDependencies` function pointers vs. `AppContext` capabilities

### Background
`doctor/mod.rs` has a well-structured `DoctorDependencies` struct with 6 function-pointer fields (`run_git_command`, `check_git_available`, `resolve_state_root`, etc.). These map naturally to `AppContext` capabilities (`fs`, `git`) but the current code uses inline closures.

### Options

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| **A — Keep `DoctorDependencies`, pass `AppContext` into closures** | `DoctorDependencies` stays. Each closure captures `&AppContext`. | Minimal change to doctor internals. Existing test patterns (passing closures) still work. | Two DI patterns side by side. |
| **B — Replace `DoctorDependencies` with `&AppContext`** | Delete `DoctorDependencies`. `execute_doctor` takes `&AppContext` directly and calls `ctx.git.run_command(...)`. | Single DI pattern everywhere. | Larger refactor of `doctor/` internals. |
| **C — Hybrid: `DoctorDependencies` is a view over `AppContext`** | `DoctorDependencies` remains but is constructed from `&AppContext` automatically. | Preserves existing doctor test seam. Still centralizes capabilities. | Extra indirection layer. |

### Recommendation
**Option B — replace `DoctorDependencies` with `&AppContext`**.

**Rationale:** The whole point of the refactor is to eliminate ad-hoc DI patterns. `DoctorDependencies` was a good local solution, but `AppContext` is the global solution. The doctor tests can be updated to construct a test `AppContext` with stubbed capabilities.

**Migration path:** T05 of `cli-service-lifecycle` does the replacement. `doctor/inspect.rs` and `doctor/fixes.rs` are updated to call `ctx.git.run_command(...)` instead of `(dependencies.run_git_command)(...)`.

---

## Decision 6: Registry population — static vs. dynamic

### Background
`cli-command-registry` T01 defines a `CommandRegistry`. How does it get populated?

### Options

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| **A — Static `build_default_registry()` function** | A hardcoded function registers all commands: `registry.register("auth", || Box::new(AuthCommand::default()))`. | Simple. Zero runtime cost. Compile-time safety. | Adding a command requires editing this function. |
| **B — Derive macro on `cli_schema::Commands`** | A procedural derive inspects `cli_schema::Commands` and auto-generates registry entries. | Zero boilerplate for new commands. | Requires writing a proc-macro. Overkill for ~8 commands. |
| **C — Lazy registration via `inventory` crate** | Commands self-register using the `inventory` crate (type erasure + link-time registration). | No central list. Commands register themselves. | Adds a dependency. Link-time registration can be brittle. |

### Recommendation
**Option A — static `build_default_registry()` function**.

**Rationale:** We have ~8 commands. The cost of a central function is negligible. The benefit is explicitness: you can see all commands in one place. If we ever grow past 20 commands, we can revisit `inventory` or a derive macro.

```rust
pub fn build_default_registry() -> CommandRegistry {
    let mut registry = CommandRegistry::new();
    registry.register(services::auth_command::NAME, || Box::new(AuthCommand::default()));
    registry.register(services::config::NAME, || Box::new(ConfigCommand::default()));
    // ... etc
    registry
}
```

---

## Decision 7: Should `ServiceLifecycle` be a single trait or split per-operation?

### Background
`cli-service-lifecycle` proposes `ServiceLifecycle` with `diagnose`, `fix`, and `setup`. Not all services need all three (e.g., `version` has nothing to diagnose or fix).

### Options

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| **A — Single `ServiceLifecycle` trait with all three methods** | Every service implements the full trait. Unused methods are no-ops. | Simple. One registry list. | Services like `version` implement empty methods. |
| **B — Split traits: `Diagnosable`, `Fixable`, `Setuppable`** | Each service implements only the traits it needs. `doctor` iterates over `&dyn Diagnosable`. | Precise. No empty methods. | Three registries or one heterogeneous registry. More complex dispatch. |
| **C — Single trait with `Option<...>` return types** | `fn diagnose(...) -> Option<Vec<HealthProblem>>`. `None` means "not applicable." | One trait. Explicit opt-out. | Callers must handle `None`. Less type-safe than split traits. |

### Recommendation
**Option A — single `ServiceLifecycle` trait with default no-op impls**.

```rust
pub trait ServiceLifecycle: Send + Sync {
    fn diagnose(&self, _ctx: &AppContext) -> Vec<HealthProblem> { Vec::new() }
    fn fix(&self, _ctx: &AppContext, _problems: &[HealthProblem]) -> Vec<FixResult> { Vec::new() }
    fn setup(&self, _ctx: &AppContext) -> Result<SetupOutcome, Error> { Ok(SetupOutcome::default()) }
}
```

**Rationale:** Simplicity wins. With ~5-8 services, the cost of empty methods is negligible. The benefit is a single `Vec<Arc<dyn ServiceLifecycle>>` that `doctor` and `setup` can iterate without trait-object casting. We can always split later if the surface grows.

---

## Decision 8: `FsOps` and `GitOps` trait granularity

### Background
If `AppContext` carries `Arc<dyn FsOps>` and `Arc<dyn GitOps>`, how coarse or fine should these traits be?

### Options

| Option | Description | Pros | Cons |
|--------|-------------|------|------|
| **A — Broad traits** | `FsOps` has `read_file`, `write_file`, `metadata`, `exists`. `GitOps` has `run_command`, `resolve_repository_root`, `resolve_hooks_directory`. | Few traits. Easy to mock (one stub per concern). | Might force impls to provide methods they don't need. |
| **B — Fine-grained traits** | `FileReader`, `FileWriter`, `GitCommandRunner`, `GitPathResolver`, etc. | Maximum precision. | Trait explosion. `AppContext` would need many fields. |
| **C — `std::io::Write` + `std::io::Read` abstractions** | Reuse standard traits where possible. | Zero custom traits for basic I/O. | Doesn't cover git. Awkward for `metadata`/`exists`. |

### Recommendation
**Option A — broad traits**.

```rust
pub trait FsOps: Send + Sync {
    fn read_file(&self, path: &Path) -> Result<String>;
    fn write_file(&self, path: &Path, content: &str) -> Result<()>;
    fn metadata(&self, path: &Path) -> Result<Metadata>;
    fn exists(&self, path: &Path) -> bool;
}

pub trait GitOps: Send + Sync {
    fn run_command(&self, repo: &Path, args: &[&str]) -> Result<String>;
    fn resolve_repository_root(&self, dir: &Path) -> Result<PathBuf>;
    fn resolve_hooks_directory(&self, repo: &Path) -> Result<PathBuf>;
    fn is_available(&self) -> bool;
}
```

**Rationale:** These map 1:1 to the current `DoctorDependencies` fields and the actual needs of `hooks`/`config`/`local_db`. If a service needs a method that isn't in the trait, we add it.

---

## Recommended execution order (updated)

Given these decisions, the updated task ordering is:

1. **Add `T00` to `cli-command-registry`** — Pre-convert single-file services to directories.
2. **Execute `cli-observability-di` T01-T05** — Extract traits, build `AppContext` (with `FsOps`/`GitOps` per Decision 1, Option B).
3. **Execute `cli-command-registry` T01-T06** — Build registry, move commands, thin `app.rs` (with `command_runtime` parse module per Decision 2, Option B).
4. **Execute `cli-service-lifecycle` T01-T07** — Define `ServiceLifecycle`, move health/setup logic into services (replacing `DoctorDependencies` per Decision 5, Option B).

---

## Status tracker

| Decision | Status | Blocking task |
|----------|--------|---------------|
| 1 — AppContext capabilities | **Pending** | `cli-observability-di` T03 |
| 2 — Parse module extraction | **Pending** | `cli-command-registry` T01-T05 |
| 3 — Test relocation | **Pending** | `cli-command-registry` T05 |
| 4 — Directory pre-conversion | **Pending** | `cli-command-registry` T00 (new) |
| 5 — DoctorDependencies replacement | **Pending** | `cli-service-lifecycle` T05 |
| 6 — Registry population | **Pending** | `cli-command-registry` T01 |
| 7 — Single vs. split lifecycle trait | **Pending** | `cli-service-lifecycle` T01 |
| 8 — FsOps/GitOps granularity | **Pending** | `cli-observability-di` T03 |
