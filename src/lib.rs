// Cromulent library crate — re-exports modules so integration tests can import them.
// The binary (main.rs) declares the same modules.

pub mod agent {
    pub mod compaction;
    pub mod prompt;
    pub mod runner;
    pub mod transcript;
}

pub mod app {
    pub mod output;
    pub mod router;
    pub mod runtime;
    pub mod state;
}

pub mod auth {
    pub mod codex;
    pub mod config;
}

pub mod process {
    pub mod bash_runner;
}

pub mod protocol {
    pub mod commands;
    pub mod events;
    pub mod responses;
    pub mod types;
}

pub mod providers;

pub mod session {
    pub mod export;
    pub mod fork;
    pub mod store;
}

pub mod tools;

pub mod transport {
    pub mod reader;
    pub mod writer;
}

pub mod util {
    pub mod fs;
    pub mod ids;
    pub mod time;
}
