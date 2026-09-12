// =============================================================================
// ecosystem/npm/types_companion.rs — DefinitelyTyped companions stand in for
// the package they type
//
// `import { useState } from 'react'` names `react`, whose declarations ship
// in `@types/react`. The companion's files are therefore offered under the
// owner's module key as well as their own, and the companion's entry file is
// the owner's entry whenever the owner publishes none of its own.
// =============================================================================

use std::collections::HashMap;
use std::path::PathBuf;

/// The package a DefinitelyTyped package types: `@types/react` → `react`,
/// `@types/babel__core` → `@babel/core`. `None` for any other module.
pub(crate) fn owner_of_types_package(module: &str) -> Option<String> {
    let companion = module.strip_prefix("@types/")?;
    if companion.is_empty() {
        return None;
    }
    Some(match companion.split_once("__") {
        Some((scope, name)) if !scope.is_empty() && !name.is_empty() => format!("@{scope}/{name}"),
        _ => companion.to_string(),
    })
}

/// The module keys a dep root's files are offered under: its own, plus the
/// owner's when the root is a DefinitelyTyped companion.
pub(super) fn module_keys(module: &str) -> Vec<String> {
    let mut keys = vec![module.to_string()];
    keys.extend(owner_of_types_package(module));
    keys
}

/// Register `entry` as `module`'s `.` entry. A companion also claims the
/// owner's entry, without displacing one the owner publishes itself.
pub(super) fn insert_package_entry(
    pkg_entry: &mut HashMap<String, PathBuf>,
    module: &str,
    entry: PathBuf,
) {
    if let Some(owner) = owner_of_types_package(module) {
        pkg_entry.entry(owner).or_insert_with(|| entry.clone());
    }
    pkg_entry.insert(module.to_string(), entry);
}

#[cfg(test)]
#[path = "types_companion_tests.rs"]
mod tests;
