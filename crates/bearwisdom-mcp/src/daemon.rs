use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::{info, warn};

const SWEEP_INTERVAL_MS: i64 = 60_000;
const SUPERVISOR_POLL: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnsureWriterResult {
    Disabled,
    AlreadyRunning,
    Started,
}

/// Ensure a detached BearWisdom-owned writer exists for this project.
/// Concurrent MCP clients may race here; the core OS lease elects one child
/// and every losing child exits immediately.
pub fn ensure_running(project: &Path) -> Result<EnsureWriterResult> {
    if std::env::var_os("BEARWISDOM_DISABLE_AUTO_WRITER").is_some() {
        return Ok(EnsureWriterResult::Disabled);
    }

    let project = canonical_project(project);
    let executable = std::env::current_exe().context("resolve bw-mcp executable")?;
    let args = vec![
        OsString::from("index-daemon"),
        OsString::from("--project"),
        project.as_os_str().to_owned(),
    ];
    match bearwisdom::ensure_index_writer_process(&project, &executable, &args)? {
        bearwisdom::IndexWriterLaunch::AlreadyRunning => Ok(EnsureWriterResult::AlreadyRunning),
        bearwisdom::IndexWriterLaunch::Started => {
            info!("automatic index writer active for {}", project.display());
            Ok(EnsureWriterResult::Started)
        }
    }
}

/// Run the elected project writer. This process remains independent of the
/// MCP client that launched it and keeps both a watcher and periodic catch-up
/// sweep alive.
pub fn run(project: &Path, debounce_ms: u64) -> Result<()> {
    let project = canonical_project(project);
    let db_path = bearwisdom::resolve_db_path(&project)?;
    let options = bearwisdom::IndexServiceOptions {
        pool_size: 2,
        watch: true,
        debounce: Duration::from_millis(debounce_ms),
        allow_refresh: true,
    };

    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let service = loop {
        match bearwisdom::IndexService::open(&db_path, &project, options.clone()) {
            Ok(service) => break Arc::new(service),
            Err(_error) if bearwisdom::index_writer_status(&db_path)?.active => {
                info!(
                    "another automatic index writer already owns {}; exiting",
                    project.display()
                );
                return Ok(());
            }
            Err(error) if std::time::Instant::now() < deadline => {
                // A status reader may hold a shared probe for a few
                // microseconds. Retry so observation cannot prevent election.
                warn!("writer lease acquisition retry: {error}");
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(error).context("open automatic index writer"),
        }
    };

    info!("automatic index writer elected for {}", project.display());
    if let Err(error) = service.reindex_now() {
        warn!("automatic writer initial refresh failed: {error:#}");
    }

    loop {
        std::thread::sleep(SUPERVISOR_POLL);
        service.try_spawn_sweep(SWEEP_INTERVAL_MS);
    }
}

fn canonical_project(project: &Path) -> PathBuf {
    project
        .canonicalize()
        .unwrap_or_else(|_| project.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_writer_prevents_process_spawn() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = bearwisdom::resolve_db_path(dir.path()).unwrap();
        let _lease = bearwisdom::IndexWriterLease::try_acquire(&db_path, true)
            .unwrap()
            .expect("writer lease");

        assert_eq!(
            ensure_running(dir.path()).unwrap(),
            EnsureWriterResult::AlreadyRunning
        );
    }
}
