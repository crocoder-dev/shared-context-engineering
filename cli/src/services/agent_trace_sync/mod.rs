//! Synchronization of a repository's local Agent Trace capture database with
//! the control-plane Agent Trace ingestion API.

pub mod control_plane;

#[cfg(test)]
pub(crate) mod test_http_server;
