use super::*;

#[test]
fn server_for_known_languages() {
    let reg = ServerRegistry::new();

    let ts = reg.server_for(&Language::TypeScript).unwrap();
    assert_eq!(ts.command, "typescript-language-server");
    assert!(ts.args.contains(&"--stdio".to_string()));

    let cs = reg.server_for(&Language::CSharp).unwrap();
    assert_eq!(cs.command, "csharp-ls");

    let rust = reg.server_for(&Language::Rust).unwrap();
    assert_eq!(rust.command, "rust-analyzer");

    let py = reg.server_for(&Language::Python).unwrap();
    assert_eq!(py.command, "pyright-langserver");

    let go = reg.server_for(&Language::Go).unwrap();
    assert_eq!(go.command, "gopls");

    let java = reg.server_for(&Language::Java).unwrap();
    assert_eq!(java.command, "jdtls");

    let cpp = reg.server_for(&Language::Cpp).unwrap();
    assert_eq!(cpp.command, "clangd");
}

#[test]
fn all_entries_have_display_name() {
    let reg = ServerRegistry::new();
    for entry in &reg.entries {
        assert!(
            !entry.display_name.is_empty(),
            "entry for {:?} missing display_name",
            entry.language
        );
    }
}

#[test]
fn javascript_shares_binary_with_typescript() {
    let reg = ServerRegistry::new();
    let js = reg.server_for(&Language::JavaScript).unwrap();
    let ts = reg.server_for(&Language::TypeScript).unwrap();
    assert_eq!(js.command, ts.command);
}
