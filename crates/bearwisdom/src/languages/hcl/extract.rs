// =============================================================================
// languages/hcl/extract.rs  —  HCL / Terraform extractor
//
// What we extract
// ---------------
// SYMBOLS (Terraform semantic mapping):
//   Class     — block type=resource ("type.name"), block type=data ("data.type.name"),
//               block type=module ("module.name"), block type=provider ("provider.name")
//   Variable  — block type=variable, block type=output, block type=provider
//               attribute inside locals block
//   Namespace — block type=terraform
//
// REFERENCES:
//   TypeRef   — variable_expr root + get_attr chain ("var.x", "local.x", "module.x")
//   Imports   — module block source attribute value
//   Calls     — function_call identifier
//
// Grammar: tree-sitter-hcl (not yet in Cargo.toml — ready for when added).
// Node names follow the HCL grammar: config_file → body → block / attribute.
// Block: first identifier child = block type; subsequent string_lit/identifier = labels.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from an HCL/Terraform document.
///
/// Requires the tree-sitter-hcl grammar to be available as `language`.
/// Called by `HclPlugin::extract()` once the grammar is wired in.
#[allow(dead_code)]
pub fn extract(source: &str, language: tree_sitter::Language) -> crate::types::ExtractionResult {
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load HCL grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return crate::types::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit_body(tree.root_node(), source, &mut symbols, &mut refs, None);

    crate::types::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Body traversal
// ---------------------------------------------------------------------------

/// Visit a `config_file` or `body` node, extracting top-level blocks and
/// attributes. The `locals_parent` is set when we're inside a `locals` block
/// so attribute children are emitted as Variables.
fn visit_body(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "config_file" | "body" => {
                visit_body(child, src, symbols, refs, parent_idx);
            }
            "block" => {
                extract_block(&child, src, symbols, refs);
            }
            "attribute" => {
                // Extract ALL attributes as symbols (not just locals).
                extract_attribute(&child, src, parent_idx, symbols, refs);
            }
            _ => {}
        }
    }
}

/// Extract any attribute as a Variable symbol + refs from its value.
fn extract_attribute(
    node: &Node,
    src: &str,
    parent_idx: Option<usize>,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match first_identifier_text(node, src) {
        Some(n) => n,
        None => return,
    };
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Variable,
        node,
        None,
        parent_idx,
    ));
    extract_refs_in_subtree(node, src, idx, refs);
}

// ---------------------------------------------------------------------------
// Block extraction
// ---------------------------------------------------------------------------

/// Extract a top-level HCL block. Uses the block type (first identifier child)
/// to determine the SymbolKind and qualified name strategy per Terraform semantics.
fn extract_block(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // Collect all identifier and string_lit children to get block_type + labels.
    let (block_type, labels) = collect_block_type_and_labels(node, src);
    let block_type = match block_type {
        Some(t) => t,
        None => return,
    };

    match block_type.as_str() {
        "resource" => extract_resource_block(node, src, &labels, symbols, refs),
        "data" => extract_data_block(node, src, &labels, symbols, refs),
        "variable" => extract_variable_block(node, src, &labels, symbols, refs),
        "output" => extract_output_block(node, src, &labels, symbols, refs),
        "module" => extract_module_block(node, src, &labels, symbols, refs),
        "provider" => extract_provider_block(node, src, &labels, symbols, refs),
        "terraform" => extract_terraform_block(node, src, symbols, refs),
        "locals" => extract_locals_block(node, src, symbols, refs),
        _ => {
            // Generic block — emit as Variable with block_type as name prefix
            let name = if labels.is_empty() {
                block_type.clone()
            } else {
                format!("{}.{}", block_type, labels.join("."))
            };
            let idx = symbols.len();
            symbols.push(make_symbol(
                name.clone(),
                name,
                SymbolKind::Variable,
                node,
                Some(format!("{} {{...}}", block_type)),
                None,
            ));
            // Extract refs within the block body
            extract_block_refs(node, src, idx, symbols, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// resource "<type>" "<name>" { ... }  →  Class "type.name"
// ---------------------------------------------------------------------------

fn extract_resource_block(
    node: &Node,
    src: &str,
    labels: &[String],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let (res_type, res_name) = match (labels.first(), labels.get(1)) {
        (Some(t), Some(n)) => (t.clone(), n.clone()),
        (Some(t), None) => (t.clone(), String::new()),
        _ => return,
    };

    let name = if res_name.is_empty() {
        res_type.clone()
    } else {
        format!("{}.{}", res_type, res_name)
    };
    let sig = format!("resource \"{}\" \"{}\"", res_type, res_name);

    let idx = symbols.len();
    symbols.push(make_symbol(name.clone(), name, SymbolKind::Class, node, Some(sig), None));
    extract_block_refs(node, src, idx, symbols, refs);
}

// ---------------------------------------------------------------------------
// data "<type>" "<name>" { ... }  →  Class "data.type.name"
// ---------------------------------------------------------------------------

fn extract_data_block(
    node: &Node,
    src: &str,
    labels: &[String],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let (data_type, data_name) = match (labels.first(), labels.get(1)) {
        (Some(t), Some(n)) => (t.clone(), n.clone()),
        (Some(t), None) => (t.clone(), String::new()),
        _ => return,
    };

    let name = if data_name.is_empty() {
        format!("data.{}", data_type)
    } else {
        format!("data.{}.{}", data_type, data_name)
    };
    let sig = format!("data \"{}\" \"{}\"", data_type, data_name);

    let idx = symbols.len();
    symbols.push(make_symbol(name.clone(), name, SymbolKind::Class, node, Some(sig), None));
    extract_block_refs(node, src, idx, symbols, refs);
}

// ---------------------------------------------------------------------------
// variable "<name>" { ... }  →  Variable
// ---------------------------------------------------------------------------

fn extract_variable_block(
    node: &Node,
    src: &str,
    labels: &[String],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match labels.first() {
        Some(n) => n.clone(),
        None => return,
    };
    let sig = format!("variable \"{}\"", name);
    let idx = symbols.len();
    symbols.push(make_symbol(name.clone(), name, SymbolKind::Variable, node, Some(sig), None));
    extract_block_refs(node, src, idx, symbols, refs);
}

// ---------------------------------------------------------------------------
// output "<name>" { ... }  →  Variable
// ---------------------------------------------------------------------------

fn extract_output_block(
    node: &Node,
    src: &str,
    labels: &[String],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match labels.first() {
        Some(n) => n.clone(),
        None => return,
    };
    let sig = format!("output \"{}\"", name);
    let idx = symbols.len();
    symbols.push(make_symbol(name.clone(), name, SymbolKind::Variable, node, Some(sig), None));
    extract_block_refs(node, src, idx, symbols, refs);
}

// ---------------------------------------------------------------------------
// module "<name>" { source = "..." }  →  Namespace + Imports ref
// ---------------------------------------------------------------------------

fn extract_module_block(
    node: &Node,
    src: &str,
    labels: &[String],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let label = match labels.first() {
        Some(n) => n.clone(),
        None => return,
    };
    let name = format!("module.{}", label);
    let sig = format!("module \"{}\"", label);

    let idx = symbols.len();
    symbols.push(make_symbol(name.clone(), name, SymbolKind::Namespace, node, Some(sig), None));

    // Look for `source = "..."` attribute in the block body — emit as Imports
    if let Some(source_val) = find_attribute_value(node, src, "source") {
        refs.push(ExtractedRef {
            source_symbol_index: idx,
            target_name: source_val.clone(),
            kind: EdgeKind::Imports,
            line: node.start_position().row as u32,
            module: Some(source_val),
            chain: None,
            byte_offset: 0,
        });
    }

    extract_block_refs(node, src, idx, symbols, refs);
}

// ---------------------------------------------------------------------------
// provider "<name>" { ... }  →  Class
// ---------------------------------------------------------------------------

fn extract_provider_block(
    node: &Node,
    src: &str,
    labels: &[String],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match labels.first() {
        Some(n) => n.clone(),
        None => return,
    };
    let sig = format!("provider \"{}\"", name);
    let idx = symbols.len();
    symbols.push(make_symbol(name.clone(), name, SymbolKind::Class, node, Some(sig), None));
    extract_block_refs(node, src, idx, symbols, refs);
}

// ---------------------------------------------------------------------------
// terraform { ... }  →  Namespace
// ---------------------------------------------------------------------------

fn extract_terraform_block(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let _ = src;
    let idx = symbols.len();
    symbols.push(make_symbol(
        "terraform".to_string(),
        "terraform".to_string(),
        SymbolKind::Namespace,
        node,
        Some("terraform { ... }".to_string()),
        None,
    ));
    extract_block_refs(node, src, idx, symbols, refs);
}

// ---------------------------------------------------------------------------
// locals { key = value ... }  →  one Variable per attribute
// ---------------------------------------------------------------------------

fn extract_locals_block(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // The locals block itself is a synthetic scope — emit a synthetic parent
    // Variable so children have a parent_index to attach to.
    let parent_idx = symbols.len();
    symbols.push(make_symbol(
        "locals".to_string(),
        "locals".to_string(),
        SymbolKind::Namespace,
        node,
        Some("locals { ... }".to_string()),
        None,
    ));

    // Walk the body child and extract attribute children
    visit_body(*node, src, symbols, refs, Some(parent_idx));
}

// ---------------------------------------------------------------------------
// Reference extraction within block bodies
// ---------------------------------------------------------------------------

/// Scan a block node's body for nested blocks, attributes, and refs.
fn extract_block_refs(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "body" {
            // Recurse into body — handles nested blocks and attributes
            visit_body(child, src, symbols, refs, Some(source_symbol_index));
        }
    }
}

/// Recursively scan a subtree for reference-producing nodes.
///
/// When an `expression` node is encountered it is handed off to
/// `extract_reference_chain`, which builds a single compound target like
/// `"var.region"`, `"data.aws_ami.ubuntu"`, or `"aws_instance.web"`.  That
/// function consumes the expression entirely, so we do not recurse further
/// into its children (avoids the old fragmented emission of `"var"` and
/// `"region"` as separate refs).
///
/// `function_call` nodes emit a `Calls` edge and their argument subtrees are
/// still scanned for nested refs.  All other node kinds are traversed
/// transparently.
fn extract_refs_in_subtree(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "expression" => {
            // Intercept here: build one compound ref for the whole chain and
            // do not recurse into children (the chain builder reads them).
            extract_reference_chain(node, src, source_symbol_index, refs);
            return;
        }
        "function_call" => {
            extract_function_call_ref(node, src, source_symbol_index, refs);
            // Fall through to recurse into arguments — they may contain refs.
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_refs_in_subtree(&child, src, source_symbol_index, refs);
    }
}

/// Extract a Terraform reference chain from an `expression` node.
///
/// The grammar represents `var.region`, `data.aws_ami.ubuntu.id`, and
/// `aws_instance.web.public_ip` as:
///
/// ```text
/// expression
///   variable_expr { identifier: "var" }
///   get_attr { identifier: "region" }
/// ```
///
/// We build a single `target_name` that matches how the extractor names the
/// corresponding symbol, so the resolver can do a direct lookup:
///
/// | Expression          | target_name emitted     | Symbol qname          |
/// |---------------------|-------------------------|-----------------------|
/// | `var.region`        | `"var.region"`          | `"region"` (Variable) |
/// | `local.env`         | `"local.env"`           | `"env"` (Variable)    |
/// | `module.vpc.vpc_id` | `"module.vpc"`          | `"module.vpc"` (NS)   |
/// | `data.aws_ami.u.id` | `"data.aws_ami.ubuntu"` | `"data.aws_ami.ubuntu"` (Class) |
/// | `aws_instance.web`  | `"aws_instance.web"`    | `"aws_instance.web"` (Class) |
///
/// Terraform meta-roots (`each`, `count`, `self`, `path`, `terraform`) are
/// silently dropped — the resolver classifies them as external.
fn extract_reference_chain(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    let mut root: Option<String> = None;
    let mut attrs: Vec<String> = Vec::new();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "variable_expr" => {
                root = Some(node_text(child, src));
            }
            "get_attr" => {
                if let Some(ident) = first_identifier_text(&child, src) {
                    attrs.push(ident);
                }
            }
            "function_call" => {
                // A function call inside an expression (e.g. `tostring(var.x)`)
                // — emit a Calls ref for the function name, then recurse into
                // the argument subtrees so nested refs (like `var.x`) are also
                // captured.  We use extract_refs_in_subtree here because the
                // arguments may themselves contain expressions that need chain
                // extraction.
                extract_function_call_ref(&child, src, source_symbol_index, refs);
                // Recurse into function arguments only
                let mut fc_cursor = child.walk();
                for fc_child in child.children(&mut fc_cursor) {
                    if fc_child.kind() == "function_arguments" {
                        extract_refs_in_subtree(&fc_child, src, source_symbol_index, refs);
                    }
                }
            }
            _ => {}
        }
    }

    let root = match root {
        Some(r) => r,
        None => return,
    };

    // Terraform meta-references — no project symbol to resolve against.
    if matches!(root.as_str(), "each" | "count" | "self" | "path" | "terraform") {
        return;
    }

    // Build the canonical target_name that matches the index symbol's
    // qualified_name so the resolver can look it up directly.
    let target = match root.as_str() {
        // var.name → "var.name"  (resolver strips "var." → looks for Variable "name")
        "var" => match attrs.first() {
            Some(a) => format!("var.{}", a),
            None => return,
        },
        // local.name → "local.name"  (resolver strips "local." → Variable "name")
        "local" => match attrs.first() {
            Some(a) => format!("local.{}", a),
            None => return,
        },
        // module.name[.output] → "module.name"  (Namespace symbol qname)
        "module" => match attrs.first() {
            Some(a) => format!("module.{}", a),
            None => return,
        },
        // data.type.name[.attr] → "data.type.name"  (Class symbol qname)
        "data" => match (attrs.first(), attrs.get(1)) {
            (Some(t), Some(n)) => format!("data.{}.{}", t, n),
            (Some(t), None) => format!("data.{}", t),
            _ => return,
        },
        // resource_type.name[.attr] → "resource_type.name"  (Class symbol qname)
        _ => match attrs.first() {
            Some(a) => format!("{}.{}", root, a),
            // Bare identifier — emit as-is (e.g. a local variable in a for expr)
            None => root.clone(),
        },
    };

    if target.is_empty() {
        return;
    }

    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: target,
        kind: EdgeKind::TypeRef,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
        byte_offset: 0,
    });
}

/// Emit a Calls edge for a function_call node.
fn extract_function_call_ref(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if let Some(name) = first_identifier_text(node, src) {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: name,
            kind: EdgeKind::Calls,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
            byte_offset: 0,
        });
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Collect the block type (first identifier child) and labels (subsequent
/// identifier or string_lit children, with quotes stripped) from a `block` node.
fn collect_block_type_and_labels(node: &Node, src: &str) -> (Option<String>, Vec<String>) {
    let mut block_type: Option<String> = None;
    let mut labels: Vec<String> = Vec::new();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                if block_type.is_none() {
                    block_type = Some(node_text(child, src));
                } else {
                    labels.push(node_text(child, src));
                }
            }
            "string_lit" => {
                // Strip surrounding quotes from labels like "aws_instance"
                let raw = node_text(child, src);
                let stripped = raw.trim_matches('"').to_string();
                labels.push(stripped);
            }
            "body" => break, // Stop before block body
            _ => {}
        }
    }

    (block_type, labels)
}

/// Find the string value of a named attribute in a block's body.
/// Returns the unquoted string value, or None if not found.
fn find_attribute_value(block_node: &Node, src: &str, attr_name: &str) -> Option<String> {
    let mut cursor = block_node.walk();
    for child in block_node.children(&mut cursor) {
        if child.kind() == "body" {
            let mut bc = child.walk();
            for attr in child.children(&mut bc) {
                if attr.kind() == "attribute" {
                    if let Some(key) = first_identifier_text(&attr, src) {
                        if key == attr_name {
                            // Find the string_lit value in the expression
                            if let Some(val) = find_string_literal_in_subtree(&attr, src) {
                                return Some(val);
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Recursively find the first string_lit text in a subtree (unquoted).
fn find_string_literal_in_subtree(node: &Node, src: &str) -> Option<String> {
    if node.kind() == "string_lit" || node.kind() == "template_literal" {
        let raw = node_text(*node, src);
        let stripped = raw.trim_matches('"').to_string();
        if !stripped.is_empty() {
            return Some(stripped);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(v) = find_string_literal_in_subtree(&child, src) {
            return Some(v);
        }
    }
    None
}

/// Get the first `identifier` child's text from a node.
fn first_identifier_text(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            let t = node_text(child, src);
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

fn make_symbol(
    name: String,
    qualified_name: String,
    kind: SymbolKind,
    node: &Node,
    signature: Option<String>,
    parent_index: Option<usize>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: None,
        scope_path: None,
        parent_index,
    }
}

fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}
