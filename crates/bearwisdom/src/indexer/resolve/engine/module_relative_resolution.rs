//! A relative specifier names an indexed file by the importing file's own
//! path rules; the graph's path table is the only file evidence consulted.
use super::super::{module_paths, ModuleGraph};

impl ModuleGraph {
    /// The indexed file `spec` names relative to `source_file`, spelled as the
    /// graph keys it. `None` for a non-relative specifier, a source without a
    /// module input, or a base no indexed file spells.
    pub(in crate::indexer::resolve::engine) fn resolve_relative(
        &self,
        source_file: &str,
        spec: &str,
    ) -> Option<&str> {
        let input = self.inputs.get(&module_paths::normalize(source_file))?;
        let base = module_paths::relative_base(&input.path, spec)?;
        module_paths::find(&base, &input.paths, |path| {
            self.paths
                .get_key_value(path)
                .map(|(key, _)| key.as_str())
        })
    }
}

#[cfg(test)]
#[path = "module_relative_resolution_tests.rs"]
mod tests;
