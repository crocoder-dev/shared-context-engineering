# Decision: Enable native Codex telemetry only through the `sce codex` launcher

Date: 2026-11-24
Status: Accepted
Plan: `context/plans/codex-native-otel-instrumentation.md`
Task: `T02`

## Context

Codex CLI 0.172.0 runs `otel.headers_helper` from `CODEX_HOME` and gives it no repository or thread context. A helper command fixed in user-level Codex config therefore cannot identify the repository, and every repository would be attributed to one credential. Repository-local Codex config cannot set `otel.*`. Codex attaches a helper header to every `otlp-http` exporter that is not pinned, including user-owned and profile exporters, so supplying only the helper hands the credential to those destinations. Codex command hooks receive only the `TRACEPARENT` inherited from the process that launched Codex, never the current Codex span. The probe evidence is in [codex-native-telemetry-contract.md](../sce/codex-native-telemetry-contract.md).

## Decision

Native Codex OpenTelemetry export is enabled only through the public `sce codex` launcher. The launcher resolves the repository and the effective access policy before launch. It then `exec`s `codex` with process-local `-c` overrides only:

- `otel.headers_helper` set to the shell-quoted `<sce executable> auth telemetry-headers --repo <repository root>`;
- `otel.headers_helper_refresh_interval_ms=300000`;
- `otel.exporter="none"`, so native log export stays off;
- `otel.trace_exporter` and `otel.metrics_exporter` as `otlp-http` with protocol `binary`, pinned to the managed endpoint's per-signal paths;
- `otel.log_user_prompt`, `otel.log_agent_responses`, and `otel.log_guardian_assessments` set to `false`.

The launcher launches Codex unchanged, without these overrides, and prints a one-line stderr reason in these cases:

- user, profile, or readable system Codex config defines OTEL settings;
- a non-canonical `OTEL_EXPORTER_OTLP_*` destination is visible;
- the caller passes a `-c otel.*` override;
- the invocation mode is unproven: resume without `-C`, interactive fork, or `app-server`, `exec-server`, `remote-control`, `cloud`, `mcp`, or `review`;
- the repository has no authorized telemetry credential.

When the access policy disables telemetry, the launcher launches Codex unchanged without a notice. No Codex config file is written, and no credential appears in arguments. A plain `codex` launch exports no native telemetry. Codex hook spans (`sce.hook`, `sce.agent=codex`) export independently of the launcher as independent roots, with no `TRACEPARENT` extraction.

## Rationale

- Only a launch-time wrapper knows the repository before Codex starts, so only it can pass explicit repository context to the helper.
- Pinning every exporter key with `-c` overrides user and profile layers per key, which closes the observed credential leak path.
- Process-local overrides leave user-owned and enterprise Codex config untouched.
- Native logs carry tool arguments and the user email without a redaction control, so disabling them keeps content off the wire. Metrics and spans still carry tokens, cache, model, tool counts, and durations.
- Inherited `TRACEPARENT` would attach SCE hook spans to whatever trace launched Codex, so Codex hook spans stay roots.

## Alternatives considered

- **User-level Codex config install, like Claude** — rejected because the helper cannot identify the repository from `CODEX_HOME`.
- **Hook spans only, native export deferred** — rejected because the launcher delivers token, model, and tool telemetry now for repository-safe modes.
- **Supplying only the helper override** — rejected because the helper header reaches unpinned user and profile exporters.
- **Exporting native logs** — rejected because tool arguments and `user.email` cannot be redacted in Codex 0.172.0.

## Compatibility and risks

- Coverage is limited to `sce codex` launches. Plain `codex` launches keep only SCE hook spans.
- In-process thread switches to another repository (subagents, `/new`, in-TUI resume) remain attributed to the launch repository.
- MDM and cloud requirements cannot be read by the launcher. Upstream still prevents a user helper from reaching a requirements-layer exporter.
- Codex helper failure suspends export without replay. The Codex session itself is not affected.
- Unrecognized Codex options launch without telemetry, so a new Codex option disables telemetry until the launcher learns it.

## Guardrails

- The launcher never writes Codex config files and never places a credential in arguments, files, or output.
- Every OTEL exporter key is pinned whenever the helper override is supplied.
- The no-argument `sce auth telemetry-headers` form keeps its Claude behavior. The `--repo` form applies the same policy gate, destination tripwire, `auth.db` check, repository binding, and deadline to the named repository.
- Codex hook spans never derive a parent from `TRACEPARENT` or from session, turn, or tool-use identifiers.
- The launcher and the header helper never initialize managed exporter telemetry.

## Consequences

- Native Codex traces and metrics reach the managed collector with repository-scoped credentials for new interactive sessions, `exec`, `exec fork`, and resume with `-C`.
- Codex hook processes are admitted to managed trace export. This narrows the "never admitted" list in the Claude hook admission decision.
- Codex metric names and semantics differ from Claude's. Existing dashboards may need control-plane work to use them.

## Follow-up

- Live Dash0 verification of delivery, attribution, and privacy is recorded by plan task T03.

## References

- Plan: [`codex-native-otel-instrumentation`](../plans/codex-native-otel-instrumentation.md)
- Task: `T02`
- Current-state context: [`telemetry-credential.md`](../sce/telemetry-credential.md)
- Current-state context: [`cli-observability-contract.md`](../sce/cli-observability-contract.md)
- Current-state context: [`access-policy.md`](../cli/access-policy.md)
- Current-state context: [`cli-command-surface.md`](../cli/cli-command-surface.md)
- Evidence: [`codex-native-telemetry-contract.md`](../sce/codex-native-telemetry-contract.md)
- Related decision: [Admit Claude-invoked SCE hooks to managed trace export](2026-10-17-claude-hook-managed-trace-admission.md)
- Related decision: [Claude telemetry removal on global withdrawal](2026-10-21-claude-telemetry-removal-on-global-withdrawal.md)
