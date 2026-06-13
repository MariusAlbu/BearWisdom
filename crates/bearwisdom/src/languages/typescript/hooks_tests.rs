// =============================================================================
// languages/typescript/hooks_tests.rs — sanity checks for the hook skeleton.
// =============================================================================

use super::*;
use crate::languages::typescript::extract;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::FlowMeta;

#[test]
fn typescript_hooks_is_send_sync() {
    fn require_send_sync<T: Send + Sync + ?Sized>() {}
    require_send_sync::<TypeScriptHooks>();
    require_send_sync::<dyn LanguageEngineHooks>();
}

/// Build a `ParsedFile` carrying the refs the TS extractor produces for
/// `source`, so the import table `build_file_context_inner` derives can be
/// asserted end to end.
fn parsed_file(source: &str) -> ParsedFile {
    let r = extract::extract(source, false);
    ParsedFile {
        path: "src/file.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: "h".to_string(),
        size: source.len() as u64,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: r.symbols,
        refs: r.refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

#[test]
fn import_type_bindings_surface_in_file_context() {
    let src = r#"
import type {
  Person as P,
  Calendar,
  CalendarEvent,
} from "@calcom/types/Calendar";
"#;
    let ctx = build_file_context_inner(&parsed_file(src), None);
    let has = |name: &str| {
        ctx.imports.iter().any(|e| {
            e.imported_name == name
                && e.module_path.as_deref() == Some("@calcom/types/Calendar")
        })
    };
    assert!(has("Person"), "imports: {:?}", ctx.imports);
    assert!(has("Calendar"), "imports: {:?}", ctx.imports);
    assert!(has("CalendarEvent"), "imports: {:?}", ctx.imports);
}
