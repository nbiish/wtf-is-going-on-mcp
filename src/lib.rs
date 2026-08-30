//! wtf-is-going-on-mcp — see README.md.
//!
//! Everything is implemented in-tree on top of `std` only:
//! SHA-256, HMAC-SHA256, JSON, HTTP/1.1 server+client, and the MCP stdio
//! bridge. Zero external crates is a hard project constraint.

pub mod api;
pub mod auth;
pub mod bins;
pub mod client;
pub mod config;
pub mod gcm;
pub mod dashboard;
pub mod hmac;
pub mod http;
pub mod json;
pub mod aes;
pub mod keccak;
pub mod mcp;
pub mod mlkem768;
pub mod ntt_tables;
pub mod rand;
pub mod sessions;
pub mod sha256;
pub mod store;
pub mod util;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
