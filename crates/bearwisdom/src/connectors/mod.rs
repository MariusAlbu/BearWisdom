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
//! - `http_api` — route normalisation helpers used by the matcher

// Core infrastructure
pub mod connector_db;
pub mod http_api;
pub mod matcher;
pub mod registry;
pub mod traits;
pub mod types;
