// =============================================================================
// type_checker/profile/registry.rs — ProfileRegistry
//
// Maps `language_id` to the static `LanguageProfile` the engine consumes.
// Languages register their profile via `ProfileRegistry::register`; lookups
// fall back to `DEFAULT_PROFILE` when no entry exists for the requested id.
// =============================================================================

use rustc_hash::FxHashMap;

use super::language_profile::{LanguageProfile, DEFAULT_PROFILE};

#[derive(Default)]
pub struct ProfileRegistry {
    by_id: FxHashMap<&'static str, &'static LanguageProfile>,
}

impl ProfileRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert / overwrite the profile registered for `language_id`. The id
    /// must match `LanguageProfile::id` for the profile to be looked up
    /// later by id.
    pub fn register(&mut self, profile: &'static LanguageProfile) {
        self.by_id.insert(profile.id, profile);
    }

    /// Number of distinct profiles registered.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Look up the profile for `language_id`. Returns `&DEFAULT_PROFILE`
    /// when no profile is registered for the id.
    pub fn get(&self, language_id: &str) -> &'static LanguageProfile {
        self.by_id
            .get(language_id)
            .copied()
            .unwrap_or(&DEFAULT_PROFILE)
    }

    /// Returns true when `language_id` has a profile registered (i.e. lookup
    /// would not fall through to `DEFAULT_PROFILE`).
    pub fn contains(&self, language_id: &str) -> bool {
        self.by_id.contains_key(language_id)
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
