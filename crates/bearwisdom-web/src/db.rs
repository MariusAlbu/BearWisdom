use std::path::Path;

use anyhow::Result;

pub fn resolve_db_path(project_root: &Path) -> Result<std::path::PathBuf> {
    bearwisdom::resolve_db_path(project_root)
}

pub fn db_exists(project_root: &Path) -> bool {
    bearwisdom::db_exists(project_root)
}

pub fn open_existing_db(project_root: &Path) -> Result<bearwisdom::Database> {
    let db_path = resolve_db_path(project_root)?;
    if !db_path.exists() {
        anyhow::bail!(
            "No index found for {}. POST /api/index first.",
            project_root.display()
        );
    }
    bearwisdom::Database::open_with_vec(&db_path)
        .with_context(|| format!("Failed to open database at {}", db_path.display()))
}

use anyhow::Context;
