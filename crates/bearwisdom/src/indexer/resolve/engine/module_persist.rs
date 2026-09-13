//! Durable form of the graph: the ingestion recipes and the module
//! configuration cross the database boundary; nothing numeric does.
use super::*;

impl ModuleGraph {
    pub(in crate::indexer::resolve::engine) fn persist(
        &self,
        conn: &rusqlite::Connection,
    ) -> rusqlite::Result<()> {
        self.programs.persist(conn)?;
        let config = serde_json::to_string(&self.configuration)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        conn.execute("INSERT OR REPLACE INTO _bearwisdom_meta (key,value) VALUES ('module_configuration_v1',?1)", [config])?;
        let payload = serde_json::to_string(&self.inputs.values().collect::<Vec<_>>())
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        conn.execute(
            "INSERT OR REPLACE INTO _bearwisdom_meta (key,value) VALUES ('module_bindings_v1',?1)",
            [payload],
        )?;
        Ok(())
    }

    pub(in crate::indexer::resolve::engine) fn load(
        &mut self,
        conn: &rusqlite::Connection,
    ) -> rusqlite::Result<()> {
        self.programs.load(conn)?;
        use rusqlite::OptionalExtension;
        if self.configuration.is_none() {
            let config: Option<String> = conn
                .query_row(
                    "SELECT value FROM _bearwisdom_meta WHERE key='module_configuration_v1'",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(config) = config {
                self.configuration = serde_json::from_str(&config).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
            }
        }
        let payload: Option<String> = conn
            .query_row(
                "SELECT value FROM _bearwisdom_meta WHERE key='module_bindings_v1'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let Some(payload) = payload else {
            return Ok(());
        };
        let stored: Vec<ModuleInput> = serde_json::from_str(&payload).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?;
        let mut statement = conn.prepare("SELECT path,hash FROM files")?;
        let live: FxHashSet<(String, String)> = statement
            .query_map([], |r| {
                Ok((module_paths::normalize(&r.get::<_, String>(0)?), r.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        // A source fingerprint fences stale BindingIds, even when a barrel has
        // no declaration rows. Freshly parsed module environments always win.
        for input in stored {
            if input.binding_epoch == super::super::module_input::BINDING_EPOCH
                && live.contains(&(input.path.clone(), input.content_hash.clone()))
            {
                self.inputs.entry(input.path.clone()).or_insert(input);
            }
        }
        Ok(())
    }
}
