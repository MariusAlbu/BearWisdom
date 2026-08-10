// =============================================================================
// nuget/version_select.rs — package-version directory selection
//
// Picks which cached NuGet package version to crack for a requested
// coordinate: exact directory match, else nearest by the version-key order
// documented on select_version_subdir.
// =============================================================================

use std::path::Path;

/// Ordered NuGet version key for `major.minor[.patch[.revision]][-prerelease]`
/// directory names. Numeric components compare numerically; a release
/// version outranks a prerelease with the same numeric parts.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct VersionKey {
    major: u64,
    minor: u64,
    patch: u64,
    revision: u64,
    is_release: bool,
    prerelease: String,
}

impl VersionKey {
    /// Parses `major[.minor[.patch[.revision]]][-prerelease]`; missing
    /// trailing numeric components default to 0. `None` when any present
    /// numeric segment (including the leading one) fails to parse.
    fn parse(raw: &str) -> Option<VersionKey> {
        let (numeric, prerelease) = match raw.split_once('-') {
            Some((n, p)) => (n, p.to_string()),
            None => (raw, String::new()),
        };
        let mut parts = [0u64; 4];
        let mut seen = 0usize;
        for (i, segment) in numeric.split('.').enumerate() {
            if i >= parts.len() {
                return None;
            }
            parts[i] = segment.parse().ok()?;
            seen += 1;
        }
        if seen == 0 {
            return None;
        }
        Some(VersionKey {
            major: parts[0],
            minor: parts[1],
            patch: parts[2],
            revision: parts[3],
            is_release: prerelease.is_empty(),
            prerelease,
        })
    }
}

struct VersionCandidate {
    name: String,
    key: VersionKey,
}

/// Picks the package-version directory under `dir` nearest `requested`:
/// exact name match; else the highest version sharing `requested`'s major;
/// else the lowest major above `requested`; else the highest available.
/// With no `requested` version, picks the highest available. A directory
/// name that doesn't parse as a version is never selected while a
/// parseable one exists.
pub(super) fn select_version_subdir(dir: &Path, requested: Option<&str>) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    let candidates: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            if e.file_type().ok()?.is_dir() {
                e.file_name().into_string().ok()
            } else {
                None
            }
        })
        .collect();
    select_nearest_version(&candidates, requested)
}

pub(super) fn select_nearest_version(candidates: &[String], requested: Option<&str>) -> Option<String> {
    if let Some(req) = requested {
        if let Some(exact) = candidates.iter().find(|c| c.as_str() == req) {
            return Some(exact.clone());
        }
    }

    let parsed: Vec<VersionCandidate> = candidates
        .iter()
        .filter_map(|name| {
            VersionKey::parse(name).map(|key| VersionCandidate {
                name: name.clone(),
                key,
            })
        })
        .collect();
    if parsed.is_empty() {
        return candidates.iter().max().cloned();
    }

    let Some(req_key) = requested.and_then(VersionKey::parse) else {
        return highest(&parsed);
    };

    let same_major = highest_where(&parsed, |k| k.major == req_key.major);
    if same_major.is_some() {
        return same_major;
    }

    if let Some(min_major_above) = parsed
        .iter()
        .filter(|c| c.key.major > req_key.major)
        .map(|c| c.key.major)
        .min()
    {
        let nearest_above = highest_where(&parsed, |k| k.major == min_major_above);
        if nearest_above.is_some() {
            return nearest_above;
        }
    }

    highest(&parsed)
}

fn highest(candidates: &[VersionCandidate]) -> Option<String> {
    candidates
        .iter()
        .max_by(|a, b| a.key.cmp(&b.key))
        .map(|c| c.name.clone())
}

fn highest_where(
    candidates: &[VersionCandidate],
    pred: impl Fn(&VersionKey) -> bool,
) -> Option<String> {
    candidates
        .iter()
        .filter(|c| pred(&c.key))
        .max_by(|a, b| a.key.cmp(&b.key))
        .map(|c| c.name.clone())
}

#[cfg(test)]
#[path = "version_select_tests.rs"]
mod tests;
