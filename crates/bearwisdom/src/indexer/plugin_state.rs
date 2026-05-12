// =============================================================================
// indexer/plugin_state.rs — heterogeneous per-plugin state bag
//
// Each language plugin may store one value of any `Send + Sync + 'static` type
// in the bag. The bag is populated once per index pass (full or incremental) by
// calling `LanguagePlugin::populate_project_state` on each active plugin, then
// threaded through to resolvers via `ProjectContext::plugin_state`.
//
// The map is keyed by `TypeId` so each distinct Rust type occupies exactly one
// slot — two plugins using the same type would collide. Convention: each plugin
// defines its own newtype state struct (e.g. `RobotProjectState`) to guarantee
// uniqueness.
// =============================================================================

use std::any::{Any, TypeId};
use std::collections::HashMap;

/// Heterogeneous map keyed by the `TypeId` of the stored value.
///
/// Implements `Debug` as an opaque entry count — the values are `Any` and
/// not individually debuggable without knowing their concrete types.
///
/// Each plugin stores at most one value per type. Reads are O(1)
/// HashMap lookups; there is no dynamic dispatch beyond the trait-object
/// downcast.
pub struct PluginStateBag {
    entries: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl PluginStateBag {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Insert or replace a value of type `T`.
    pub fn set<T: Any + Send + Sync>(&mut self, value: T) {
        self.entries.insert(TypeId::of::<T>(), Box::new(value));
    }

    /// Return a shared reference to the stored value of type `T`, or `None`
    /// if this plugin has not stored a value of that type.
    pub fn get<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.entries
            .get(&TypeId::of::<T>())
            .and_then(|b| b.downcast_ref::<T>())
    }

    /// Return a clone of the stored value of type `T`, or `T::default()` when
    /// no value has been stored. Requires `T: Default + Clone`.
    pub fn get_or_default<T: Any + Send + Sync + Default + Clone>(&self) -> T {
        self.get::<T>().cloned().unwrap_or_default()
    }
}

impl std::fmt::Debug for PluginStateBag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginStateBag")
            .field("entry_count", &self.entries.len())
            .finish()
    }
}

impl Default for PluginStateBag {
    fn default() -> Self {
        Self::new()
    }
}

/// Cloning a bag produces an empty bag. Values stored in the bag are not
/// `Clone`, and the bag is always repopulated from scratch each index pass,
/// so an empty clone is the correct semantic.
impl Clone for PluginStateBag {
    fn clone(&self) -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "plugin_state_tests.rs"]
mod tests;
