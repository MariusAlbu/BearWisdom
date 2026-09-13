use super::*;
use crate::types::{DbMappingSource, EdgeKind, ExtractedDbSet, ExtractedRef, ExtractedRoute};

const CONTAINER: &str = "Step";
const CONTAINER_QNAME: &str = "lib/std/Build/Step";

fn symbol(name: &str, kind: SymbolKind, parent_index: Option<usize>) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 1,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: parent_index.map(|_| "Inner".to_string()),
        parent_index,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn plain_ref(source_symbol_index: usize) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index,
        target_name: "mem".to_string(),
        kind: EdgeKind::Imports,
        line: 1,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        call_args: Vec::new(),
    }
}

/// Three symbols where index 2 is already a child of index 0, plus one ref
/// sourced from index 2.
fn seeded() -> ExtractionResult {
    ExtractionResult {
        symbols: vec![
            symbol("Inner", SymbolKind::Struct, None),
            symbol("run", SymbolKind::Function, None),
            symbol("field", SymbolKind::Field, Some(0)),
        ],
        refs: vec![plain_ref(2)],
        ..Default::default()
    }
}

fn container(name: &str) -> FileContainer<'_> {
    FileContainer {
        name,
        qualified_name: CONTAINER_QNAME,
        kind: SymbolKind::Struct,
        visibility: Visibility::Public,
        end_line: 9,
    }
}

#[test]
fn container_is_index_zero_and_owns_former_top_levels() {
    let mut result = seeded();
    materialize(&mut result, container(CONTAINER));

    assert_eq!(result.symbols.len(), 4);
    assert_eq!(result.symbols[0].name, CONTAINER);
    assert_eq!(result.symbols[0].qualified_name, CONTAINER_QNAME);
    assert_eq!(result.symbols[0].kind, SymbolKind::Struct);
    assert_eq!(result.symbols[0].parent_index, None);

    // The two former top-levels are adopted, scoped under the container's
    // qualified name.
    for idx in [1usize, 2] {
        assert_eq!(result.symbols[idx].parent_index, Some(0));
        assert_eq!(
            result.symbols[idx].scope_path.as_deref(),
            Some(CONTAINER_QNAME)
        );
    }

    // The symbol that already had a parent keeps it, shifted 0 -> 1.
    assert_eq!(result.symbols[3].parent_index, Some(1));
    assert_eq!(result.symbols[3].scope_path.as_deref(), Some("Inner"));

    // The ref still points at the symbol it was extracted from.
    assert_eq!(result.refs[0].source_symbol_index, 3);
    assert_eq!(result.symbols[3].name, "field");
}

#[test]
fn container_satisfies_symbol_index_contract() {
    let source = "const Inner = struct {\n    field: u8,\n};\nfn run() void {}\n";
    let mut result = seeded();
    materialize(&mut result, container(CONTAINER));

    let violations = crate::indexer::canonical_form::validate_extraction(
        result,
        source,
        "lib/std/Build/Step.zig",
        "zig",
    );
    let structural: Vec<_> = violations
        .iter()
        .filter(|v| v.code == "SYM-002" || v.code == "SYM-003")
        .collect();
    assert!(
        structural.is_empty(),
        "index contract broken: {structural:?}"
    );
}

#[test]
fn empty_container_name_is_a_no_op() {
    let mut result = seeded();
    materialize(&mut result, container(""));

    assert_eq!(result.symbols.len(), 3);
    assert_eq!(result.symbols[0].name, "Inner");
    assert_eq!(result.symbols[0].parent_index, None);
    assert_eq!(result.symbols[1].parent_index, None);
    assert_eq!(result.symbols[2].parent_index, Some(0));
    assert_eq!(result.refs[0].source_symbol_index, 2);
}

#[test]
fn path_stem_keeps_the_directory_and_drops_the_extension() {
    assert_eq!(path_stem("lib/std/Build/Step.zig"), "lib/std/Build/Step");
    assert_eq!(path_stem(r"lib\std\mem.zig"), "lib/std/mem");
    assert_eq!(path_stem("std.zig"), "std");
    assert_eq!(file_stem("lib/std/Build/Step.zig"), "Step");
    assert_eq!(file_stem(r"lib\std\Build\Step.zig"), "Step");
}

#[test]
fn routes_and_db_set_indices_shift() {
    let mut result = seeded();
    result.routes.push(ExtractedRoute {
        handler_symbol_index: 1,
        http_method: "GET".to_string(),
        template: "/x".to_string(),
    });
    result.db_sets.push(ExtractedDbSet {
        property_symbol_index: 2,
        entity_type: "Row".to_string(),
        table_name: "rows".to_string(),
        source: DbMappingSource::Convention,
    });

    materialize(&mut result, container(CONTAINER));

    assert_eq!(result.routes[0].handler_symbol_index, 2);
    assert_eq!(result.db_sets[0].property_symbol_index, 3);
}
