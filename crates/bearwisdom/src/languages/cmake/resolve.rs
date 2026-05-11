// =============================================================================
// languages/cmake/resolve.rs — CMake resolution rules
//
// CMake references fall into two categories:
//
//   include(SomeModule)            → Imports, target_name = "SomeModule"
//   find_package(Qt6 REQUIRED)     → Imports, target_name = "Qt6"
//   add_subdirectory(subdir)       → Imports, target_name = "subdir"
//   my_function(arg1 arg2)         → Calls,   target_name = "my_function"
//   ${MY_VARIABLE}                 → TypeRef, target_name = "MY_VARIABLE"
//
// Resolution strategy:
//   1. Same-file: functions and macros defined in the same file.
//   2. Global name lookup: user-defined functions/macros from included modules.
//   3. CMake built-in commands and variables are marked external.
// =============================================================================

use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct CMakeResolver;

impl LanguageResolver for CMakeResolver {
    fn language_ids(&self) -> &[&str] {
        &["cmake"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: r.module.clone().or_else(|| Some(r.target_name.clone())),
                alias: None,
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "cmake".to_string(),
            imports,
            file_namespace: None,
        }
    }

    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        // Import declarations are module-level, not symbol refs.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // CMake built-in commands and variables don't live in the project index.
        if is_cmake_builtin(target) {
            return None;
        }

        engine::resolve_common("cmake", file_ctx, ref_ctx, lookup, cmake_kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Imported targets use `Pkg::Component` syntax, which is never valid
        // for user-defined CMake functions or project-local targets.
        if target.contains("::") {
            let pkg = target.split("::").next().unwrap_or(target);
            return Some(pkg.to_string());
        }

        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, is_cmake_builtin)
    }
}

/// Edge kind / symbol kind compatibility for CMake.
fn cmake_kind_compatible(edge_kind: crate::types::EdgeKind, sym_kind: &str) -> bool {
    use crate::types::EdgeKind;
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "function" | "macro"),
        EdgeKind::TypeRef => matches!(sym_kind, "variable" | "function" | "macro"),
        _ => true,
    }
}

/// CMake built-in commands, control structures, and standard variables.
///
/// Exposed as `pub(super)` so `extract.rs` can use it to skip emitting
/// unresolvable `Calls` refs for built-in command names.
pub(super) fn is_cmake_builtin(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let s = lower.as_str();

    // CMake standard variable prefixes — these are never user-defined symbols
    if s.starts_with("cmake_")
        || s.starts_with("project_")
        || s.starts_with("cpack_")
        || s.starts_with("ctest_")
        || s.starts_with("fetchcontent_")
    {
        return true;
    }

    // CMake special function argument variables — ARGC, ARGN, ARGV, ARGV0..ARGV9
    if matches!(s, "argc" | "argn" | "argv")
        || (s.starts_with("argv") && s[4..].parse::<u8>().is_ok())
    {
        return true;
    }

    super::keywords::KEYWORDS.contains(&s)
}
