//! wtf-is-going-on-mcp — see README.md.
//!
//! Everything is implemented in-tree on top of `std` only:
//! SHA-256, HMAC-SHA256, JSON, HTTP/1.1 server+client, and the MCP stdio
//! bridge. Zero external crates is a hard project constraint.

pub mod config;
pub mod hmac;
pub mod json;
pub mod rand;
pub mod sha256;
pub mod store;
pub mod util;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
