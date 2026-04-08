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
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
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

        // Step 1: Same-file resolution (functions/macros defined here).
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name.eq_ignore_ascii_case(target) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "cmake_same_file",
                });
            }
        }

        // Step 2: Global name lookup (included modules, subdirectories).
        // CMake names are case-insensitive for commands; use eq_ignore_ascii_case.
        for sym in lookup.by_name(target) {
            if matches!(sym.kind.as_str(), "function" | "macro") {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.9,
                    strategy: "cmake_global_function",
                });
            }
        }

        // Step 3: Variable reference fallback.
        if edge_kind == EdgeKind::TypeRef {
            if let Some(sym) = lookup.by_name(target).into_iter().next() {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.8,
                    strategy: "cmake_global_var",
                });
            }
        }

        None
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        if is_cmake_builtin(target) {
            return Some("cmake".to_string());
        }

        // find_package() imports are external packages.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return Some("cmake".to_string());
        }

        None
    }
}

/// CMake built-in commands, control structures, and standard variables.
fn is_cmake_builtin(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        // Control flow
        "if" | "else" | "elseif" | "endif"
            | "while" | "endwhile"
            | "foreach" | "endforeach"
            | "function" | "endfunction"
            | "macro" | "endmacro"
            | "return" | "break" | "continue"
            // Configuration / project
            | "cmake_minimum_required" | "project" | "cmake_policy"
            | "cmake_parse_arguments" | "cmake_language"
            // Target commands
            | "add_executable" | "add_library" | "add_custom_target"
            | "add_custom_command" | "add_test" | "add_subdirectory"
            | "add_dependencies" | "add_compile_options" | "add_compile_definitions"
            | "add_link_options"
            // Property commands
            | "set_target_properties" | "get_target_property"
            | "set_property" | "get_property"
            | "target_compile_options" | "target_compile_definitions"
            | "target_include_directories" | "target_link_libraries"
            | "target_link_options" | "target_sources"
            | "target_compile_features" | "target_precompile_headers"
            // Find commands
            | "find_package" | "find_library" | "find_program"
            | "find_path" | "find_file"
            // Variable / cache
            | "set" | "unset" | "option" | "list" | "string" | "math"
            | "message" | "configure_file" | "file" | "include"
            | "include_directories" | "link_directories" | "link_libraries"
            // Install
            | "install" | "export"
            // Testing
            | "enable_testing" | "ctest_configure" | "ctest_build"
            // Misc
            | "execute_process" | "try_compile" | "try_run"
            | "define_property" | "mark_as_advanced"
            | "source_group" | "aux_source_directory"
    )
}
