//! Application layer: use cases and ports orchestrating the domain.
//!
//! Application code may depend on `crate::domain` but must not depend on
//! `crate::adapters`, `crate::composition`, `crate::services`, or
//! infrastructure crates (CLI parsing, database, HTTP, process/filesystem
//! access). See `context/architecture.md` for the full dependency-direction
//! rules.

mod error;
mod ports;
mod use_cases;
