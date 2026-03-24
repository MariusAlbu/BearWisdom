use std::path::Path;

/// Additional subdirectories to probe for entry-point files beyond the root.
/// These are common in monorepos / Tauri-style projects.
const PROBE_SUBDIRS: &[&str] = &[
    "src",
    "src-tauri",
    "app",
    "apps",
    "lib",
    "libs",
    "packages",
    "backend",
    "frontend",
    "client",
    "server",
    "api",
    "web",
    "mobile",
    "desktop",
    "crates",
];

/// Find entry-point files for a language in `root` and common subdirectories.
///
/// Returns relative paths (as strings) for any entry-point file that exists.
/// `max_depth` controls how many levels of subdirectory nesting to probe.
pub fn find_entry_points(
    root: &Path,
    entry_point_files: &[&str],
    max_depth: usize,
) -> Vec<String> {
    let mut found = Vec::new();

    // Check root first.
    check_dir(root, root, entry_point_files, &mut found);

    // Check known subdirs up to max_depth.
    if max_depth >= 1 {
        for subdir in PROBE_SUBDIRS {
            let sub = root.join(subdir);
            if sub.is_dir() {
                check_dir(&sub, root, entry_point_files, &mut found);

                if max_depth >= 2 {
                    // Go one level deeper inside monorepo sub-packages.
                    if let Ok(entries) = std::fs::read_dir(&sub) {
                        for entry in entries.flatten() {
                            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                                let grandchild = entry.path();
                                check_dir(&grandchild, root, entry_point_files, &mut found);
                            }
                        }
                    }
                }
            }
        }
    }

    found.sort();
    found.dedup();
    found
}

fn check_dir(
    dir: &Path,
    root: &Path,
    entry_point_files: &[&str],
    found: &mut Vec<String>,
) {
    for ep in entry_point_files {
        // Support simple glob patterns like "*.csproj".
        if ep.contains('*') {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if glob_match(ep, &name) {
                        push_relative(dir.join(&name), root, found);
                    }
                }
            }
        } else {
            let candidate = dir.join(ep);
            if candidate.exists() {
                push_relative(candidate, root, found);
            }
        }
    }
}

fn push_relative(path: std::path::PathBuf, root: &Path, found: &mut Vec<String>) {
    if let Ok(rel) = path.strip_prefix(root) {
        found.push(rel.to_string_lossy().into_owned());
    }
}

fn glob_match(pattern: &str, name: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix('*') {
        name.ends_with(suffix)
    } else if let Some(prefix) = pattern.strip_suffix('*') {
        name.starts_with(prefix)
    } else {
        pattern == name
    }
}
