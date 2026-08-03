//! Domain layer: pure business types and rules for the CLI.
//!
//! Domain code must not depend on `crate::adapters`, `crate::application`,
//! `crate::composition`, `crate::services`, or any infrastructure crate
//! (CLI parsing, database, HTTP, process/filesystem/env access). See
//! `context/architecture.md` for the full dependency-direction rules.
