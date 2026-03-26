use super::*;
use std::io::Write;
use tempfile::TempDir;

fn write_file(dir: &TempDir, name: &str, content: &str) {
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
}

fn not_cancelled() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

#[test]
fn literal_search_finds_exact_match() {
    let dir = TempDir::new().unwrap();
    write_file(&dir, "foo.rs", "fn hello() {}\nfn world() {}\n");

    let opts = GrepOptions::default();
    let results = grep_search(dir.path(), "hello", &opts, &not_cancelled()).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].line_number, 1);
    assert_eq!(results[0].line_content, "fn hello() {}");
    assert!(results[0].line_content.contains("hello"));
    // match_start should point at 'h' in hello
    let start = results[0].match_start as usize;
    let end = results[0].match_end as usize;
    assert_eq!(&results[0].line_content[start..end], "hello");
}

#[test]
fn regex_search_uses_pattern_as_regex() {
    let dir = TempDir::new().unwrap();
    write_file(&dir, "app.ts", "const x = 42;\nconst y = 99;\nlet z = 0;\n");

    let opts = GrepOptions {
        regex: true,
        ..Default::default()
    };
    let results = grep_search(dir.path(), r"const \w+ = \d+", &opts, &not_cancelled()).unwrap();

    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|m| m.line_content.starts_with("const")));
}

#[test]
fn case_insensitive_search() {
    let dir = TempDir::new().unwrap();
    write_file(&dir, "readme.md", "Hello World\nhello world\nHELLO WORLD\n");

    let opts = GrepOptions {
        case_sensitive: false,
        ..Default::default()
    };
    let results = grep_search(dir.path(), "hello", &opts, &not_cancelled()).unwrap();

    assert_eq!(results.len(), 3, "Should match all three case variants");
}

#[test]
fn whole_word_excludes_partial_matches() {
    let dir = TempDir::new().unwrap();
    write_file(&dir, "code.rs", "fn foo() {}\nfn foobar() {}\nlet foo_x = 1;\n");

    let opts = GrepOptions {
        whole_word: true,
        ..Default::default()
    };
    let results = grep_search(dir.path(), "foo", &opts, &not_cancelled()).unwrap();

    // "fn foo()" matches; "foobar" and "foo_x" do not (word boundary)
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].line_content.trim(), "fn foo() {}");
}

#[test]
fn max_results_caps_output() {
    let dir = TempDir::new().unwrap();
    // 20 matching lines
    let content = (0..20).map(|i| format!("needle {i}\n")).collect::<String>();
    write_file(&dir, "big.txt", &content);

    let opts = GrepOptions {
        max_results: 5,
        ..Default::default()
    };
    let results = grep_search(dir.path(), "needle", &opts, &not_cancelled()).unwrap();

    assert_eq!(results.len(), 5);
}

#[test]
fn cancellation_stops_search_early() {
    let dir = TempDir::new().unwrap();
    // Write many files so the walk has work to do
    for i in 0..20 {
        write_file(&dir, &format!("file{i}.txt"), "match me\n");
    }

    let cancelled = Arc::new(AtomicBool::new(true)); // already cancelled
    let opts = GrepOptions::default();
    let results = grep_search(dir.path(), "match me", &opts, &cancelled).unwrap();

    // With cancellation set from the start, zero files should be searched.
    assert_eq!(results.len(), 0);
}

#[test]
fn scope_language_filter_applied() {
    let dir = TempDir::new().unwrap();
    write_file(&dir, "lib.rs", "fn target() {}\n");
    write_file(&dir, "app.ts", "function target() {}\n");

    let opts = GrepOptions {
        scope: SearchScope::default().with_language("rust"),
        ..Default::default()
    };
    let results = grep_search(dir.path(), "target", &opts, &not_cancelled()).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].file_path, "lib.rs");
}

#[test]
fn literal_metacharacters_escaped_correctly() {
    let dir = TempDir::new().unwrap();
    write_file(&dir, "query.txt", "price > (100 + 50)\nprice > 200\n");

    // Pattern contains regex metacharacters; should be treated literally.
    let opts = GrepOptions::default(); // regex: false
    let results = grep_search(dir.path(), "(100 + 50)", &opts, &not_cancelled()).unwrap();

    assert_eq!(results.len(), 1);
    assert!(results[0].line_content.contains("(100 + 50)"));
}

#[test]
fn match_offsets_are_correct() {
    let dir = TempDir::new().unwrap();
    write_file(&dir, "offsets.txt", "prefix NEEDLE suffix\n");

    let opts = GrepOptions::default();
    let results = grep_search(dir.path(), "NEEDLE", &opts, &not_cancelled()).unwrap();

    assert_eq!(results.len(), 1);
    let m = &results[0];
    let extracted = &m.line_content[m.match_start as usize..m.match_end as usize];
    assert_eq!(extracted, "NEEDLE");
}
