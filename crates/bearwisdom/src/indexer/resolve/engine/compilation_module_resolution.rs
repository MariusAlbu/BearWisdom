//! Source-module specifier → indexed file for the module-scoped rungs. Package
//! entries answer bare and adapter-keyed specifiers; the module graph answers
//! relative ones, configured alias targets and sibling workspace packages by
//! the importing file's own path rules.
use super::Compilation;
use crate::indexer::symbol_ids::SymbolIds;
use crate::types::ParsedFile;
use crate::indexer::resolve::engine::contract::SymbolLookup;

impl Compilation {
    pub(super) fn resolve_module_path(&self, source_file: &str, spec: &str) -> Option<&str> {
        // A sibling workspace package is its own source: the first declared
        // entry candidate an indexed file spells wins over any copy of the
        // package pulled from a dependency directory.
        if let Some(entry) = self.workspace_package_entry(source_file, spec) {
            return Some(entry);
        }
        if let Some(entry) = self.workspace_source_root_entry(source_file, spec) {
            return Some(entry);
        }
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

    fn workspace_package_entry(&self, source_file: &str, spec: &str) -> Option<&str> {
        self.workspace_entry_candidates(spec)
            .iter()
            .find_map(|base| self.modules.resolve_base(source_file, base))
    }

    /// A workspace package that publishes from a declared source root maps a
    /// deep specifier structurally: `<pkg>/<sub>` names the indexed file
    /// `<package root>/<source root>/<sub>`. A package that declares no source
    /// root publishes from its own root and is left to the other links.
    fn workspace_source_root_entry(&self, source_file: &str, spec: &str) -> Option<&str> {
        let (pkg_id, sub) = crate::ecosystem::module_specifier::workspace_package_sub_path(
            spec,
            &self.workspace_pkg_by_declared_name,
        )?;
        if sub.is_empty() {
            return None;
        }
        let root = self.module_specifier.pkg_source_root.get(&pkg_id)?;
        self.modules.resolve_base(source_file, &format!("{root}/{sub}"))
    }

    /// Only the exact package specifier claims the package's `.` entries; a
    /// subpath (`next/link`) names an export the manifest would have to
    /// declare separately and falls through to the other links.
    pub(super) fn workspace_entry_candidates(&self, specifier: &str) -> &[String] {
        if !self.is_workspace_declared_name(specifier) {
            return &[];
        }
        self.workspace_package_id(specifier)
            .and_then(|id| self.module_specifier.workspace_entries.get(&id))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(super) fn resolve_via_language_resolver(
        &self,
        language: &str,
        source_file: &str,
        spec: &str,
    ) -> Option<String> {
        super::module_specifier::resolve_via_module_resolver(
            language,
            source_file,
            spec,
            self.package_id_for_file(source_file),
            &self.module_specifier.pkg_declared_name,
            &self.module_specifier.resolver_inputs,
            &self.module_specifier.workspace_packages,
            &self.module_specifier.file_paths,
        )
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

#[cfg(test)]
#[path = "compilation_module_resolution_tests.rs"]
mod tests;
