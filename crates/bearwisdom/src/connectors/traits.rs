// =============================================================================
// connectors/traits.rs — Connector trait and descriptor
//
// Every protocol-specific connector implements `Connector`.  The registry
// calls detect → extract → custom_match in that order.
// =============================================================================

use anyhow::Result;
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::Path;

use super::types::{ConnectionPoint, Protocol, ResolvedFlow};
use crate::indexer::project_context::ProjectContext;

/// Static metadata about a connector — logged by the registry.
pub struct ConnectorDescriptor {
    pub name: &'static str,
    pub protocols: &'static [Protocol],
    pub languages: &'static [&'static str],
}

/// A connector extracts `ConnectionPoint`s from an indexed project and
/// optionally provides a custom matcher that runs instead of the generic
/// `ProtocolMatcher`.
pub trait Connector: Send + Sync {
    fn descriptor(&self) -> ConnectorDescriptor;

    /// Return `false` to skip this connector for the current project.
    ///
    /// The default always returns `true`; connectors that require a specific
    /// framework or language should inspect `ctx` and return `false` early.
    fn detect(&self, _ctx: &ProjectContext) -> bool {
        true
    }

    /// Extract all connection points from the indexed database.
    ///
    /// `conn` is a read/write connection to the BearWisdom SQLite database.
    /// `project_root` is the absolute path to the project root on disk,
    /// needed when connectors must read source files directly (e.g. proto files).
    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>>;

    /// Incremental variant: extract connection points only from files in
    /// `changed_paths`. Connectors that re-read source files from disk
    /// (e.g. C# DI registration scan, event-handler scan) override this
    /// to skip the full project sweep — saving 10k+ disk reads on every
    /// incremental save.
    ///
    /// Default falls back to `extract` (full project scan), which is
    /// correct but wasteful on incremental.
    fn incremental_extract(
        &self,
        conn: &Connection,
        project_root: &Path,
        _changed_paths: &HashSet<String>,
    ) -> Result<Vec<ConnectionPoint>> {
        self.extract(conn, project_root)
    }

    /// Optional custom matching pass that replaces the generic `ProtocolMatcher`
    /// for this connector's protocol(s).
    ///
    /// Return `Ok(Some(flows))` to provide your own matches.
    /// Return `Ok(None)` to fall through to `ProtocolMatcher`.
    fn custom_match(
        &self,
        _conn: &Connection,
        _starts: &[ConnectionPoint],
        _stops: &[ConnectionPoint],
    ) -> Result<Option<Vec<ResolvedFlow>>> {
        Ok(None)
    }
}
