use super::*;

#[test]
fn language_for_file_known_extensions() {
    assert_eq!(
        LspManager::language_for_file("src/main.rs"),
        Some(Language::Rust)
    );
    assert_eq!(
        LspManager::language_for_file("index.ts"),
        Some(Language::TypeScript)
    );
    assert_eq!(
        LspManager::language_for_file("app.tsx"),
        Some(Language::TypeScript)
    );
    assert_eq!(
        LspManager::language_for_file("Program.cs"),
        Some(Language::CSharp)
    );
    assert_eq!(
        LspManager::language_for_file("main.py"),
        Some(Language::Python)
    );
    assert_eq!(
        LspManager::language_for_file("main.go"),
        Some(Language::Go)
    );
}

#[test]
fn language_for_file_unknown_returns_none() {
    assert_eq!(LspManager::language_for_file("Cargo.toml"), None);
    assert_eq!(LspManager::language_for_file("README.md"), None);
    assert_eq!(LspManager::language_for_file("noextension"), None);
}

#[test]
fn file_uri_unix_style() {
    // Test the URI helper with a synthetic path rather than the cfg-gated
    // platform implementation — just verify the prefix.
    let root = PathBuf::from("/home/user/project");
    let uri = LspManager::file_uri(&root, "src/main.rs");
    assert!(
        uri.starts_with("file://"),
        "URI must start with file://: {uri}"
    );
    assert!(uri.contains("main.rs"), "URI must contain filename: {uri}");
}

#[test]
fn status_returns_none_for_unstarted_server() {
    let mgr = LspManager::new("/tmp/fake");
    assert!(mgr.status(&Language::Rust).is_none());
    assert_eq!(mgr.state(&Language::TypeScript), ServerState::Stopped);
}

#[test]
fn extract_hover_text_string() {
    let text = extract_hover_text(HoverContents::String("hello".to_string()));
    assert_eq!(text, "hello");
}

#[test]
fn extract_hover_text_markup() {
    let text = extract_hover_text(HoverContents::MarkupContent {
        kind: "markdown".to_string(),
        value: "**bold**".to_string(),
    });
    assert_eq!(text, "**bold**");
}

#[test]
fn extract_hover_text_array_mixed() {
    let arr = vec![
        serde_json::json!("plain string"),
        serde_json::json!({ "language": "rust", "value": "fn foo()" }),
    ];
    let text = extract_hover_text(HoverContents::Array(arr));
    assert!(text.contains("plain string"));
    assert!(text.contains("fn foo()"));
}

#[test]
fn parse_locations_null_returns_empty() {
    let result = parse_locations(serde_json::Value::Null).unwrap();
    assert!(result.is_empty());
}

#[test]
fn parse_locations_array() {
    let json = serde_json::json!([
        {
            "uri": "file:///src/main.rs",
            "range": {
                "start": { "line": 1, "character": 4 },
                "end":   { "line": 1, "character": 7 }
            }
        }
    ]);
    let locs = parse_locations(json).unwrap();
    assert_eq!(locs.len(), 1);
    assert_eq!(locs[0].uri, "file:///src/main.rs");
    assert_eq!(locs[0].range.start.line, 1);
}

#[test]
fn parse_locations_single_object() {
    let json = serde_json::json!({
        "uri": "file:///lib.rs",
        "range": {
            "start": { "line": 0, "character": 0 },
            "end":   { "line": 0, "character": 3 }
        }
    });
    let locs = parse_locations(json).unwrap();
    assert_eq!(locs.len(), 1);
    assert_eq!(locs[0].uri, "file:///lib.rs");
}

// -----------------------------------------------------------------------
// Integration test — requires typescript-language-server installed.
// Run manually: cargo test -- --ignored lsp_goto_definition_typescript
// -----------------------------------------------------------------------

/// Verify that LspManager can start typescript-language-server, open a
/// TypeScript file, and perform a goto_definition that resolves an import.
///
/// Skip automatically when `typescript-language-server` is not on PATH.
#[tokio::test]
#[ignore = "requires typescript-language-server on PATH; run with --ignored"]
async fn lsp_goto_definition_typescript() {
    use std::fs;

    // Check whether the server binary is available before doing anything.
    let server_available = std::process::Command::new("typescript-language-server")
        .arg("--version")
        .output()
        .is_ok();

    if !server_available {
        eprintln!("SKIP: typescript-language-server not found on PATH");
        return;
    }

    // Create a temp workspace with two TypeScript files:
    //   lib.ts  — exports a function `greet`
    //   main.ts — imports and calls `greet` from `./lib`
    let dir = tempfile::TempDir::new().expect("tempdir");
    let lib_path = dir.path().join("lib.ts");
    let main_path = dir.path().join("main.ts");

    fs::write(
        &lib_path,
        "export function greet(name: string): string {\n    return `Hello, ${name}`;\n}\n",
    ).expect("write lib.ts");

    fs::write(
        &main_path,
        "import { greet } from './lib';\nconst msg = greet('world');\nconsole.log(msg);\n",
    ).expect("write main.ts");

    // Build URIs for the LSP server (file:///... scheme).
    let main_uri = path_to_uri(&main_path);
    let _lib_uri = path_to_uri(&lib_path);

    let mgr = LspManager::new(dir.path());

    // Open main.ts so the server has its content.
    let main_content = fs::read_to_string(&main_path).expect("read main.ts");
    mgr.did_open(&main_uri, &main_content)
        .await
        .expect("did_open main.ts");

    // Give the server a moment to process the file before querying.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // goto_definition at line 0, col 9 — the `greet` identifier in the import.
    // Expected: jumps to lib.ts where `greet` is defined.
    let locations = mgr
        .goto_definition(&main_uri, 0, 9)
        .await
        .expect("goto_definition");

    // Clean up.
    mgr.shutdown_all().await.expect("shutdown");

    assert!(
        !locations.is_empty(),
        "Expected at least one definition location, got none"
    );

    let def = &locations[0];
    assert!(
        def.uri.contains("lib"),
        "Definition should be in lib.ts, got: {}",
        def.uri
    );
    assert_eq!(
        def.range.start.line, 0,
        "greet is defined on line 0 of lib.ts"
    );

    eprintln!("goto_definition resolved to: {} line {}", def.uri, def.range.start.line);
}
