// indexer/module_resolution/dotnet.rs — .NET resolver
//
// .NET uses namespace-based resolution, not file paths.  A `using Foo.Bar`
// statement brings types from the namespace into scope but doesn't point to a
// specific file — multiple files can contribute to the same namespace.
//
// This resolver intentionally returns `None` for all inputs and lets the
// existing scope-based resolution (LanguageResolver for csharp/fsharp) handle
// reference resolution.  It is registered so the dispatch layer knows .NET
// languages are covered, but the module-to-file map is not populated for them.

use super::ModuleResolver;

pub struct DotNetModuleResolver;

const LANGUAGES: &[&str] = &["csharp", "fsharp", "vbnet"];

impl ModuleResolver for DotNetModuleResolver {
    fn language_ids(&self) -> &[&str] {
        LANGUAGES
    }

    fn resolve_to_file(
        &self,
        _specifier: &str,
        _importing_file: &str,
        _file_paths: &[&str],
    ) -> Option<String> {
        // .NET namespaces don't map 1-to-1 to files.
        // Defer entirely to the language-specific scope resolver.
        None
    }
}
