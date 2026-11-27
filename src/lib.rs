//! wtf — stateless zero-dependency SSH/local fleet status, goals, and pickup CLI.
//!
//! Implemented purely on Rust `std` (zero external crates).

pub mod config;
pub mod fleet;
pub mod json;
pub mod probe;
pub mod rand;
pub mod util;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
