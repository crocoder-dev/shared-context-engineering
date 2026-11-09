## Comments in code

Prefer clear names and structure over comments. Add comments only to explain non-obvious constraints, invariants, or behavior that the code cannot express clearly. Keep broader design rationale and implementation history in SCE context. Never use code comments to narrate changes, describe previous implementations, or maintain change logs.

## Rust conventions

- Prefer static dispatch through concrete types, generics, and enums. Use dynamic dispatch (`dyn Trait`) only when a concrete requirement justifies it.
- Prefer explicit, typed errors with meaningful variants over `anyhow::Error`. Preserve underlying causes where relevant. Use `anyhow` only when typed errors provide no meaningful benefit.
