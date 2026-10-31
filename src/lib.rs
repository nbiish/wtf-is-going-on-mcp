//! wtf-live — one localhost view of cross-machine agent sessions.
//!
//! Two live sources, one page:
//!   * Herdr — workspaces, panes, agent state and pane output.
//!   * Orca  — worktrees, terminals, agent state and terminal previews.
//!
//! Plus a file-backed pastebin shared between machines. Everything is `std`
//! only; the whole server is the binary and one embedded page.

pub mod bins;
pub mod exec;
pub mod http;
pub mod json;
pub mod model;
pub mod server;
pub mod sources;
pub mod util;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_PORT: u16 = 7800;
