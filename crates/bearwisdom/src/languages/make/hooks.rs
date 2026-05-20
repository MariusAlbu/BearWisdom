// Make/Makefile hooks. Absorbed from the deleted `make/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct MakeHooks;

/// Make built-in functions and special/automatic variables.
pub(crate) fn is_make_builtin(name: &str) -> bool {
    matches!(
        name,
        "subst" | "patsubst" | "strip" | "findstring" | "filter"
            | "filter-out" | "sort" | "word" | "words" | "wordlist"
            | "firstword" | "lastword"
            | "dir" | "notdir" | "suffix" | "basename" | "addsuffix"
            | "addprefix" | "join" | "wildcard" | "realpath" | "abspath"
            | "if" | "or" | "and" | "not"
            | "foreach" | "call" | "eval" | "value" | "let"
            | "shell" | "origin" | "flavor" | "error" | "warning" | "info"
            | "file" | "guile"
            | "@" | "%" | "<" | "?" | "^" | "+" | "|" | "*"
            | "@D" | "@F" | "%D" | "%F" | "<D" | "<F" | "?D" | "?F"
            | "^D" | "^F" | "+D" | "+F" | "*D" | "*F"
            | ".PHONY" | ".SUFFIXES" | ".DEFAULT" | ".PRECIOUS"
            | ".INTERMEDIATE" | ".SECONDARY" | ".SECONDEXPANSION"
            | ".DELETE_ON_ERROR" | ".IGNORE" | ".LOW_RESOLUTION_TIME"
            | ".SILENT" | ".EXPORT_ALL_VARIABLES" | ".NOTPARALLEL"
            | ".ONESHELL" | ".POSIX" | ".MAKE" | ".MAKEFLAGS"
            | "MAKE" | "MAKEFILE_LIST" | "MAKEFLAGS" | "MFLAGS"
            | "MAKELEVEL" | "MAKEFILES" | "MAKECMDGOALS"
            | "CURDIR" | "VPATH" | "SUFFIXES"
            | "AR" | "AS" | "CC" | "CXX" | "CPP" | "FC" | "M2C"
            | "PC" | "CO" | "GET" | "LEX" | "YACC" | "LINT" | "MAKEINFO"
            | "TEX" | "TEXI2DVI" | "WEAVE" | "CWEAVE" | "TANGLE" | "CTANGLE"
            | "RM"
            | "ARFLAGS" | "ASFLAGS" | "CFLAGS" | "CXXFLAGS" | "COFLAGS"
            | "CPPFLAGS" | "FFLAGS" | "GFLAGS" | "LDFLAGS" | "LFLAGS"
            | "YFLAGS" | "PFLAGS" | "RFLAGS" | "LINTFLAGS"
    )
}

impl LanguageEngineHooks for MakeHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        if is_make_builtin(&ref_ctx.extracted_ref.target_name) {
            return Some("make".to_string());
        }
        None
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
                module_path: Some(r.target_name.clone()),
                alias: None,
                is_wildcard: false,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "make".to_string(),
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
        if is_make_builtin(target) {
            return None;
        }
        engine::resolve_common("make", file_ctx, ref_ctx, lookup, |_, _| true)
    }
}

pub static MAKE_HOOKS: MakeHooks = MakeHooks;
