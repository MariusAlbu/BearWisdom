// =============================================================================
// typescript/module_policy.rs — ES-module specifier grammar as resolver data
//
// What a relative specifier looks like and which indexed files a joined
// relative base may name. Every plugin whose source imports through ES module
// syntax (TypeScript, JavaScript, Vue, Svelte, Astro) supplies this one value.
// =============================================================================

use crate::type_checker::profile::language_profile::{
    ModuleSpecifierClass, SourceModulePathPolicy,
};

pub(crate) const SOURCE_MODULE_PATH_POLICY: SourceModulePathPolicy = SourceModulePathPolicy {
    classify_specifier: classify_module_specifier,
    relative_candidate_paths,
    // A bare specifier names a package, a runtime builtin, or a configured
    // alias; it never names a project file by spelling.
    bare_module_matches_file: |_file, _module| false,
    external_import_match_terms: |_| Vec::new(),
};

/// `./`, `../`, `/` and drive-letter specifiers name files; everything else
/// names a package, a builtin, or a configured alias.
fn classify_module_specifier(specifier: &str) -> ModuleSpecifierClass {
    if specifier.starts_with('.')
        || specifier.starts_with('/')
        || (specifier.len() >= 2 && specifier.as_bytes()[1] == b':')
    {
        ModuleSpecifierClass::Relative
    } else {
        ModuleSpecifierClass::Bare
    }
}

/// Source extensions a relative specifier may omit, probed as `base.<ext>`
/// and then as the directory entry `base/index.<ext>`.
const SOURCE_EXTENSIONS: &[&str] = &[
    "ts", "tsx", "d.ts", "js", "jsx", "mjs", "mts", "cts", "cjs", "svelte", "astro", "vue",
];

/// An emitted-extension specifier (`./x.js`) names the source it compiles from.
const EMITTED_SOURCES: &[(&str, &[&str])] = &[
    (".js", &["ts", "tsx", "d.ts"]),
    (".mjs", &["mts", "d.mts"]),
    (".cjs", &["cts", "d.cts"]),
];

fn relative_candidate_paths(base: &str) -> Vec<String> {
    let mut out = Vec::with_capacity(SOURCE_EXTENSIONS.len() * 2 + 4);
    for (emitted, sources) in EMITTED_SOURCES {
        if let Some(stem) = base.strip_suffix(emitted) {
            out.extend(sources.iter().map(|ext| format!("{stem}.{ext}")));
        }
    }
    out.push(base.to_string());
    out.extend(SOURCE_EXTENSIONS.iter().map(|ext| format!("{base}.{ext}")));
    out.extend(
        SOURCE_EXTENSIONS
            .iter()
            .map(|ext| format!("{base}/index.{ext}")),
    );
    out
}

#[cfg(test)]
#[path = "module_policy_tests.rs"]
mod tests;
