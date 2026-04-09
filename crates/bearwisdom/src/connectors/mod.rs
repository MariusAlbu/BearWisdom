//! Connector infrastructure — shared types, registry, and matcher.
//!
//! All protocol-specific connector implementations live in their language
//! plugin directories (`languages/{lang}/connectors.rs`). This module
//! provides only the shared plumbing:
//!
//! - `traits` — `Connector` trait and `ConnectorDescriptor`
//! - `types` — `Protocol`, `ConnectionPoint`, `ResolvedFlow`, etc.
//! - `registry` — `ConnectorRegistry` with detect/extract/match/write pipeline
//! - `matcher` — `ProtocolMatcher` for generic key-based flow matching
//! - `connector_db` — SQLite I/O helpers for connection points and flow edges
//!
//! Backing helpers for language plugins (called via `crate::connectors::*`):
//! - `http_api` — route detection helpers
//! - `tauri_ipc` — Tauri IPC detection helpers
//! - `ef_core` — Entity Framework Core DB mapping helpers
//! - `react_patterns` — React concept detection helpers
//! - `docker_compose` — Docker Compose service dependency helpers
//! - `kubernetes` — Kubernetes manifest helpers
//! - `dockerfile` — Dockerfile detection helpers

// Core infrastructure
pub mod connector_db;
pub mod matcher;
pub mod registry;
pub mod traits;
pub mod types;

// Backing helpers for language plugin connectors
pub mod docker_compose;
pub mod dockerfile;
pub mod ef_core;
pub mod http_api;
pub mod kubernetes;
pub mod react_patterns;
pub mod tauri_ipc;
