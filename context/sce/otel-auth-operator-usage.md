# OTLP Auth Operator Usage

`sce` supports env-only OTLP header authentication for hosted OpenTelemetry collectors.
Header values are secret-bearing and must remain outside repo-managed configuration.

## Safe runtime setup

Use shell environment variables or a local secret manager to provide collector settings at process launch time:

```bash
SCE_OTEL_ENABLED=true \
OTEL_EXPORTER_OTLP_ENDPOINT=https://example-collector.invalid/v1/traces \
OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf \
OTEL_EXPORTER_OTLP_HEADERS='Authorization=Bearer <token>' \
sce version
```

For gRPC collectors, keep the same header syntax and switch only the protocol/endpoint shape:

```bash
SCE_OTEL_ENABLED=true \
OTEL_EXPORTER_OTLP_ENDPOINT=https://example-collector.invalid:4317 \
OTEL_EXPORTER_OTLP_PROTOCOL=grpc \
OTEL_EXPORTER_OTLP_HEADERS='Authorization=Bearer <token>' \
sce version
```

Dash0 and similar hosted collectors use this same standard OTLP mechanism; `sce` does not provide Dash0-specific config keys.

## Local verification

Before running a command that exports telemetry, verify that `sce` sees header auth without printing header material:

```bash
SCE_OTEL_ENABLED=true \
OTEL_EXPORTER_OTLP_HEADERS='Authorization=Bearer <token>' \
sce config show
```

Expected safe signal:

- text output includes `otel.exporter_otlp_headers: [REDACTED] (source: env)`
- JSON output reports `configured: true`, `display_value: "[REDACTED]"`, and `source: "env"`
- raw header names and values are not rendered in config output, logs, stderr diagnostics, or file sinks

Malformed header syntax fails only when OTEL export is enabled. Use comma-separated `key=value` pairs; values may contain additional `=` characters after the first separator.

## Safeguards

- Do not write `OTEL_EXPORTER_OTLP_HEADERS` or bearer tokens into `.sce/config.json`.
- Do not add OTLP header values to `config/schema/sce-config.schema.json` or Pkl schema sources.
- Do not commit real collector tokens, API keys, or realistic token-looking examples to tests, docs, context files, shell history captures, or `context/tmp/` artifacts.
- Prefer placeholders such as `<token>` in durable docs and examples.
- Use `sce config show` for redacted local verification; use collector dashboards only as an external operational check.

Related context: [CLI Observability Contract](cli-observability-contract.md), [CLI Config Precedence Contract](../cli/config-precedence-contract.md).
