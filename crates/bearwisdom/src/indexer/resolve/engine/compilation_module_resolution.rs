//! Source-module specifier → indexed file for the module-scoped rungs. Package
//! entries answer bare and adapter-keyed specifiers; the module graph answers
//! relative ones and configured alias targets by the importing file's own
//! path rules.
use super::Compilation;
use crate::indexer::symbol_ids::SymbolIds;
use crate::types::ParsedFile;
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

    /// Install ecosystem-published package entries. An entry names an ingested
    /// file; it replaces whatever the batch's own barrel/depth pick chose, and
    /// the module bindings are refreshed so imports link through it.
    pub(crate) fn apply_package_entries(
        &mut self,
        entries: &[(String, String)],
        files: &[ParsedFile],
        ids: &SymbolIds,
    ) {
        let mut changed = false;
        for (module, path) in entries {
            if self.by_file.contains_key(path)
                && self.module_entry.get(module) != Some(path)
            {
                self.module_entry.insert(module.clone(), path.clone());
                changed = true;
            }
        }
        if changed {
            self.refresh_module_bindings(files, ids);
        }
    }
}
