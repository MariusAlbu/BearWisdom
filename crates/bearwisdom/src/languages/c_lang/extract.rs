// =============================================================================
// languages/c_lang/extract.rs  —  C and C++ extractor entry point
// =============================================================================

use tree_sitter::Parser;

use super::salvage_callconv::salvage_missed_msvc_calling_convention_decls;
use super::salvage_defines::salvage_missed_defines;
use super::salvage_funcptr::salvage_missed_function_pointer_decls;
use super::salvage_macro_expand::salvage_macro_expanded_decls;
use super::salvage_template_class::salvage_missed_template_class_decls;
use super::type_refs::sweep_typerefs;
use super::visitor::extract_node;
use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

pub(crate) static C_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind {
        node_kind: "struct_specifier",
        name_field: "name",
    },
    ScopeKind {
        node_kind: "enum_specifier",
        name_field: "name",
    },
    ScopeKind {
        node_kind: "union_specifier",
        name_field: "name",
    },
];

pub(crate) static CPP_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind {
        node_kind: "namespace_definition",
        name_field: "name",
    },
    ScopeKind {
        node_kind: "class_specifier",
        name_field: "name",
    },
    ScopeKind {
        node_kind: "struct_specifier",
        name_field: "name",
    },
    ScopeKind {
        node_kind: "enum_specifier",
        name_field: "name",
    },
    ScopeKind {
        node_kind: "union_specifier",
        name_field: "name",
    },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Return `true` when `source` contains C++-only constructs that indicate the
/// file should be parsed with the C++ grammar even if the language id was
/// detected as `"c"` (happens for `.h` files in mixed C/C++ projects).
fn is_cpp_content(source: &str) -> bool {
    // Fast byte-scan: look for C++-only keywords before the first function
    // body (i.e. the first `{`). Using byte search avoids regex overhead.
    let sentinel = source.find('{').unwrap_or(source.len());
    let header = &source[..sentinel];
    for token in [
        "namespace ",
        "template<",
        "template <",
        "class ",
        "operator ",
    ] {
        if header.contains(token) {
            return true;
        }
    }
    false
}

pub fn extract(source: &str, language: &str) -> super::ExtractionResult {
    extract_with_file(source, "", language)
}

pub fn extract_with_file(source: &str, file_path: &str, language: &str) -> super::ExtractionResult {
    // Upgrade ".h" files that contain C++-only constructs to the C++ grammar.
    // The language-profile detector maps ".h" → "c" (correct for pure C
    // projects), but in mixed or C++-only projects the header files contain
    // namespaces, templates, and classes that require the C++ grammar and the
    // CPP_SCOPE_KINDS scope config.
    let effective_language = if language == "c" && is_cpp_content(source) {
        "cpp"
    } else {
        language
    };

    let lang: tree_sitter::Language = if effective_language == "c" {
        tree_sitter_c::LANGUAGE.into()
    } else {
        tree_sitter_cpp::LANGUAGE.into()
    };

    let mut parser = Parser::new();
    parser
        .set_language(&lang)
        .expect("Failed to load C/C++ grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let root = tree.root_node();
    let src = source.as_bytes();
    let has_errors = root.has_error();

    let scope_config = if effective_language == "c" {
        C_SCOPE_KINDS
    } else {
        CPP_SCOPE_KINDS
    };
    let scope_tree = scope_tree::build(root, src, scope_config);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_node(
        root,
        src,
        &scope_tree,
        effective_language,
        &mut symbols,
        &mut refs,
        None,
    );

    // Full-CST type-ref sweep: emit TypeRef for every non-builtin type_identifier
    // and a ref for every template_argument_list in the CST.  This ensures the
    // ref coverage engine can match all type_identifier and template_argument_list
    // nodes, regardless of their depth or syntactic context.
    let macro_catalog = super::macro_catalog::catalog_for_file(file_path);
    let sweep_idx = symbols.len().saturating_sub(1);
    sweep_typerefs(root, src, sweep_idx, effective_language, &mut refs);

    // Drop TypeRefs whose name is a calling-convention / export-qualifier macro
    // (`pTHX_`, `LUA_API`, `WINAPI`, `GLAPI`, `__aio`). Tree-sitter, lacking
    // preprocessing, parses such a leading macro token as a `type_identifier`
    // in return-type / parameter position; the macro expands to a storage or
    // linkage specifier, never a type, so the ref can only ever be unresolvable.
    // Filtering here — after both the visitor and the sweep have emitted — is
    // the single chokepoint every TypeRef-producing path converges on.
    if !macro_catalog.is_empty() {
        refs.retain(|r| {
            r.kind != EdgeKind::TypeRef
                || !macro_catalog.by_name.contains_key(r.target_name.as_str())
        });
    }

    // Raw-text fallback for `#define` symbols that tree-sitter-c missed
    // due to error recovery. Real-world C headers (curl_setup.h, libuv,
    // OpenSSL) have constructs like `typedef enum { ... } bool;` that
    // push the parser into recovery mode, after which subsequent `#define`
    // lines emit as ERROR/text content instead of preproc_def nodes.
    // Salvage missing names by scanning the source line-by-line for
    // `#define IDENT` / `#define IDENT(...)` patterns.
    salvage_missed_defines(source, &mut symbols);

    // Raw-text fallback for function-pointer-table declarations like
    //   `REDISMODULE_API int (*RedisModule_ReplyWithError)(...) REDISMODULE_ATTR;`
    // Library API export macros (REDISMODULE_API, KAPI, MY_API,
    // __declspec(dllexport)) that tree-sitter doesn't preprocess push the
    // declaration into ERROR recovery, after which the `(*name)(` shape is
    // misparsed and no symbol gets emitted. Without this, every call site
    // through the API table is unresolved (8K refs in c-redis alone, plus
    // analogous patterns in nginx Perl/PHP modules and Postgres extensions).
    salvage_missed_function_pointer_decls(source, &mut symbols);

    // Raw-text fallback for MSVC stdlib function declarations whose
    // SAL annotations and calling-convention specifiers confuse the
    // tree-sitter-cpp parser into emitting `__cdecl`, `_Check_return_`,
    // and friends as the function name. Real names like `printf`,
    // `strlen`, `memcpy` end up buried inside the misparsed parameter
    // list. Without this, every call into the C runtime stays
    // unresolved on Windows even when the SDK headers are indexed.
    salvage_missed_msvc_calling_convention_decls(source, &mut symbols);

    // Raw-text fallback for `<MACRO> template <...> class NAME[;|{]`
    // declarations. MSVC's `<memory>` / `<vector>` / `<string>` use
    // `_EXPORT_STD template <class _Ty> class shared_ptr;` for C++20
    // module-export forward decls. The unknown `_EXPORT_STD`
    // identifier pushes tree-sitter-cpp into recovery and the class
    // name is dropped. The salvage scans for the `template <...>
    // class IDENT` shape and emits a Class symbol regardless of any
    // prefix tokens.
    salvage_missed_template_class_decls(source, &mut symbols);

    // Generic project-macro expansion. The `#define` directives in the
    // file's neighbouring headers (sibling directory, plus
    // `<parent>/include/` and `<parent>/inc/`) are parsed once per
    // directory and cached. For every invocation in `source` whose name
    // matches a discovered macro, the macro body is substituted with the
    // call's arguments and re-fed through the declaration scanners that
    // ran on the original text. Recovers symbols generated by ANY
    // project's helper macros — Clay's `CLAY__ARRAY_DEFINE`, nginx's
    // `ngx_cdecl`, FreeBSD's `__printflike`, etc. — without baking any
    // specific name into BW.
    salvage_macro_expanded_decls(source, file_path, &mut symbols);

    super::ExtractionResult::new(symbols, refs, has_errors)
}
