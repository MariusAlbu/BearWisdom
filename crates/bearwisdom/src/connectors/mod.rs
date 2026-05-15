//! Shared types and helpers from the legacy connector kill.
//!
//! Phase H removed the matcher / registry / `ConnectionPoint` pipeline. Only
//! two utilities survive here because their consumers cross plugin boundaries:
//!
//! - `types::Protocol` — string-form protocol family used by dockerfile / HCL
//!   infrastructure connectors that write `flow_edges` directly.
//! - `url_pattern::normalize` — URL pattern normaliser used by every resolver
//!   that emits `FlowEmission::NamedChannel { kind: HttpCall, .. }`.

pub mod types;
pub mod url_pattern;
