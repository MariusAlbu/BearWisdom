// CMake language hooks. Absorbed from the deleted `cmake/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct CMakeHooks;

/// CMake built-in commands, control structures, and standard variables.
pub(crate) fn is_cmake_builtin(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let s = lower.as_str();
    if s.starts_with("cmake_")
        || s.starts_with("project_")
        || s.starts_with("cpack_")
        || s.starts_with("ctest_")
        || s.starts_with("fetchcontent_")
    {
        return true;
    }
    if s.starts_with("cpm_") {
        return true;
    }
    if matches!(s, "argc" | "argn" | "argv")
        || (s.starts_with("argv") && s[4..].parse::<u8>().is_ok())
    {
        return true;
    }
    super::keywords::KEYWORDS.contains(&s)
}

fn cmake_kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "function" | "macro"),
        EdgeKind::TypeRef => matches!(sym_kind, "variable" | "function" | "macro"),
        _ => true,
    }
}

impl LanguageEngineHooks for CMakeHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, is_cmake_builtin)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
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
        Some(FileContext {
            file_path: file.path.clone(),
            language: "cmake".to_string(),
            imports,
            file_namespace: None,
        })
    }

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;
        if edge_kind == EdgeKind::Imports {
            return None;
        }
        if is_cmake_builtin(target) {
            return None;
        }
        engine::resolve_common("cmake", file_ctx, ref_ctx, lookup, cmake_kind_compatible)
    }
}

pub static CMAKE_HOOKS: CMakeHooks = CMakeHooks;
