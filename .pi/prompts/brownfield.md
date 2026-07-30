---
description: "Reconstruct durable context from an existing repository's own evidence"
argument-hint: "[rebuild] [path ...]"
---

Invoke the `sce-brownfield` skill exactly once with `$ARGUMENTS`.
The skill owns the complete workflow, including all waits and same-session resume
behavior. Do not invoke any phase skill or sequence workflow steps in this command.
