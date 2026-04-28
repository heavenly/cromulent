// Cromulent library crate — re-exports modules so integration tests can import them.
// The binary (main.rs) declares the same modules.

pub mod app;
pub mod agent;
pub mod auth;
pub mod process;
pub mod protocol;
pub mod providers;
pub mod session;
pub mod tools;
pub mod transport;
pub mod util;
