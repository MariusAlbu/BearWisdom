use super::*;
use crate::types::{ExtractedSymbol, SymbolKind};

fn frame(kind: FrameKind, name: &str) -> ScopeFrame {
    ScopeFrame::new(kind, name, None)
}

fn ns(name: &str) -> ScopeFrame {
    frame(FrameKind::Sym(SymbolKind::Namespace), name)
}
fn class(name: &str) -> ScopeFrame {
    frame(FrameKind::Sym(SymbolKind::Class), name)
}
fn method(name: &str) -> ScopeFrame {
    frame(FrameKind::Sym(SymbolKind::Method), name)
}
fn param(name: &str) -> ScopeFrame {
    frame(FrameKind::Sym(SymbolKind::Parameter), name)
}
fn project() -> ScopeFrame {
    frame(FrameKind::Project, "<root>")
}

/// The chain that motivated the whole design: a parameter root. The enclosing
/// class must be found by KIND, not by a positional index — it sits at frame 2,
/// not `get(1)`.
#[test]
fn param_root_resolves_class_by_kind_not_position() {
    let scope = ContainingScope::new(vec![
        param("repo"),
        method("run"),
        class("App"),
        ns("app"),
        project(),
    ]);

    // enclosing_of_kind(is_type) finds App at index 2 — a `get(1)` would have
    // wrongly returned the method frame.
    assert_eq!(
        scope.enclosing_of_kind(FrameKind::is_type).map(|f| f.name.as_str()),
        Some("App")
    );
    // The immediately containing symbol is the method.
    assert_eq!(scope.containing().map(|f| f.name.as_str()), Some("run"));
    // The qualified name includes the namespace, omits the project root.
    assert_eq!(scope.to_qualified_name("."), "app.App.run.repo");
}

/// `enclosing_of_kind` includes self; `containing_of_kind` excludes it. For a
/// nested class the lexically-enclosing type is itself, but its *containing*
/// type (Roslyn `ContainingType`) is the outer class.
#[test]
fn enclosing_includes_self_containing_excludes_self() {
    let scope = ContainingScope::new(vec![
        class("Inner"),
        class("Outer"),
        ns("app"),
        project(),
    ]);

    assert_eq!(
        scope.enclosing_of_kind(FrameKind::is_type).map(|f| f.name.as_str()),
        Some("Inner")
    );
    assert_eq!(
        scope.containing_of_kind(FrameKind::is_type).map(|f| f.name.as_str()),
        Some("Outer")
    );
}

/// A method-source ref (the common chain-root case): scope_chain today is
/// `[class, package]` and `get(1)` returns the package — the latent bug. The
/// frame walk returns the class.
#[test]
fn method_source_finds_enclosing_class() {
    let scope = ContainingScope::new(vec![method("run"), class("App"), ns("app"), project()]);

    assert_eq!(
        scope.enclosing_of_kind(FrameKind::is_type).map(|f| f.name.as_str()),
        Some("App")
    );
    assert_eq!(
        scope.containing_of_kind(FrameKind::is_namespace).map(|f| f.name.as_str()),
        Some("app")
    );
}

#[test]
fn qualified_name_omits_project_and_joins_with_separator() {
    let scope = ContainingScope::new(vec![method("run"), class("App"), ns("app"), project()]);
    assert_eq!(scope.to_qualified_name("."), "app.App.run");
    assert_eq!(scope.to_qualified_name("::"), "app::App::run");
}

#[test]
fn ancestors_skip_self_chain_includes_self() {
    let scope = ContainingScope::new(vec![method("run"), class("App"), ns("app"), project()]);

    let chain: Vec<&str> = scope.chain().map(|f| f.name.as_str()).collect();
    assert_eq!(chain, vec!["run", "App", "app", "<root>"]);

    let ancestors: Vec<&str> = scope.ancestors().map(|f| f.name.as_str()).collect();
    assert_eq!(ancestors, vec!["App", "app", "<root>"]);
}

#[test]
fn empty_scope_has_no_own_or_containing() {
    let scope = ContainingScope::default();
    assert!(scope.is_empty());
    assert!(scope.own().is_none());
    assert!(scope.containing().is_none());
    assert!(scope.enclosing_of_kind(FrameKind::is_type).is_none());
    assert_eq!(scope.to_qualified_name("."), "");
}

// --- Builder ---------------------------------------------------------------

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

/// Java shape: the package is hoisted into the qname (class `parent_index` is
/// None, namespace `app` is a sibling symbol). The parameter's stored qname is
/// the *buggy* package-less form — the structural chain corrects it.
#[test]
fn java_param_chain_corrects_dropped_package() {
    let symbols = vec![
        esym("app", "app", SymbolKind::Namespace, None, None), // 0
        esym("App", "app.App", SymbolKind::Class, Some("app"), None), // 1
        esym("run", "app.App.run", SymbolKind::Method, Some("app.App"), Some(1)), // 2
        // 3: stored qname dropped the package (Bug 2). Parent is the method.
        esym("repo", "App.run.repo", SymbolKind::Parameter, Some("App.run"), Some(2)),
    ];

    // Non-parameter symbols serialize byte-identical to their stored qname.
    assert_eq!(build_containing_scope(&symbols, 0, None).to_qualified_name("."), "app");
    assert_eq!(build_containing_scope(&symbols, 1, None).to_qualified_name("."), "app.App");
    assert_eq!(build_containing_scope(&symbols, 2, None).to_qualified_name("."), "app.App.run");

    // The parameter's structural chain restores the package the stored qname
    // dropped — the Bug 2 fix, derived not hand-built.
    let repo = build_containing_scope(&symbols, 3, None);
    assert_eq!(repo.to_qualified_name("."), "app.App.run.repo");
    assert_ne!(repo.to_qualified_name("."), symbols[3].qualified_name);
    // And the chain resolves the enclosing class by kind, the method as parent.
    assert_eq!(repo.enclosing_of_kind(FrameKind::is_type).map(|f| f.name.as_str()), Some("App"));
    assert_eq!(repo.containing().map(|f| f.name.as_str()), Some("run"));
    assert_eq!(repo.containing_of_kind(FrameKind::is_namespace).map(|f| f.name.as_str()), Some("app"));
}

/// C# shape: the namespace is a real `parent_index` parent, so the chain is
/// complete structurally and no namespace tail is appended (no double frame).
#[test]
fn csharp_namespace_parent_not_double_appended() {
    let symbols = vec![
        esym("Eshop", "Eshop", SymbolKind::Namespace, None, None), // 0
        esym("Catalog", "Eshop.Catalog", SymbolKind::Class, Some("Eshop"), Some(0)), // 1
        esym("Get", "Eshop.Catalog.Get", SymbolKind::Method, Some("Eshop.Catalog"), Some(1)), // 2
    ];

    let get = build_containing_scope(&symbols, 2, None);
    assert_eq!(get.to_qualified_name("."), "Eshop.Catalog.Get");
    // Exactly one namespace frame — the parent, not a synthesized duplicate.
    assert_eq!(
        get.chain().filter(|f| f.kind.is_namespace()).count(),
        1
    );
}

/// Multi-segment package: the namespace tail splits into one frame per segment,
/// innermost first, serializing back to the dotted package prefix.
#[test]
fn multi_segment_package_tail() {
    let symbols = vec![esym(
        "Greeter",
        "com.fakeext.greeter.Greeter",
        SymbolKind::Class,
        Some("com.fakeext.greeter"),
        None,
    )];
    let scope = build_containing_scope(&symbols, 0, None);
    assert_eq!(scope.to_qualified_name("."), "com.fakeext.greeter.Greeter");
    assert_eq!(scope.containing_of_kind(FrameKind::is_namespace).map(|f| f.name.as_str()), Some("greeter"));
}

/// The optional project root caps the chain but never appears in the qname.
#[test]
fn project_root_is_capped_but_not_serialized() {
    let symbols = vec![esym("app", "app", SymbolKind::Namespace, None, None)];
    let scope = build_containing_scope(&symbols, 0, Some("my-proj"));
    assert_eq!(scope.to_qualified_name("."), "app");
    assert!(scope.chain().any(|f| f.kind == FrameKind::Project));
}

#[test]
fn build_scope_arena_maps_every_symbol() {
    let symbols = vec![
        esym("app", "app", SymbolKind::Namespace, None, None),
        esym("App", "app.App", SymbolKind::Class, Some("app"), None),
    ];
    let (arena, ids) = build_scope_arena(&symbols, None);
    assert_eq!(ids.len(), 2);
    assert_eq!(arena.get(ids[0]).to_qualified_name("."), "app");
    assert_eq!(arena.get(ids[1]).to_qualified_name("."), "app.App");
}

#[test]
fn arena_round_trips_scopes_by_handle() {
    let mut arena = ScopeArena::new();
    let a = arena.push(ContainingScope::new(vec![class("A"), ns("app"), project()]));
    let b = arena.push(ContainingScope::new(vec![class("B"), ns("app"), project()]));

    assert_ne!(a, b);
    assert_eq!(arena.len(), 2);
    assert_eq!(arena.get(a).own().map(|f| f.name.as_str()), Some("A"));
    assert_eq!(arena.get(b).to_qualified_name("."), "app.B");
}
