use super::*;
use crate::types::{ExtractedSymbol, SymbolKind};

/// Construct an `ExtractedSymbol` with only the containment-bearing fields set.
fn esym(
    name: &str,
    qname: &str,
    kind: SymbolKind,
    scope_path: Option<&str>,
    parent_index: Option<usize>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: None,
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: scope_path.map(|s| s.to_string()),
        parent_index,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

// --- normalize_qnames_from_parents -----------------------------------------

/// The Java parameter whose stored qname dropped its package is corrected from
/// its method's (already package-correct) qname; the surrounding type/method
/// symbols are left untouched.
#[test]
fn normalize_corrects_dropped_package_param() {
    let mut symbols = vec![
        esym("App", "app.App", SymbolKind::Class, None, None), // 0
        esym(
            "run",
            "app.App.run",
            SymbolKind::Method,
            Some("app.App"),
            Some(0),
        ), // 1
        // Param stored qname dropped the package: inner `App.run` is a suffix of
        // the parent's `app.App.run`.
        esym(
            "repo",
            "App.run.repo",
            SymbolKind::Parameter,
            Some("App.run"),
            Some(1),
        ), // 2
    ];

    normalize_qnames_from_parents(&mut symbols);

    assert_eq!(symbols[2].qualified_name, "app.App.run.repo");
    assert_eq!(symbols[2].scope_path.as_deref(), Some("app.App.run"));
    // The method is left exactly as the extractor produced it.
    assert_eq!(symbols[1].qualified_name, "app.App.run");
    assert_eq!(symbols[1].scope_path.as_deref(), Some("app.App"));
}

/// A local variable whose qname dropped an outer package is corrected from its
/// function's qname; a fully qualified sibling is left byte-identical.
#[test]
fn normalize_corrects_dropped_scope_variable() {
    let mut symbols = vec![
        esym("run", "pkg.run", SymbolKind::Function, None, None), // 0
        // Local dropped the `pkg` head: inner `run` is a suffix of `pkg.run`.
        esym("tmp", "run.tmp", SymbolKind::Variable, Some("run"), Some(0)), // 1
        // Already fully qualified — untouched.
        esym(
            "out",
            "pkg.run.out",
            SymbolKind::Variable,
            Some("pkg.run"),
            Some(0),
        ), // 2
    ];

    normalize_qnames_from_parents(&mut symbols);

    assert_eq!(symbols[1].qualified_name, "pkg.run.tmp");
    assert_eq!(symbols[1].scope_path.as_deref(), Some("pkg.run"));
    assert_eq!(symbols[2].qualified_name, "pkg.run.out");
}

/// A leaf whose qname equals its bare name (no `<sep><name>` tail) is NOT
/// rewritten even when its parent is a namespace-bearing scope — the
/// dropped-prefix signature requires a composed dotted/colon qname, so a
/// fully-dropped scope is left to the extractor's authority rather than guessed.
#[test]
fn normalize_leaves_bare_name_leaf_untouched() {
    let mut symbols = vec![
        esym("builder", "builder", SymbolKind::Function, None, None), // 0
        esym(
            "PATH",
            "PATH",
            SymbolKind::Variable,
            Some("builder"),
            Some(0),
        ), // 1
    ];

    normalize_qnames_from_parents(&mut symbols);

    assert_eq!(symbols[1].qualified_name, "PATH");
}

/// Non-leaf kinds (types, methods, fields, properties) are never rewritten even
/// when their qname dropped a prefix — the pass only corrects params/locals, so
/// the broad type/member surface stays byte-identical.
#[test]
fn normalize_leaves_non_leaf_kinds_untouched() {
    let mut symbols = vec![
        esym("App", "ns.App", SymbolKind::Class, None, None), // 0
        // Method, field, property each with a dropped-prefix qname — all left as-is.
        esym("run", "App.run", SymbolKind::Method, Some("App"), Some(0)), // 1
        esym("_ctx", "App._ctx", SymbolKind::Field, Some("App"), Some(0)), // 2
        esym(
            "Items",
            "App.Items",
            SymbolKind::Property,
            Some("App"),
            Some(0),
        ), // 3
    ];
    let before: Vec<String> = symbols.iter().map(|s| s.qualified_name.clone()).collect();

    normalize_qnames_from_parents(&mut symbols);

    let after: Vec<String> = symbols.iter().map(|s| s.qualified_name.clone()).collect();
    assert_eq!(before, after, "non-leaf kinds must not be rewritten");
}

/// Selector-style qnames (CSS/SCSS) whose qname equals the bare selector and
/// whose parent is an enclosing selector are NOT rewritten: there is no
/// `<sep><name>` tail to decompose, so the dropped-prefix signature fails.
#[test]
fn normalize_leaves_selector_qnames_untouched() {
    let mut symbols = vec![
        esym(".todo-item", ".todo-item", SymbolKind::Variable, None, None), // 0
        esym(
            "&:hover",
            "&:hover",
            SymbolKind::Variable,
            Some(".todo-item"),
            Some(0),
        ), // 1
        esym(
            "&::after",
            "&::after",
            SymbolKind::Variable,
            Some(".todo-item"),
            Some(0),
        ), // 2
    ];
    let before: Vec<String> = symbols.iter().map(|s| s.qualified_name.clone()).collect();

    normalize_qnames_from_parents(&mut symbols);

    let after: Vec<String> = symbols.iter().map(|s| s.qualified_name.clone()).collect();
    assert_eq!(before, after, "selector qnames must not be rewritten");
}

/// An already-consistent param (qname == parent + sep + name) is left untouched —
/// nothing was dropped.
#[test]
fn normalize_leaves_consistent_param_untouched() {
    let mut symbols = vec![
        esym("run", "app.App.run", SymbolKind::Method, None, None), // 0
        esym(
            "repo",
            "app.App.run.repo",
            SymbolKind::Parameter,
            Some("app.App.run"),
            Some(0),
        ), // 1
    ];

    normalize_qnames_from_parents(&mut symbols);

    assert_eq!(symbols[1].qualified_name, "app.App.run.repo");
    assert_eq!(symbols[1].scope_path.as_deref(), Some("app.App.run"));
}

/// A dropped-prefix `::` param is rebuilt with the `::` separator inferred from
/// the parent qname, not the default dot.
#[test]
fn normalize_rebuilds_with_inferred_colon_separator() {
    let mut symbols = vec![
        esym("m", "crate::foo::Bar::m", SymbolKind::Method, None, None), // 0
        // Param dropped the `crate::foo` head: inner `Bar::m` is a `::`-suffix of
        // the parent qname.
        esym(
            "x",
            "Bar::m::x",
            SymbolKind::Parameter,
            Some("Bar::m"),
            Some(0),
        ), // 1
    ];

    normalize_qnames_from_parents(&mut symbols);

    assert_eq!(symbols[1].qualified_name, "crate::foo::Bar::m::x");
    assert_eq!(symbols[1].scope_path.as_deref(), Some("crate::foo::Bar::m"));
}
