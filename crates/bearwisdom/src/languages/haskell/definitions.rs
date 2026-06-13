// =============================================================================
// languages/haskell/definitions.rs  —  Top-level declaration extractors
//
// Functions invoked by the main `visit` walker for each declaration kind:
//   function, data_type, newtype, class, instance, type_synomym/family,
//   import, foreign_import/export, signature.
// Plus the helpers each emitter uses (deriving, data constructor, record
// field, instance type extraction).
// =============================================================================

use crate::parser::scope_tree;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

use super::extract::{make_symbol, node_text};

// ---------------------------------------------------------------------------
// function  →  Function or Method
// ---------------------------------------------------------------------------

pub(super) fn extract_function(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    kind: SymbolKind,
    parent_index: Option<usize>,
) -> Option<usize> {
    // `name` field is optional in tree-sitter-haskell.
    // Try: name field → first variable/prefix_id child → first match child's name.
    let name = if let Some(n) = node.child_by_field_name("name") {
        let t = node_text(n, src);
        t.trim_matches(|c: char| c == '(' || c == ')').to_string()
    } else {
        // Try direct variable or prefix_id child
        let mut cursor = node.walk();
        let mut found = String::new();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "variable" | "prefix_id" => {
                    found = node_text(child, src)
                        .trim_matches(|c: char| c == '(' || c == ')')
                        .to_string();
                    break;
                }
                "infix" => {
                    // Infix-form operator definition: `a $$ b = ...`. The
                    // function name is the `operator` field of the infix node,
                    // not a variable child. Take the bare operator surface form
                    // so it matches the `$$` a call ref emits.
                    if let Some(op) = child.child_by_field_name("operator") {
                        found = node_text(op, src).trim_matches('`').to_string();
                        if !found.is_empty() {
                            break;
                        }
                    }
                }
                "match" => {
                    // match node contains the function name as first child
                    let mut mc = child.walk();
                    for mc_child in child.children(&mut mc) {
                        if mc_child.kind() == "variable" || mc_child.kind() == "prefix_id" {
                            found = node_text(mc_child, src)
                                .trim_matches(|c: char| c == '(' || c == ')')
                                .to_string();
                            break;
                        }
                    }
                    if !found.is_empty() {
                        break;
                    }
                }
                _ => {}
            }
        }
        found
    };
    let name = if !name.is_empty() {
        name
    } else {
        // Final fallback: use raw text of the first named child (truncated).
        // This handles pattern-only bindings like `(x, y) = ...` or `_ = ...`.
        let fallback = node
            .named_child(0)
            .map(|c| {
                let t = node_text(c, src);
                // Truncate to 40 chars to avoid huge names
                if t.len() > 40 {
                    t[..40].to_string()
                } else {
                    t
                }
            })
            .unwrap_or_default();
        if fallback.is_empty() {
            return None;
        }
        fallback
    };

    let scope = scope_tree::find_enclosing_scope(scope_tree, node.start_byte(), node.end_byte())
        .map(|s| s.qualified_name.clone());
    let qname = if let Some(p) = &scope {
        format!("{}.{}", p, name)
    } else {
        name.clone()
    };

    let idx = symbols.len();
    symbols.push(make_symbol(name, qname, kind, node, None, parent_index));
    // Attach scope_path
    if let Some(ref s) = scope {
        symbols[idx].scope_path = Some(s.clone());
    }
    Some(idx)
}

// ---------------------------------------------------------------------------
// bind  →  Variable (top-level / where-bound) or Method (in class / instance)
// ---------------------------------------------------------------------------
//
// A nullary binding (`answer = 42`, `($$) = ...`, a no-parameter where-local
// `g = ...`) parses as a `bind` node, distinct from the `function` node that
// carries parameter patterns. Its `name` field is a `variable` or — for an
// operator binding — a parenthesized `prefix_id`. Surface it as a callable
// symbol so refs to the bound identifier resolve; operator names are stripped
// to their bare surface form so they match the call refs an `infix` emits.

pub(super) fn extract_bind(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    kind: SymbolKind,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node
        .child_by_field_name("name")
        .or_else(|| node.named_child(0))?;
    let name = node_text(name_node, src)
        .trim_matches(|c: char| c == '(' || c == ')' || c == '`')
        .to_string();
    if name.is_empty() {
        return None;
    }

    let scope = scope_tree::find_enclosing_scope(scope_tree, node.start_byte(), node.end_byte())
        .map(|s| s.qualified_name.clone());
    let qname = if let Some(p) = &scope {
        format!("{}.{}", p, name)
    } else {
        name.clone()
    };

    let idx = symbols.len();
    symbols.push(make_symbol(name, qname, kind, node, None, parent_index));
    if let Some(ref s) = scope {
        symbols[idx].scope_path = Some(s.clone());
    }
    Some(idx)
}

// ---------------------------------------------------------------------------
// data_type / newtype / class / type_synomym  →  named symbol
// ---------------------------------------------------------------------------

pub(super) fn extract_named_symbol(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    kind: SymbolKind,
    keyword: &str,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);
    if name.is_empty() {
        return None;
    }

    let scope = scope_tree::find_enclosing_scope(scope_tree, node.start_byte(), node.end_byte())
        .map(|s| s.qualified_name.clone());
    let qname = if let Some(p) = &scope {
        format!("{}.{}", p, name)
    } else {
        name.clone()
    };

    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        qname,
        kind,
        node,
        Some(format!("{} {} ...", keyword, name)),
        parent_index,
    ));
    if let Some(ref s) = scope {
        symbols[idx].scope_path = Some(s.clone());
    }
    Some(idx)
}

// ---------------------------------------------------------------------------
// instance  →  Class + Implements edge
// ---------------------------------------------------------------------------

pub(super) fn extract_instance(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> Option<usize> {
    // instance [context =>] ClassName Type where ...
    // The type class name is in the `name` field
    let class_name_node = node.child_by_field_name("name")?;
    let class_name = node_text(class_name_node, src);
    if class_name.is_empty() {
        return None;
    }

    // The type being instantiated is in `patterns` or surrounding children
    let type_name = extract_instance_type(node, src);
    let instance_name = if type_name.is_empty() {
        class_name.clone()
    } else {
        format!("{} {}", class_name, type_name)
    };

    let scope = scope_tree::find_enclosing_scope(scope_tree, node.start_byte(), node.end_byte())
        .map(|s| s.qualified_name.clone());
    let idx = symbols.len();
    symbols.push(make_symbol(
        instance_name.clone(),
        instance_name,
        SymbolKind::Class,
        node,
        Some(format!("instance {} {}", class_name, type_name)),
        parent_index,
    ));
    if let Some(ref s) = scope {
        symbols[idx].scope_path = Some(s.clone());
    }

    // Implements edge: this type instance → the type class
    let source_idx = idx;
    refs.push(ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: source_idx,
        target_name: class_name,
        kind: EdgeKind::Implements,
        line: node.start_position().row as u32,
        col: 0,
        module: None,
        chain: None,
        byte_offset: node.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });

    Some(idx)
}

fn extract_instance_type(node: &Node, src: &[u8]) -> String {
    // Walk children after `name` field to find type identifiers
    let mut found_name = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if node.child_by_field_name("name").map(|n| n.id()) == Some(child.id()) {
            found_name = true;
            continue;
        }
        if found_name
            && (child.kind() == "name"
                || child.kind() == "constructor"
                || child.kind() == "variable")
        {
            let t = node_text(child, src);
            if !t.is_empty() {
                return t;
            }
        }
    }
    String::new()
}

// ---------------------------------------------------------------------------
// deriving  →  Implements edges
// ---------------------------------------------------------------------------

pub(super) fn extract_deriving(
    node: &Node,
    src: &[u8],
    parent_idx: Option<usize>,
    refs: &mut Vec<ExtractedRef>,
) {
    let source_idx = match parent_idx {
        Some(i) => i,
        None => return,
    };
    // Look for `deriving` child node
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "deriving" {
            // Collect all class names from the deriving clause.
            // tree-sitter-haskell grammar (v0.25+) wraps the list in a `tuple`
            // node (for `deriving (Show, Eq)`) or emits a single `name`/`constructor`
            // directly (for `deriving Show`).
            collect_deriving_names(&child, src, source_idx, refs);
        }
    }
}

/// Recursively collect `name`/`constructor`/`class` tokens from a `deriving` node
/// or any of its container wrappers (`tuple`, `list`, `class`, etc.).
fn collect_deriving_names(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "name" | "constructor" => {
                let name = node_text(child, src);
                if !name.is_empty() && name != "deriving" {
                    refs.push(ExtractedRef {
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index: source_idx,
                        target_name: name,
                        kind: EdgeKind::Implements,
                        line: child.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: child.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                }
            }
            "deriving" | "tuple" | "list" | "class" | "qualified" => {
                // Recurse into wrapper nodes
                collect_deriving_names(&child, src, source_idx, refs);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// data_constructor children  →  EnumMember symbols
// ---------------------------------------------------------------------------

pub(super) fn extract_data_constructors(
    data_node: &Node,
    src: &[u8],
    parent_idx: Option<usize>,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    // Walk all descendants looking for `data_constructor` or `gadt_constructor` nodes.
    collect_constructors(data_node, src, parent_idx, symbols);
}

fn collect_constructors(
    node: &Node,
    src: &[u8],
    parent_idx: Option<usize>,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "data_constructor" | "gadt_constructor" => {
                // The constructor name is inside a `prefix` child (or a direct
                // `constructor`/`name` child in some grammar versions).
                // Use a depth-first search limited to 3 levels to find the first
                // `constructor` node.
                if let Some(name) = find_constructor_name(&child, src, 3) {
                    symbols.push(make_symbol(
                        name.clone(),
                        name,
                        SymbolKind::EnumMember,
                        &child,
                        None,
                        parent_idx,
                    ));
                }
                // Record-syntax constructors (`Foo { field :: T }`) generate
                // field accessor functions visible to any importer. Emit them
                // as Function symbols so the resolver can bind calls to them.
                collect_record_field_names(&child, src, parent_idx, symbols);
            }
            _ => {
                collect_constructors(&child, src, parent_idx, symbols);
            }
        }
    }
}

/// Walk a `data_constructor` node looking for `field_name` nodes inside any
/// `record` → `fields` → `field` subtree. Each `field_name` child `variable`
/// is the record accessor function name.
fn collect_record_field_names(
    node: &Node,
    src: &[u8],
    parent_idx: Option<usize>,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "field_name" => {
                // field_name contains a single variable child.
                if let Some(var) = child.named_child(0) {
                    let name = node_text(var, src);
                    if !name.is_empty() {
                        symbols.push(make_symbol(
                            name.clone(),
                            name,
                            SymbolKind::Function,
                            &child,
                            None,
                            parent_idx,
                        ));
                    }
                }
            }
            "record" | "fields" | "field" | "prefix" => {
                collect_record_field_names(&child, src, parent_idx, symbols);
            }
            _ => {
                collect_record_field_names(&child, src, parent_idx, symbols);
            }
        }
    }
}

/// Depth-limited search for a constructor name inside a data/gadt
/// constructor node. Returns the first match for any of:
///   * `constructor` — prefix form (`Just a`, `Nothing`)
///   * `constructor_operator` — operator form inside an `infix` child
///     (`a : List a` → `:`)
///   * `empty_list` / `unit_constructor` — special syntactic
///     constructors built into the grammar (`[]`, `()`)
fn find_constructor_name(node: &Node, src: &[u8], depth: usize) -> Option<String> {
    if depth == 0 {
        return None;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "constructor" | "constructor_operator" => {
                let name = node_text(child, src);
                if !name.is_empty() {
                    return Some(name);
                }
            }
            "empty_list" => return Some("[]".to_string()),
            "unit_constructor" => return Some("()".to_string()),
            _ => {}
        }
        // Recurse into wrapper nodes like `prefix`, `infix`, `special`.
        if let Some(name) = find_constructor_name(&child, src, depth - 1) {
            return Some(name);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// import  →  Imports edge
// ---------------------------------------------------------------------------

pub(super) fn extract_import(
    node: &Node,
    src: &[u8],
    symbols: &[ExtractedSymbol],
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or_else(|| symbols.len().saturating_sub(1));
    // module field = the module being imported
    let module_node = match node.child_by_field_name("module") {
        Some(n) => n,
        None => return,
    };
    let module = node_text(module_node, src);
    if module.is_empty() {
        return;
    }
    // `alias` field is set by `import qualified M as A` — the `as A` part.
    // When present, `target_name` carries the alias so the resolver can map
    // qualified calls like `A.foo` to the full module `M.foo`.
    let alias_text = node
        .child_by_field_name("alias")
        .map(|n| node_text(n, src))
        .filter(|s| !s.is_empty());
    let target_name = match &alias_text {
        Some(a) => a.clone(),
        None => module.rsplit('.').next().unwrap_or(&module).to_string(),
    };
    refs.push(ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: source_idx,
        target_name,
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        col: 0,
        module: Some(module),
        chain: None,
        byte_offset: node.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
}

// ---------------------------------------------------------------------------
// foreign_import / foreign_export  →  Function
// ---------------------------------------------------------------------------

pub(super) fn extract_foreign(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    // foreign_import / foreign_export contains a `signature` child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "signature" {
            let name_node = child
                .child_by_field_name("name")
                .or_else(|| child.named_child(0))?;
            let name = node_text(name_node, src);
            if name.is_empty() {
                return None;
            }
            let scope =
                scope_tree::find_enclosing_scope(scope_tree, node.start_byte(), node.end_byte())
                    .map(|s| s.qualified_name.clone());
            let qname = if let Some(p) = &scope {
                format!("{}.{}", p, name)
            } else {
                name.clone()
            };
            let idx = symbols.len();
            symbols.push(make_symbol(
                name,
                qname,
                SymbolKind::Function,
                node,
                None,
                parent_index,
            ));
            if let Some(ref s) = scope {
                symbols[idx].scope_path = Some(s.clone());
            }
            return Some(idx);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// signature  →  Method (inside class) or Function (top-level)
// ---------------------------------------------------------------------------
//
// Tree-sitter Haskell models `(<>) :: a -> a -> a` as a `signature` node
// whose name field holds the bound identifier(s). Multiple identifiers can
// share a signature: `foo, bar :: Int` declares both. The resolver needs
// every identifier surfaced as its own symbol so chain lookups land.

/// Collect the constraint-introduced type variables a signature declares.
///
/// `f :: Ord a => a -> a -> Bool` introduces tyvar `a` (the lowercase
/// `variable` argument of the constraint `Ord a`); `Ord` is the class name
/// (an uppercase `name`) and is NOT a tyvar. The tree shape is
/// `signature` → field `type` → `context` (field `context` = the constraint
/// expression) or `forall` (field `variables` = explicit tyvars, field `type`
/// = the inner `context`). A constraint expression is an `apply` (single
/// `Ord a`), a `tuple` (`(Ord a, Show b)`), or `parens` (`(Eq a)`); in every
/// shape the tyvars are exactly the lowercase `variable` leaves, so collecting
/// all `variable` node texts under the constraint subtree is the sound rule.
/// Order-preserving, deduplicated.
fn collect_constraint_tyvars(sig_node: &Node, src: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Some(type_node) = sig_node.child_by_field_name("type") else {
        return out;
    };

    // `forall a b. Ctx => ...` — the quantifier names the explicit tyvars.
    if type_node.kind() == "forall" {
        if let Some(vars) = type_node.child_by_field_name("variables") {
            collect_variable_leaves(&vars, src, &mut out);
        }
        // Descend into the quantified body for any constraint tyvars the
        // forall list might have omitted (superset stays sound).
        if let Some(inner) = type_node.child_by_field_name("type") {
            collect_context_tyvars(&inner, src, &mut out);
        }
        return out;
    }

    collect_context_tyvars(&type_node, src, &mut out);
    out
}

/// When `node` is a `context` (`Ctx => Type`), collect tyvars from its
/// `context` field (the constraint expression). Other node kinds contribute
/// nothing — a signature with no constraint has no constraint tyvars.
fn collect_context_tyvars(node: &Node, src: &[u8], out: &mut Vec<String>) {
    if node.kind() != "context" {
        return;
    }
    if let Some(constraint) = node.child_by_field_name("context") {
        collect_variable_leaves(&constraint, src, out);
    }
}

/// Recursively collect lowercase `variable` leaf texts, skipping class names
/// (`name` / `constructor` / `qualified`, all uppercase by Haskell rule).
/// Wrapper nodes (`apply`, `tuple`, `parens`, `context`) are traversed.
fn collect_variable_leaves(node: &Node, src: &[u8], out: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "variable" => {
                let t = node_text(child, src);
                if !t.is_empty() && !out.iter().any(|v| v == &t) {
                    out.push(t);
                }
            }
            // Class names — never tyvars. Don't descend (a qualified class
            // name has no tyvar children).
            "name" | "constructor" | "qualified" | "operator" => {}
            _ => collect_variable_leaves(&child, src, out),
        }
    }
}

pub(super) fn extract_signature_symbols(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    kind: SymbolKind,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    let mut names: Vec<String> = Vec::new();

    // tree-sitter-haskell uses the `names` field (plural) when a single
    // signature declares more than one identifier — `(+), (-), (*) :: ...`
    // — and `name` (singular) for single-identifier signatures. Try both.
    let name_field = node
        .child_by_field_name("names")
        .or_else(|| node.child_by_field_name("name"));

    if let Some(named) = name_field {
        if matches!(
            named.kind(),
            "binding_list" | "names" | "name_list" | "infix_id" | "tuple"
        ) {
            let mut nc = named.walk();
            for grandchild in named.children(&mut nc) {
                if matches!(
                    grandchild.kind(),
                    "variable" | "operator" | "operator_name" | "prefix_id" | "name"
                ) {
                    let t = node_text(grandchild, src);
                    let trimmed = t.trim_matches(|c: char| c == '(' || c == ')').to_string();
                    if !trimmed.is_empty() {
                        names.push(trimmed);
                    }
                }
            }
        } else {
            let t = node_text(named, src);
            let trimmed = t.trim_matches(|c: char| c == '(' || c == ')').to_string();
            if !trimmed.is_empty() {
                names.push(trimmed);
            }
        }
    }

    if names.is_empty() {
        // Fallback: walk children directly for variable / operator tokens
        // before the `::`. tree-sitter sometimes flattens names without a
        // `name` field on simple signatures.
        for child in node.children(&mut cursor) {
            match child.kind() {
                "variable" | "prefix_id" | "operator" | "operator_name" => {
                    let t = node_text(child, src);
                    let trimmed = t.trim_matches(|c: char| c == '(' || c == ')').to_string();
                    if !trimmed.is_empty() {
                        names.push(trimmed);
                    }
                }
                "::" | "type" | "context" | "fun" | "function_type" => break,
                _ => {}
            }
        }
    }

    if names.is_empty() {
        return;
    }

    let scope = scope_tree::find_enclosing_scope(scope_tree, node.start_byte(), node.end_byte())
        .map(|s| s.qualified_name.clone());

    // Constraint-introduced type variables become generic params of every
    // identifier this signature declares. They ride on a leading `<a, b>`
    // signature clause — the channel the index build's generic-param scan
    // already reads for every `<>`/`[]` language — so the shared
    // `engine_generic_param` rung resolves their occurrences with no
    // Haskell-specific resolver code.
    let tyvars = collect_constraint_tyvars(node, src);
    let signature = if tyvars.is_empty() {
        None
    } else {
        Some(format!("<{}>", tyvars.join(", ")))
    };

    for name in names {
        let qname = if let Some(p) = &scope {
            format!("{}.{}", p, name)
        } else {
            name.clone()
        };
        let idx = symbols.len();
        symbols.push(make_symbol(
            name,
            qname,
            kind,
            node,
            signature.clone(),
            parent_index,
        ));
        if let Some(ref s) = scope {
            symbols[idx].scope_path = Some(s.clone());
        }
        // Emit the tyvar's constraint-clause occurrence as a TypeRef sourced
        // from this symbol, so the generic-param bind is observable as an edge.
        for tv in &tyvars {
            refs.push(ExtractedRef {
                is_import_binding: false,
                is_reexport: false,
                source_symbol_index: idx,
                target_name: tv.clone(),
                kind: EdgeKind::TypeRef,
                line: node.start_position().row as u32,
                col: 0,
                module: None,
                chain: None,
                byte_offset: node.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            });
        }
    }
}
