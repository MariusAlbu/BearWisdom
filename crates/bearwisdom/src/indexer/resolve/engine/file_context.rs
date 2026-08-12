// =============================================================================
// engine/file_context — FileContext assembly for the resolve pipeline
//
// Two registry-driven lookups built once per resolve pass (language-id →
// LanguageProfile, language-id → LanguagePlugin) plus the FileContext builder
// itself, which turns a ParsedFile's import-describing refs into ImportEntry
// rows and appends whatever the file's LanguagePlugin contributes from its own
// cross-file PluginStateBag state.
// =============================================================================

use rustc_hash::FxHashMap;

use crate::indexer::plugin_state::PluginStateBag;
use crate::indexer::resolve::engine::contract::{FileContext, ImportEntry};
use crate::languages::LanguagePlugin;
use crate::type_checker::profile::language_profile::{ImportModulePath, LanguageProfile};
use crate::types::{EdgeKind, ParsedFile};

// ---------------------------------------------------------------------------
// Profile map
// ---------------------------------------------------------------------------

/// Build a language-id → LanguageProfile map from the default plugin registry.
///
/// Registers each profile under every language id the plugin claims, matching
/// the same multi-id pattern `Engine::build_from_registry` uses.
#[cfg(test)]
pub(crate) fn _test_build_profiles() -> FxHashMap<&'static str, &'static LanguageProfile> {
    build_profiles()
}

pub(crate) fn build_profiles() -> FxHashMap<&'static str, &'static LanguageProfile> {
    let mut profiles: FxHashMap<&'static str, &'static LanguageProfile> = FxHashMap::default();
    for plugin in crate::languages::default_registry().all() {
        if let Some(profile) = plugin.profile() {
            for &lang in plugin.language_ids() {
                profiles.insert(lang, profile);
            }
        }
    }
    profiles
}

/// Language-id → plugin lookup, built once per resolve pass. Mirrors
/// `build_profiles()`'s registry walk; dispatches
/// `LanguagePlugin::extra_wildcard_imports` by file language without any
/// language string in the pipeline itself.
pub(crate) fn build_plugin_lookup() -> FxHashMap<&'static str, &'static dyn LanguagePlugin> {
    let mut plugins: FxHashMap<&'static str, &'static dyn LanguagePlugin> = FxHashMap::default();
    for plugin in crate::languages::default_registry().all() {
        for &lang in plugin.language_ids() {
            plugins.insert(lang, plugin.as_ref());
        }
    }
    plugins
}

// ---------------------------------------------------------------------------
// FileContext builder
// ---------------------------------------------------------------------------

/// Build a `FileContext` from profile data alone, without invoking the Engine.
///
/// Replicates the logic from `engine.rs::generic_file_context`:
/// - `FromModuleField` — any ref with a `module` field becomes an import entry.
/// - Other modes — only `EdgeKind::Imports` refs; `module_path` is either empty
///   (`None` mode) or echoes the target name (`EchoTarget` mode).
pub(crate) fn build_file_context(
    language: &str,
    file: &ParsedFile,
    profile: &LanguageProfile,
    plugin: Option<&dyn LanguagePlugin>,
    plugin_state: Option<&PluginStateBag>,
) -> FileContext {
    // A plain namespace import (`using System;`) is a wildcard of its module
    // path when the profile says so; a literal `*` target always is. Binding
    // imports (named `import { X }` forms) are never namespace wildcards. A
    // re-export ref (`export * from './x'`) also carries a literal `*`
    // target, but it describes what the file exposes to OTHER files, not
    // what it imports into its own scope — it must never become a wildcard
    // entry here, or the exporting file's own bare-name lookups would
    // wrongly search the re-exported module.
    let entry_is_wildcard = |r: &crate::types::ExtractedRef| {
        !r.is_reexport
            && (r.target_name == "*"
                || (profile.imports.namespace_imports_are_wildcards
                    && r.kind == EdgeKind::Imports
                    && !r.is_import_binding))
    };
    let mut imports: Vec<ImportEntry> = match profile.imports.import_module_path {
        // Build entries from import-describing refs only: an explicit import
        // binding (`import { X } from 'm'`) or an `Imports`-kind ref (require /
        // side-effect). A bare usage ref now also carries `module` (set from the
        // import that binds its name), so an unfiltered scan would re-derive a
        // duplicate entry per use site; sourcing the import map from binding refs
        // leaves one entry per imported name while the usage ref's module
        // attribution still reaches the rules via `ctx.r().module`.
        ImportModulePath::FromModuleField => file
            .refs
            .iter()
            .filter(|r| r.is_import_binding || r.kind == EdgeKind::Imports)
            .filter_map(|r| {
                let module = r.module.clone()?;
                // A rename import carries the module's ORIGINAL declared name
                // as a single-segment chain (`use m::Orig as Bound`). The entry
                // keys the original name — that is what the module's files
                // declare — with the locally bound name as the alias.
                let original = r.chain.as_ref().and_then(|c| match c.segments.as_slice() {
                    [seg] if seg.name != r.target_name => Some(seg.name.clone()),
                    _ => None,
                });
                Some(match original {
                    Some(orig) => ImportEntry {
                        imported_name: orig,
                        module_path: Some(module),
                        alias: Some(r.target_name.clone()),
                        is_wildcard: false,
                    },
                    None => ImportEntry {
                        imported_name: r.target_name.clone(),
                        module_path: Some(module),
                        alias: None,
                        is_wildcard: entry_is_wildcard(r),
                    },
                })
            })
            .collect(),
        mode => file
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .map(|r| ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: match mode {
                    ImportModulePath::None => None,
                    ImportModulePath::EchoTarget => Some(r.target_name.clone()),
                    ImportModulePath::FromModuleField => unreachable!(),
                },
                alias: None,
                is_wildcard: entry_is_wildcard(r),
            })
            .collect(),
    };
    if let (Some(plugin), Some(state)) = (plugin, plugin_state) {
        imports.extend(plugin.extra_wildcard_imports(state, file));
    }
    FileContext {
        file_path: file.path.clone(),
        language: language.to_string(),
        imports,
        file_namespace: None,
    }
}

#[cfg(test)]
#[path = "file_context_tests.rs"]
mod tests;
