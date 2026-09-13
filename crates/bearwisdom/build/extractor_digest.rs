//! The external parse cache's version tag: a digest of every source file
//! whose content determines a `ParsedFile`'s cached extraction shape, exposed
//! to the crate as the `BW_EXTRACTOR_DIGEST` environment variable.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Hex digest of every source file whose content determines a `ParsedFile`'s
/// cached extraction shape, emitted as `BW_EXTRACTOR_DIGEST`. The external
/// parse cache keys on it, so an extractor change makes every prior entry
/// un-matchable without a hand-maintained version number.
pub(crate) fn emit(manifest: &Path) {
    let src = manifest.join("src");
    let mut files: Vec<PathBuf> = Vec::new();
    for dir in ["languages", "parser"] {
        collect_extraction_sources(&src.join(dir), &mut files);
    }
    for file in [
        "indexer/external_parse_payload.rs",
        "indexer/parse_file.rs",
        "ecosystem/external_policy.rs",
        "types.rs",
        "type_checker/core/types.rs",
    ] {
        files.push(src.join(file));
    }
    files.sort();

    let mut hasher = Sha256::new();
    for path in &files {
        let rel = path
            .strip_prefix(manifest)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        hasher.update(rel.as_bytes());
        hasher.update(fs::read(path).unwrap_or_default());
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let digest = format!("{:x}", hasher.finalize());
    println!("cargo:rustc-env=BW_EXTRACTOR_DIGEST={}", &digest[..16]);
}

/// Every `.rs` source under `dir` that shapes extraction: generated query data
/// (rewritten by this script on every run) and sibling test files are not part
/// of the shape.
fn collect_extraction_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if name != "query_builtins" {
                collect_extraction_sources(&path, out);
            }
        } else if name.ends_with(".rs")
            && !name.ends_with("_tests.rs")
            && !name.ends_with("_test.rs")
        {
            out.push(path);
        }
    }
}
