// =============================================================================
// languages/make/resolve.rs — Makefile / Make resolution rules
//
// Make references:
//
//   include other.mk             → Imports, target_name = "other.mk"
//   $(shell ...)                 → Calls,   target_name = "shell" (built-in)
//   $(wildcard *.c)              → Calls,   target_name = "wildcard" (built-in)
//   $(MY_VAR)                    → TypeRef, target_name = "MY_VAR"
//   target: dep1 dep2            → target depends on dep1, dep2 (Calls)
//
// Resolution strategy:
//   1. Same-file: variables and targets defined in the same Makefile.
//   2. Global name lookup: included Makefiles bring their targets/variables
//      into scope.
//   3. Make built-in functions and automatic variables are external.
// =============================================================================

use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct MakeResolver;

impl LanguageResolver for MakeResolver {
    fn language_ids(&self) -> &[&str] {
        &["make", "makefile"]
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
                module_path: Some(r.target_name.clone()),
                alias: None,
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "make".to_string(),
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

        // Include directives don't resolve to a symbol.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Make built-in functions and automatic variables are never in index.
        if is_make_builtin(target) {
            return None;
        }

        engine::resolve_common("make", file_ctx, ref_ctx, lookup, |_, _| true)
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        if is_make_builtin(&ref_ctx.extracted_ref.target_name) {
            return Some("make".to_string());
        }
        None
    }
}

/// Make built-in functions and special/automatic variables.
fn is_make_builtin(name: &str) -> bool {
    matches!(
        name,
        // Text functions
        "subst" | "patsubst" | "strip" | "findstring" | "filter"
            | "filter-out" | "sort" | "word" | "words" | "wordlist"
            | "firstword" | "lastword"
            // Filename functions
            | "dir" | "notdir" | "suffix" | "basename" | "addsuffix"
            | "addprefix" | "join" | "wildcard" | "realpath" | "abspath"
            // Conditional functions
            | "if" | "or" | "and" | "not"
            // foreach / call / eval / value / let
            | "foreach" | "call" | "eval" | "value" | "let"
            // Shell / origin / flavor
            | "shell" | "origin" | "flavor" | "error" | "warning" | "info"
            // File functions
            | "file" | "guile"
            // Automatic variables (stripped of $ and parens by extractor)
            | "@" | "%" | "<" | "?" | "^" | "+" | "|" | "*"
            | "@D" | "@F" | "%D" | "%F" | "<D" | "<F" | "?D" | "?F"
            | "^D" | "^F" | "+D" | "+F" | "*D" | "*F"
            // Special targets
            | ".PHONY" | ".SUFFIXES" | ".DEFAULT" | ".PRECIOUS"
            | ".INTERMEDIATE" | ".SECONDARY" | ".SECONDEXPANSION"
            | ".DELETE_ON_ERROR" | ".IGNORE" | ".LOW_RESOLUTION_TIME"
            | ".SILENT" | ".EXPORT_ALL_VARIABLES" | ".NOTPARALLEL"
            | ".ONESHELL" | ".POSIX" | ".MAKE" | ".MAKEFLAGS"
            // Common predefined variables
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
