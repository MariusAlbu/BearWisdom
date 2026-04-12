/// Runtime globals always external for Rust.
/// Crate names commonly seen in attribute paths (#[serde(...)], #[tokio::main]).
pub(crate) const EXTERNALS: &[&str] = &[
    "serde", "async_trait", "tokio", "tracing", "anyhow", "thiserror",
    "clap", "log", "env_logger",
];
