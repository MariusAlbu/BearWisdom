use std::path::Path;

use tempfile::TempDir;

use super::*;
use crate::db::Database;
use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::type_checker::core::types::TypeArena;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, SymbolKind, Visibility};

fn module_symbol(name: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Module,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

/// A `use ExternalMacro` ref, module-tagged so `materialize_externals`'s
/// demand pull locates and parses the external file that defines it — the
/// same shape a real `use`/`alias` directive's extracted ref carries.
fn use_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Imports,
        line: 1,
        col: 0,
        module: Some(target.to_string()),
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

/// The demand pull inside `materialize_and_build_tree` surfaces external
/// files that never went through the caller's pre-resolve plugin-state
/// phases. `resolve_with_plugin_refresh` must fold that batch back in, rerun
/// `populate_post_externals` + `synthesize_and_persist`, and persist the
/// member the external file's `__using__` macro injects — Elixir's
/// `ElixirPlugin` end to end, no synthetic plugin needed, since the gap is in
/// the orchestration around plugin hooks, not in any one plugin's logic.
#[test]
fn demand_pulled_external_using_macro_synthesizes_member() {
    let dir = TempDir::new().unwrap();
    let external_path = dir.path().join("external_macro.ex");
    std::fs::write(
        &external_path,
        r#"
defmodule ExternalMacro do
  defmacro __using__(_opts) do
    quote do
      def injected, do: :ok
    end
  end
end
"#,
    )
    .unwrap();

    let mut loc = SymbolLocationIndex::new();
    loc.insert("ExternalMacro", "ExternalMacro", external_path.clone());

    let consumer = ParsedFile {
        path: "lib/consumer.ex".to_string(),
        language: "elixir".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: vec![module_symbol("Consumer")],
        refs: vec![use_ref("ExternalMacro")],
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: Some("defmodule Consumer do\n  use ExternalMacro\nend\n".to_string()),
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    };

    let mut db = Database::open_in_memory().unwrap();
    let (_files, mut symbol_id_map) =
        crate::indexer::write::write_parsed_files_with_origin(&db, std::slice::from_ref(&consumer), "internal", None)
            .unwrap();

    let mut project_ctx = ProjectContext::default();
    project_ctx.language_presence.insert("elixir".to_string());

    let mut parsed = vec![consumer];
    let arena = std::sync::Arc::new(TypeArena::new());
    let registry = crate::languages::default_registry();

    resolve_with_plugin_refresh(
        &mut db,
        &mut parsed,
        &mut symbol_id_map,
        &mut project_ctx,
        registry,
        Path::new("."),
        arena,
        std::sync::Arc::new(loc),
    )
    .unwrap();

    let synthesized: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE qualified_name = 'Consumer.injected'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        synthesized, 1,
        "the demand-pulled external's __using__ macro should synthesize Consumer.injected"
    );
}
