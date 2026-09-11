//! Source-module specifier → indexed file for the module-scoped rungs. Package
//! entries answer bare and adapter-keyed specifiers; the module graph answers
//! relative ones by the importing file's own path rules.
use super::Compilation;

impl Compilation {
    pub(super) fn resolve_module_path(&self, source_file: &str, spec: &str) -> Option<&str> {
        if let Some(key) = crate::ecosystem::module_specifier::relative_entry_key(source_file, spec)
        {
            return self.module_entry.get(&key).map(String::as_str);
        }
        if let Some(entry) = self.module_entry.get(spec) {
            return Some(entry.as_str());
        }
        self.modules.resolve_relative(source_file, spec)
    }
}
