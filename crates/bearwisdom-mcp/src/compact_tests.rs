use super::*;
use bearwisdom::search::grep::GrepMatch;
use bearwisdom::SearchResult;

fn make_search_result(file: &str, line: u32) -> SearchResult {
    SearchResult {
        name: "DoThing".to_string(),
        qualified_name: "App.DoThing".to_string(),
        kind: "function".to_string(),
        file_path: file.to_string(),
        start_line: line,
        signature: None,
        score: 1.0,
    }
}

fn make_grep_match(file: &str, line_number: u32, content: &str) -> GrepMatch {
    GrepMatch {
        file_path: file.to_string(),
        line_number,
        column: 0,
        line_content: content.to_string(),
        match_start: 0,
        match_end: 0,
    }
}

#[test]
fn search_single_result_inlines_path_and_omits_files_section() {
    let results = vec![make_search_result("src/foo.rs", 42)];
    let out = search(&results);

    assert!(out.contains("#format:compact-v1"));
    assert!(!out.contains("#files"), "single-result must not emit a #files registry, got:\n{out}");
    assert!(!out.contains("F1:"), "single-result must not produce F1 references, got:\n{out}");
    assert!(out.contains("src/foo.rs:42"), "single-result must inline the path, got:\n{out}");
}

#[test]
fn search_multi_result_uses_files_registry() {
    let results = vec![
        make_search_result("src/a.rs", 1),
        make_search_result("src/b.rs", 2),
    ];
    let out = search(&results);

    assert!(out.contains("#files"), "multi-result must keep the file registry, got:\n{out}");
    assert!(out.contains("F1:src/a.rs"));
    assert!(out.contains("F2:src/b.rs"));
}

#[test]
fn with_freshness_header_injects_index_block_into_compact_response() {
    let response = format!("#format:compact-v1\n#meta\ncount:0\n");
    let out = crate::server::BearWisdomServer::with_freshness_header(
        response,
        Some(1_700_000_000_000),
    );
    assert!(out.starts_with("#format:compact-v1\n"));
    assert!(out.contains("#index"));
    assert!(out.contains("last_indexed_at_ms:1700000000000"));
    assert!(out.contains("age_ms:"));
    assert!(out.contains("#meta"));
}

#[test]
fn with_freshness_header_skips_non_compact_responses() {
    let response = r#"{"ok":true,"data":[]}"#.to_string();
    let out = crate::server::BearWisdomServer::with_freshness_header(response, Some(123));
    assert_eq!(out, r#"{"ok":true,"data":[]}"#);
}

#[test]
fn with_freshness_header_handles_unknown_index_time() {
    let response = format!("#format:compact-v1\n#meta\ncount:0\n");
    let out = crate::server::BearWisdomServer::with_freshness_header(response, None);
    assert!(out.contains("last_indexed_at_ms:unknown"));
}

#[test]
fn grep_single_result_inlines_path() {
    let results = vec![make_grep_match("crates/foo/src/lib.rs", 10, "fn hello() {}")];
    let out = grep(&results);

    assert!(!out.contains("#files"), "single-result grep must skip #files, got:\n{out}");
    assert!(out.contains("crates/foo/src/lib.rs:10"));
}
