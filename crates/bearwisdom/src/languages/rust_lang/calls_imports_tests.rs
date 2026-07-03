// Tests for calls_imports.rs — `use` wildcard module resolution.

use crate::types::EdgeKind;

/// Parse a Rust source snippet and return the `module` field of its first
/// `use ...::*;` wildcard import ref.
fn wildcard_import_module(src: &str) -> Option<String> {
    let result = super::super::extract::extract(src);
    result
        .refs
        .into_iter()
        .find(|r| r.kind == EdgeKind::Imports && r.target_name == "*")
        .and_then(|r| r.module)
}

#[test]
fn super_wildcard_at_crate_root_resolves_to_bare_crate() {
    let src = r#"
pub struct Thing;
pub fn helper() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let _ = Thing;
        helper();
    }
}
"#;
    assert_eq!(wildcard_import_module(src), Some("crate".to_string()));
}

#[test]
fn super_wildcard_inside_nested_module_resolves_to_crate_rooted_submodule() {
    let src = r#"
mod foo {
    pub struct Thing;

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn it_works() {
            let _ = Thing;
        }
    }
}
"#;
    assert_eq!(
        wildcard_import_module(src),
        Some("crate::foo".to_string())
    );
}
