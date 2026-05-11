use super::*;
use crate::types::SymbolKind;

#[test]
fn headings_extracted_like_markdown() {
    let src = "# Top\n\n## Sub\n";
    let r = extract(src, "page.mdx");
    let h: Vec<&str> = r
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Field)
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(h, vec!["Top", "Sub"]);
}

#[test]
fn file_stem_host_symbol_emitted() {
    let src = "plain\n";
    let r = extract(src, "content/post.mdx");
    assert_eq!(r.symbols[0].name, "post");
}

#[test]
fn capitalized_jsx_becomes_calls_ref() {
    let src = "Hello.\n\n<Button variant=\"primary\">Click</Button>\n";
    let r = extract(src, "page.mdx");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert_eq!(calls, vec!["Button"]);
}

#[test]
fn self_closing_jsx_becomes_calls_ref() {
    let src = "<Hero title=\"Hi\" />\n";
    let r = extract(src, "page.mdx");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert_eq!(calls, vec!["Hero"]);
}

#[test]
fn dotted_jsx_becomes_calls_ref() {
    let src = "<Tabs.Root>\n<Tabs.Item />\n</Tabs.Root>\n";
    let r = extract(src, "page.mdx");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    // `</Tabs.Root>` is an end tag and skipped.
    assert!(calls.contains(&"Tabs.Root"));
    assert!(calls.contains(&"Tabs.Item"));
}

#[test]
fn lowercase_html_tag_is_not_a_ref() {
    let src = "A paragraph with <div>inner</div>.\n";
    let r = extract(src, "page.mdx");
    let calls_count = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .count();
    assert_eq!(calls_count, 0);
}

#[test]
fn lowercase_dotted_accepted_motion_style() {
    let src = "<motion.div animate={{ x: 1 }} />\n";
    let r = extract(src, "page.mdx");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert_eq!(calls, vec!["motion.div"]);
}

#[test]
fn fragment_tag_ignored() {
    let src = "<>\n<div />\n</>\n";
    let r = extract(src, "page.mdx");
    let calls_count = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .count();
    assert_eq!(calls_count, 0);
}

#[test]
fn fragment_named_tag_is_not_a_ref() {
    // `<Fragment slot="...">` is an Astro/MDX built-in slot wrapper — never
    // a user-imported component. Verify it produces no Calls ref.
    let src = "<Fragment slot=\"sidebar\">\nsome content\n</Fragment>\n<Card />\n";
    let r = extract(src, "page.mdx");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert_eq!(calls, vec!["Card"], "Fragment must not emit a Calls ref");
}

#[test]
fn jsx_inside_fence_not_extracted() {
    let src = "```tsx\n<Button />\n```\n\n<Outside />\n";
    let r = extract(src, "page.mdx");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    // `<Button />` is inside the tsx fence and must NOT emit a ref
    // from MDX's own scanner — the TS sub-extractor handles it.
    assert_eq!(calls, vec!["Outside"]);
}

#[test]
fn relative_link_still_becomes_imports_ref() {
    let src = "See [details](./info.md).\n";
    let r = extract(src, "page.mdx");
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::Imports && r.target_name == "info")
    );
}

#[test]
fn end_tag_is_not_emitted_as_separate_ref() {
    let src = "<Card>body</Card>\n";
    let r = extract(src, "page.mdx");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert_eq!(calls, vec!["Card"]);
}

#[test]
fn inline_code_generic_types_are_not_jsx() {
    let src = "Handlers implement `ICommand<TResponse>` and `IQuery<TResponse>` — see `ValueTask<T>`.\n\n<RealComponent />";
    let r = extract(src, "page.mdx");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert_eq!(
        calls,
        vec!["RealComponent"],
        "inline-code generic types leaked through"
    );
}

#[test]
fn double_backtick_inline_code_also_skipped() {
    let src = "Use ``<Button>`` to render. And <ActualButton />.\n";
    let r = extract(src, "page.mdx");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert_eq!(calls, vec!["ActualButton"]);
}
