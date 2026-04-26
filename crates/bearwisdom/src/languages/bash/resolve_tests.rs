// Tests for bash/resolve.rs — shell source resolution helpers and integration.
//
// Ported from pre-restructure commit 8dcc438 (dangling in object store).

use super::{ends_with_path_suffix, shell_path_suffix, BashResolver};
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, SymbolIndex,
};
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, FlowMeta, ParsedFile, SymbolKind, Visibility,
};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Test fixtures
// ---------------------------------------------------------------------------

fn make_sh_file(
    path: &str,
    symbols: Vec<ExtractedSymbol>,
    refs: Vec<ExtractedRef>,
) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "bash".to_string(),
        content_hash: "x".to_string(),
        size: 100,
        line_count: 10,
        mtime: None,
        package_id: None,
        symbols,
        refs,
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    }
}

fn make_fn_sym(name: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 5,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

fn make_source_import(raw_path: &str) -> ExtractedRef {
    let stem = raw_path
        .rsplit('/')
        .next()
        .unwrap_or(raw_path)
        .trim_end_matches(".sh")
        .trim_end_matches(".bash")
        .to_string();
    ExtractedRef {
        source_symbol_index: 0,
        target_name: stem,
        kind: EdgeKind::Imports,
        line: 1,
        module: Some(raw_path.to_string()),
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
    }
}

fn make_calls_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 5,
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// shell_path_suffix helper
// ---------------------------------------------------------------------------

#[test]
fn shell_path_suffix_handles_variable_prefix() {
    assert_eq!(shell_path_suffix("$OSH/themes/foo.sh"), "themes/foo.sh");
    assert_eq!(shell_path_suffix("${OSH}/themes/foo.sh"), "themes/foo.sh");
    assert_eq!(shell_path_suffix("./lib/helpers.sh"), "lib/helpers.sh");
    assert_eq!(shell_path_suffix("../shared/util.sh"), "shared/util.sh");
    assert_eq!(shell_path_suffix("foo.sh"), "foo.sh");
    assert_eq!(shell_path_suffix("/etc/profile"), ""); // absolute → skip
    assert_eq!(shell_path_suffix("$VAR"), ""); // bare variable, no path
}

// ---------------------------------------------------------------------------
// ends_with_path_suffix helper
// ---------------------------------------------------------------------------

#[test]
fn ends_with_path_suffix_requires_boundary() {
    // True when the suffix aligns to a directory separator.
    assert!(ends_with_path_suffix(
        "themes/powerline/powerline.base.sh",
        "powerline/powerline.base.sh"
    ));
    assert!(ends_with_path_suffix(
        "themes/powerline/powerline.base.sh",
        "powerline.base.sh"
    ));
    assert!(ends_with_path_suffix("lib/helpers.sh", "helpers.sh"));
    assert!(ends_with_path_suffix("foo/obar.sh", "obar.sh")); // slash IS the boundary
    // Exact match is fine.
    assert!(ends_with_path_suffix("foo.sh", "foo.sh"));
    // False when the suffix partially overlaps a filename component (no slash boundary).
    // "src/foobar.sh" ends with "bar.sh" in bytes but 'b' before 'bar' isn't a separator.
    assert!(!ends_with_path_suffix("src/foobar.sh", "bar.sh"));
}

// ---------------------------------------------------------------------------
// Shell source resolution — relative path `source ./lib/helpers.sh`
// ---------------------------------------------------------------------------

#[test]
fn shell_source_resolves_relative_path() {
    // helpers.sh defines `run_backup`.
    let helpers = make_sh_file("lib/helpers.sh", vec![make_fn_sym("run_backup")], vec![]);

    // main.sh sources `./lib/helpers.sh` and calls `run_backup`.
    let main_sh = make_sh_file(
        "main.sh",
        vec![make_fn_sym("main")],
        vec![
            make_source_import("./lib/helpers.sh"),
            make_calls_ref("run_backup"),
        ],
    );

    let parsed = vec![helpers, main_sh];

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("lib/helpers.sh".to_string(), "run_backup".to_string()), 10);
    id_map.insert(("main.sh".to_string(), "main".to_string()), 20);

    let index = SymbolIndex::build(&parsed, &id_map);
    let resolver = BashResolver;

    // Build FileContext for main.sh using the resolver.
    let file_ctx = resolver.build_file_context(&parsed[1], None);

    let calls_ref = make_calls_ref("run_backup");
    let source_sym = make_fn_sym("main");
    let ref_ctx = RefContext {
        extracted_ref: &calls_ref,
        source_symbol: &source_sym,
        scope_chain: vec![],
        file_package_id: None,
    };

    let res = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(res.is_some(), "run_backup called in main.sh should resolve via shell source");
    let res = res.unwrap();
    assert_eq!(res.target_symbol_id, 10, "should resolve to helpers.sh:run_backup (id=10)");
    assert_eq!(res.confidence, 0.90);
    assert_eq!(res.strategy, "bash_shell_source");
}

// ---------------------------------------------------------------------------
// Shell source resolution — $VAR-prefixed path
// ---------------------------------------------------------------------------

#[test]
fn shell_source_resolves_variable_prefixed_path() {
    // powerline.base.sh defines `__powerline_prompt_command`.
    let base = make_sh_file(
        "themes/powerline/powerline.base.sh",
        vec![make_fn_sym("__powerline_prompt_command")],
        vec![],
    );

    // powerline.theme.sh sources `$OSH/themes/powerline/powerline.base.sh`.
    let theme = make_sh_file(
        "themes/powerline/powerline.theme.sh",
        vec![make_fn_sym("_omb_theme_PROMPT_COMMAND")],
        vec![
            make_source_import("$OSH/themes/powerline/powerline.base.sh"),
            make_calls_ref("__powerline_prompt_command"),
        ],
    );

    let parsed = vec![base, theme];

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(
        (
            "themes/powerline/powerline.base.sh".to_string(),
            "__powerline_prompt_command".to_string(),
        ),
        42,
    );
    id_map.insert(
        (
            "themes/powerline/powerline.theme.sh".to_string(),
            "_omb_theme_PROMPT_COMMAND".to_string(),
        ),
        43,
    );

    let index = SymbolIndex::build(&parsed, &id_map);
    let resolver = BashResolver;

    let file_ctx = resolver.build_file_context(&parsed[1], None);
    let calls_ref = make_calls_ref("__powerline_prompt_command");
    let source_sym = make_fn_sym("_omb_theme_PROMPT_COMMAND");
    let ref_ctx = RefContext {
        extracted_ref: &calls_ref,
        source_symbol: &source_sym,
        scope_chain: vec![],
        file_package_id: None,
    };

    let res = resolver.resolve(&file_ctx, &ref_ctx, &index);
    assert!(
        res.is_some(),
        "theme.sh should resolve __powerline_prompt_command via $OSH-prefixed source"
    );
    let res = res.unwrap();
    assert_eq!(res.target_symbol_id, 42);
    assert_eq!(res.confidence, 0.90);
}

// ---------------------------------------------------------------------------
// Shell source does NOT match: absolute source path is skipped
// ---------------------------------------------------------------------------

#[test]
fn shell_source_skips_absolute_source_path() {
    // some.sh sources `/etc/profile` — absolute, not in the project.
    let some = make_sh_file(
        "some.sh",
        vec![make_fn_sym("do_thing")],
        vec![make_source_import("/etc/profile"), make_calls_ref("profile_fn")],
    );
    // A file that happens to define `profile_fn` (but not sourced by some.sh).
    let other = make_sh_file("other.sh", vec![make_fn_sym("profile_fn")], vec![]);

    let parsed = vec![some, other];

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    id_map.insert(("some.sh".to_string(), "do_thing".to_string()), 1);
    id_map.insert(("other.sh".to_string(), "profile_fn".to_string()), 2);

    let index = SymbolIndex::build(&parsed, &id_map);
    let resolver = BashResolver;
    let file_ctx = resolver.build_file_context(&parsed[0], None);

    let calls_ref = make_calls_ref("profile_fn");
    let source_sym = make_fn_sym("do_thing");
    let ref_ctx = RefContext {
        extracted_ref: &calls_ref,
        source_symbol: &source_sym,
        scope_chain: vec![],
        file_package_id: None,
    };

    // Shell source step must not fire (absolute path → suffix is "").
    // The ref may still resolve via P4 heuristic (name+kind), which is fine —
    // we only assert that if it resolves, strategy is NOT bash_shell_source.
    let res = resolver.resolve(&file_ctx, &ref_ctx, &index);
    if let Some(r) = res {
        assert_ne!(
            r.strategy, "bash_shell_source",
            "absolute source path must not produce a shell_source resolution"
        );
    }
    // If None, the test passes (no false shell-source match).
}
