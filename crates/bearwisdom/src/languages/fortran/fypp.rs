// languages/fortran/fypp.rs — fypp preprocessor integration
//
// Locates the fypp binary (or `python -m fypp` fallback), discovers the
// nearest `include/` directory containing `common.fypp`, and runs fypp on
// `.fypp` source files.  Output is cached in the system temp directory keyed
// by SHA-256 of the input bytes so repeated indexing passes skip the
// subprocess entirely.
//
// If fypp is not installed or the subprocess fails, the caller falls back to
// the existing line-blanking strategy — this module never panics and never
// causes an indexing failure.

use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// fypp executable discovery
// ---------------------------------------------------------------------------

/// Absolute path (or command name) of the fypp executable, discovered once
/// per process.  `None` means fypp is not available on this machine.
static FYPP_EXE: OnceLock<Option<FyppInvoker>> = OnceLock::new();

#[derive(Debug, Clone)]
enum FyppInvoker {
    /// `fypp` is directly on PATH.
    Direct,
    /// Python interpreter path that supports `python -m fypp`.
    PythonModule(String),
}

impl FyppInvoker {
    fn build_command(&self, args: &[&str]) -> Command {
        match self {
            FyppInvoker::Direct => {
                let mut cmd = Command::new("fypp");
                cmd.args(args);
                cmd
            }
            FyppInvoker::PythonModule(python) => {
                let mut cmd = Command::new(python);
                cmd.arg("-m").arg("fypp").args(args);
                cmd
            }
        }
    }
}

fn locate_fypp() -> Option<&'static FyppInvoker> {
    FYPP_EXE
        .get_or_init(|| {
            // Try bare `fypp` first.
            if Command::new("fypp")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                return Some(FyppInvoker::Direct);
            }

            // Try known Python 3 interpreter locations for `python -m fypp`.
            let candidates = [
                "python3",
                "python",
                // Windows-style paths the user may have installed.
                r"C:\Users\Reaper\AppData\Local\Programs\Python\Python313\python.exe",
                r"C:\Python313\python.exe",
                r"C:\Python311\python.exe",
                r"C:\Python310\python.exe",
            ];
            for py in &candidates {
                if Command::new(py)
                    .args(["-m", "fypp", "--version"])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
                {
                    return Some(FyppInvoker::PythonModule((*py).to_owned()));
                }
            }
            None
        })
        .as_ref()
}

// ---------------------------------------------------------------------------
// Include-directory discovery
// ---------------------------------------------------------------------------

/// Walk upward from `file_path` looking for an `include/common.fypp` marker.
/// Returns the absolute path to the `include/` directory if found.
///
/// fortran-stdlib keeps `include/common.fypp` at the project root.  Other
/// fypp-based projects follow the same convention.  When `file_path` is
/// relative (as it is when the indexer passes the DB-relative path), the
/// search is retried anchored to the process working directory so that the
/// walk sees real filesystem paths.
fn find_include_dir(file_path: &str) -> Option<String> {
    // Build a canonical starting path: absolute paths resolve directly;
    // relative paths are joined against the process CWD.
    let base = {
        let p = Path::new(file_path);
        if p.is_absolute() {
            p.to_path_buf()
        } else if let Ok(cwd) = std::env::current_dir() {
            cwd.join(p)
        } else {
            p.to_path_buf()
        }
    };

    let start = base.parent()?;
    let mut dir = start;
    loop {
        let candidate = dir.join("include").join("common.fypp");
        if candidate.exists() {
            return Some(dir.join("include").to_string_lossy().into_owned());
        }
        match dir.parent() {
            Some(p) if p != dir => dir = p,
            _ => return None,
        }
    }
}

// ---------------------------------------------------------------------------
// Disk cache
// ---------------------------------------------------------------------------

/// Path of the on-disk cached output for a given input hash, stored in the
/// system temp directory.
fn cache_path(hash: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("bw_fypp_{hash}.f90"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Try to preprocess `source` (the raw `.fypp` content of `file_path`) via
/// fypp.  Returns the generated Fortran source on success.  Returns `None`
/// when fypp is unavailable, the subprocess fails, or the output is empty —
/// the caller should fall back to the existing line-blanking strategy.
pub fn preprocess(file_path: &str, source: &[u8]) -> Option<String> {
    let invoker = locate_fypp()?;

    let hash = sha256_hex(source);
    let cached = cache_path(&hash);

    // Fast path: cached output already on disk.
    if cached.exists() {
        if let Ok(content) = std::fs::read_to_string(&cached) {
            if !content.is_empty() {
                return Some(content);
            }
        }
    }

    // Locate the include directory so `#:include "common.fypp"` resolves.
    let include_dir = find_include_dir(file_path);

    // Write source to a named temp file so fypp can read it by path.
    // fypp does not support stdin on all platforms.
    let input_path = std::env::temp_dir().join(format!("bw_fypp_in_{hash}.fypp"));
    if std::fs::write(&input_path, source).is_err() {
        return None;
    }

    let out_path = cached.clone();
    let input_path_str = input_path.to_string_lossy().into_owned();
    let out_path_str = out_path.to_string_lossy().into_owned();

    // Build the argument list.  Owned strings that are borrowed by `args` must
    // outlive the `args` vec, so they are declared before it.
    let include_dir_owned: String;
    let mut args: Vec<&str> = Vec::new();

    // Inject include directory when found, so `#:include "common.fypp"` resolves.
    if let Some(inc) = include_dir {
        include_dir_owned = inc;
        args.push("-I");
        args.push(&include_dir_owned);
    }

    // Provide dummy version variables so common.fypp's `PROJECT_VERSION`
    // expression doesn't abort — the exact values don't affect symbol names.
    args.extend_from_slice(&[
        "-DPROJECT_VERSION_MAJOR=0",
        "-DPROJECT_VERSION_MINOR=0",
        "-DPROJECT_VERSION_PATCH=0",
    ]);

    args.push(&input_path_str);
    args.push(&out_path_str);

    let status = invoker.build_command(&args).status().ok()?;

    // Clean up temp input regardless of outcome.
    let _ = std::fs::remove_file(&input_path);

    if !status.success() {
        return None;
    }

    let content = std::fs::read_to_string(&out_path).ok()?;
    if content.is_empty() {
        return None;
    }
    Some(content)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "fypp_tests.rs"]
mod fypp_tests;
