//! Cross-process ownership for the single project index writer.
//!
//! The lock is advisory and held by an open file descriptor, so the operating
//! system releases it if the writer exits or crashes. The JSON payload is only
//! diagnostic; lock ownership, rather than a PID file, is the liveness proof.

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

const WRITER_LOCK_FILE: &str = "index-writer.lock";
const WRITER_STATUS_FILE: &str = "index-writer.json";
const START_TIMEOUT: Duration = Duration::from_secs(3);
const START_POLL: Duration = Duration::from_millis(25);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexWriterState {
    Starting,
    Refreshing,
    Watching,
    Idle,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IndexWriterInfo {
    pub pid: u32,
    pub started_at_ms: i64,
    pub heartbeat_at_ms: i64,
    pub state: IndexWriterState,
    pub watching: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexWriterStatus {
    pub active: bool,
    pub info: Option<IndexWriterInfo>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndexWriterLaunch {
    AlreadyRunning,
    Started,
}

struct LeaseInner {
    _file: File,
    info: IndexWriterInfo,
}

/// Exclusive, process-scoped ownership of a project's shared index writer.
pub struct IndexWriterLease {
    status_path: PathBuf,
    inner: Mutex<LeaseInner>,
}

impl IndexWriterLease {
    /// Attempt to own the writer for `index.db`. `None` means another process
    /// already owns it; unexpected filesystem/locking failures are errors.
    pub fn try_acquire(db_path: &Path, watching: bool) -> Result<Option<Self>> {
        let path = writer_lock_path(db_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create writer lock directory {}", parent.display()))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("open writer lock {}", path.display()))?;

        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => {}
            Err(error) if lock_is_contended(&error) => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("lock project writer {}", path.display()))
            }
        }

        let now = now_ms();
        let info = IndexWriterInfo {
            pid: std::process::id(),
            started_at_ms: now,
            heartbeat_at_ms: now,
            state: IndexWriterState::Starting,
            watching,
        };
        let status_path = writer_status_path(db_path);
        persist(&status_path, &info)?;
        Ok(Some(Self {
            status_path,
            inner: Mutex::new(LeaseInner { _file: file, info }),
        }))
    }

    /// Publish writer lifecycle state while retaining the same OS lock.
    pub fn update(&self, state: IndexWriterState, watching: bool) -> Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("writer lease mutex poisoned"))?;
        inner.info.state = state;
        inner.info.watching = watching;
        inner.info.heartbeat_at_ms = now_ms();
        let info = inner.info.clone();
        persist(&self.status_path, &info)
    }
}

/// Observe writer liveness without trusting a stale PID file.
pub fn index_writer_status(db_path: &Path) -> Result<IndexWriterStatus> {
    let path = writer_lock_path(db_path);
    let file = match OpenOptions::new().read(true).write(true).open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(IndexWriterStatus {
                active: false,
                info: None,
            })
        }
        Err(error) => {
            return Err(error).with_context(|| format!("open writer status {}", path.display()))
        }
    };

    // Shared probes do not misclassify concurrent status readers as a writer.
    // The elected writer holds the only exclusive lease.
    match FileExt::try_lock_shared(&file) {
        Ok(()) => {
            let _ = FileExt::unlock(&file);
            Ok(IndexWriterStatus {
                active: false,
                info: None,
            })
        }
        Err(error) if lock_is_contended(&error) => {
            let mut payload = String::new();
            if let Ok(mut status_file) = File::open(writer_status_path(db_path)) {
                status_file.read_to_string(&mut payload)?;
            }
            let info = serde_json::from_str(payload.trim()).ok();
            Ok(IndexWriterStatus { active: true, info })
        }
        Err(error) => {
            Err(error).with_context(|| format!("inspect writer status {}", path.display()))
        }
    }
}

/// Start a detached writer command and wait until it owns the OS lease. The
/// caller supplies the current binary's daemon subcommand arguments, allowing
/// both `bw` and `bw-mcp` to use the same lifecycle implementation.
pub fn ensure_index_writer_process(
    project_root: &Path,
    executable: &Path,
    args: &[OsString],
) -> Result<IndexWriterLaunch> {
    let db_path = crate::resolve_db_path(project_root)?;
    if index_writer_status(&db_path)?.active {
        return Ok(IndexWriterLaunch::AlreadyRunning);
    }

    let log_path = project_root.join(".bearwisdom").join("index-writer.log");
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("open writer log {}", log_path.display()))?;
    let stderr = stdout.try_clone().context("clone writer log handle")?;

    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .current_dir(project_root);
    configure_detached(&mut command);

    let mut child = command.spawn().with_context(|| {
        format!(
            "start automatic index writer {} for {}",
            executable.display(),
            project_root.display()
        )
    })?;

    let deadline = Instant::now() + START_TIMEOUT;
    while Instant::now() < deadline {
        if index_writer_status(&db_path)?.active {
            return Ok(IndexWriterLaunch::Started);
        }
        if let Some(status) = child.try_wait().context("poll automatic index writer")? {
            if status.success() && index_writer_status(&db_path)?.active {
                return Ok(IndexWriterLaunch::AlreadyRunning);
            }
            anyhow::bail!(
                "automatic index writer exited before acquiring its lease ({status}); see {}",
                log_path.display()
            );
        }
        std::thread::sleep(START_POLL);
    }

    anyhow::bail!(
        "automatic index writer did not acquire its lease within {}ms; see {}",
        START_TIMEOUT.as_millis(),
        log_path.display()
    )
}

fn writer_lock_path(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(WRITER_LOCK_FILE)
}

fn writer_status_path(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(WRITER_STATUS_FILE)
}

fn persist(path: &Path, info: &IndexWriterInfo) -> Result<()> {
    let mut payload = serde_json::to_vec(info)?;
    payload.push(b'\n');
    std::fs::write(path, payload)
        .with_context(|| format!("update writer status {}", path.display()))
}

fn lock_is_contended(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock
        || matches!(error.raw_os_error(), Some(11 | 33 | 35 | 36))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(windows)]
fn configure_detached(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

#[cfg(unix)]
fn configure_detached(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(any(windows, unix)))]
fn configure_detached(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_is_exclusive_and_released_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("index.db");

        let lease = IndexWriterLease::try_acquire(&db_path, true)
            .unwrap()
            .expect("first writer");
        lease.update(IndexWriterState::Watching, true).unwrap();

        let status = index_writer_status(&db_path).unwrap();
        assert!(status.active);
        assert_eq!(status.info.unwrap().state, IndexWriterState::Watching);
        assert!(IndexWriterLease::try_acquire(&db_path, true)
            .unwrap()
            .is_none());

        drop(lease);
        assert!(!index_writer_status(&db_path).unwrap().active);
        assert!(IndexWriterLease::try_acquire(&db_path, false)
            .unwrap()
            .is_some());
    }
}
