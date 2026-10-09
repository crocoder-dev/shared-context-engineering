# Hook telemetry benchmark and export decision gate (T05)

Evidence for the `sce-opentelemetry-tracing` plan, task T05. It benchmarks a hook-style `sce` subprocess exporting through the D6 `telemetry-test-receiver` loopback mechanism and records the hook flush-budget decision.

## Decision

- Production hook export stays disabled in standalone and managed modes. The D3 hook-export gate (`RuntimeCommand::is_hook_invocation`, `telemetry_hook_export_gated`) remains in force; this note does not enable anything.
- Candidate hook flush budget: **100 ms**. Rationale: a healthy or refused receiver adds about 4-5 ms end to end and under 1 ms after the command completes, while a blackhole receiver costs the full budget (about 100 ms at a 100 ms budget, about 755 ms at the 1000 ms foreground default, where the 750 ms client timeout bounds it). A 100 ms ceiling keeps the worst case a small fraction of typical hook latency budgets without cutting off the healthy case.
- The budget is not yet a gate pass. Enabling production hook export additionally needs `telemetry_shutdown_subprocess` configured with this budget (hook workload), `telemetry_hook_export_gated` still passing, and a separate change adding an explicit, documented, user-owned enablement policy (never repository-local, never environment-inherited).
- A remote collector is not measured here (loopback only). A local forwarder, which would keep hook export off the network path entirely, is recorded as a separate follow-up and is not implemented by this plan.

## Build command

```
nix develop -c ./scripts/run-cli-cargo.sh build --release --features telemetry-test-receiver --manifest-path cli/Cargo.toml
```

Binary: `cli/target/release/sce` (release profile, 52,154,152 bytes). The feature adds only the D6 resolution branch and lifecycle markers. The packaged `.#default` build excludes it, verified by the flake check `packaged-binary-excludes-test-receiver`. T07 reuses this same feature-enabled binary.

## Methodology

- Script: `scripts/bench-hook-telemetry.sh <binary> <output-dir>`, run as `nix shell nixpkgs#hyperfine nixpkgs#python3 -c scripts/bench-hook-telemetry.sh cli/target/release/sce <output-dir>`; `SCE_TELEMETRY_FLUSH_TIMEOUT_MS` selects the budget.
- Workload: `sce hooks pre-commit` (typed hook command, exits 0 outside a repository), `env -i` with a throwaway `HOME`/`XDG_*`, stdin null.
- Modes: `disabled`; `reachable` (loopback listener answering 200); `blackhole` (loopback listener that accepts and never responds); `refused` (loopback port with no listener). Enabled modes set `SCE_TELEMETRY=test-receiver` and `SCE_TELEMETRY_TEST_RECEIVER_ENDPOINT=http://127.0.0.1:<port>`.
- hyperfine: 5 warmup and 40 measured runs per mode, one mode at a time. Overhead is `enabled - disabled p50` per sample. Hyperfine warns the disabled command is under 5 ms, so its shell-start calibration is imprecise; the same shell wrapper is applied to every mode and the figures are relative.
- Post-command duration: 40 additional runs per enabled mode with `SCE_TELEMETRY_TEST_LIFECYCLE_FILE`; post-command is `process_exit_requested - command_complete`, shutdown is `shutdown_end - shutdown_begin` (process-relative monotonic nanoseconds).
- Host: AMD Ryzen 9 5900X (24 threads), Linux 6.18.37 NixOS, rustc 1.95.0, single host, repository HEAD 77462524 plus the uncommitted T05 working tree.

## Results (milliseconds)

Flush budget 1000 ms (run 1):

| Mode | e2e p50 | e2e p95 | e2e max | overhead p50 | overhead p95 | overhead max | post-command p50 / p95 / max |
| --- | --- | --- | --- | --- | --- | --- | --- |
| disabled | 4.56 | 5.05 | 5.26 | - | - | - | - |
| reachable | 8.57 | 9.23 | 9.44 | 4.01 | 4.67 | 4.88 | 0.61 / 0.72 / 0.79 |
| blackhole | 759.30 | 759.76 | 759.82 | 754.74 | 755.20 | 755.26 | 750.60 / 750.89 / 751.50 |
| refused | 8.61 | 9.22 | 9.24 | 4.05 | 4.66 | 4.67 | 0.39 / 0.46 / 0.53 |

Flush budget 100 ms (run 2, candidate hook budget; its own disabled baseline was re-measured):

| Mode | e2e p50 | e2e p95 | e2e max | overhead p50 | overhead p95 | overhead max | post-command p50 / p95 / max |
| --- | --- | --- | --- | --- | --- | --- | --- |
| reachable | 8.76 | 9.43 | 9.51 | 4.32 | 4.99 | 5.07 | 0.63 / 0.74 / 0.93 |
| blackhole | 109.19 | 109.49 | 109.65 | 104.75 | 105.05 | 105.21 | 100.37 / 100.64 / 101.48 |
| refused | 8.49 | 9.15 | 9.25 | 4.05 | 4.71 | 4.81 | 0.39 / 0.49 / 0.51 |

Overhead is measured against the disabled p50 of the same run (run 1 disabled p50 4.56 ms; run 2 about 4.44 ms), so overhead p95 and max include the enabled mode's own spread.

Reading: the reachable and refused overhead of about 4-5 ms is dominated by exporter and runtime construction, not by export waiting (post-command under 1 ms). The blackhole post-command wait equals the budget plus about 1 ms, so the flush budget, not the exporter, bounds process lifetime at both budgets.

## Verification record

- Build command includes `--features telemetry-test-receiver`; `.#default` check `packaged-binary-excludes-test-receiver` passed.
- `... --features telemetry-test-receiver telemetry_hook_export_gated` through `scripts/check-cargo-test-count.sh`: 6 executed (minimum 6).
- The benchmark script was run twice end to end (budgets 1000 ms and 100 ms) with consistent reachable/refused figures (8.57 vs 8.76 ms and 8.61 vs 8.49 ms p50).
