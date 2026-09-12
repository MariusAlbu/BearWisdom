//! Source-module specifier → indexed file for the module-scoped rungs. Package
//! entries answer bare and adapter-keyed specifiers; the module graph answers
//! relative ones and configured alias targets by the importing file's own
//! path rules.
use super::Compilation;
use crate::indexer::resolve::engine::contract::SymbolLookup;

impl Compilation {
    pub(super) fn resolve_module_path(&self, source_file: &str, spec: &str) -> Option<&str> {
        if let Some(key) = crate::ecosystem::module_specifier::relative_entry_key(source_file, spec)
        {
            return self.module_entry.get(&key).map(String::as_str);
        }
        if let Some(entry) = self.module_entry.get(spec) {
            return Some(entry.as_str());
        }
        if let Some(path) = self.modules.resolve_relative(source_file, spec) {
            return Some(path);
        }
        let alias = self
            .resolver_policy_for(self.package_id_for_file(source_file))
            .resolve_module_alias(spec)?;
        self.modules
            .resolve_base(source_file, &super::super::module_paths::normalize(&alias))
    }
}
