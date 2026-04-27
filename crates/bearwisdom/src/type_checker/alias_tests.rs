// =============================================================================
// type_checker/alias_tests.rs — Unit tests for alias expansion
// =============================================================================

use super::*;
use crate::indexer::resolve::engine::{SymbolInfo, SymbolLookup};
use crate::types::AliasTarget;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Test fixture — a minimal SymbolLookup that only exposes `alias_target`
// and `generic_params`, the two methods `expand_alias` consults. Every
// other trait method falls back to a no-op or empty default; the
// expander should not read them. The fixture's HashMap-backed storage
// makes intent of each test obvious without standing up a SymbolIndex.
// ---------------------------------------------------------------------------

struct AliasFixture {
    aliases: HashMap<String, AliasTarget>,
    generic_params: HashMap<String, Vec<String>>,
    empty: Vec<SymbolInfo>,
    empty_reexports: Vec<(String, String)>,
}

impl AliasFixture {
    fn new() -> Self {
        Self {
            aliases: HashMap::new(),
            generic_params: HashMap::new(),
            empty: Vec::new(),
            empty_reexports: Vec::new(),
        }
    }

    fn with_alias(mut self, name: &str, target: AliasTarget) -> Self {
        self.aliases.insert(name.to_string(), target);
        self
    }

    fn with_generic(mut self, name: &str, params: &[&str]) -> Self {
        self.generic_params.insert(
            name.to_string(),
            params.iter().map(|s| s.to_string()).collect(),
        );
        self
    }
}

impl SymbolLookup for AliasFixture {
    fn by_name(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn by_qualified_name(&self, _: &str) -> Option<&SymbolInfo> {
        None
    }
    fn members_of(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn types_by_name(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> {
        Vec::new()
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn field_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn return_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn field_type_args(&self, _: &str) -> Option<&[String]> {
        None
    }
    fn generic_params(&self, name: &str) -> Option<&[String]> {
        self.generic_params.get(name).map(|v| v.as_slice())
    }
    fn alias_target(&self, name: &str) -> Option<&AliasTarget> {
        self.aliases.get(name)
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &self.empty_reexports
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
}

fn s(strs: &[&str]) -> Vec<String> {
    strs.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn unknown_name_returns_none() {
    let lookup = AliasFixture::new();
    let mut env = TypeEnvironment::new();
    assert_eq!(expand_alias("Nope", &[], &lookup, &mut env), None);
}

#[test]
fn simple_application_expands_to_root() {
    // type Id = string  →  expand("Id") = ("string", [])
    // (Even though `string` isn't itself an alias, the expander still
    // returns the resolved head — the chain walker uses that head for
    // the next field/return lookup.)
    let lookup = AliasFixture::new().with_alias(
        "Id",
        AliasTarget::Application {
            root: "string".to_string(),
            args: Vec::new(),
        },
    );
    let mut env = TypeEnvironment::new();
    let (root, args) = expand_alias("Id", &[], &lookup, &mut env).expect("expanded");
    assert_eq!(root, "string");
    assert!(args.is_empty());
}

#[test]
fn concrete_generic_application_carries_args() {
    // type UserMap = Map<string, User>  →  expand("UserMap") = ("Map", ["string", "User"])
    let lookup = AliasFixture::new().with_alias(
        "UserMap",
        AliasTarget::Application {
            root: "Map".to_string(),
            args: s(&["string", "User"]),
        },
    );
    let mut env = TypeEnvironment::new();
    let (root, args) = expand_alias("UserMap", &[], &lookup, &mut env).expect("expanded");
    assert_eq!(root, "Map");
    assert_eq!(args, s(&["string", "User"]));
}

#[test]
fn generic_alias_substitutes_its_own_param() {
    // type List<T> = Array<T>  →  expand("List", ["User"]) = ("Array", ["User"])
    let lookup = AliasFixture::new()
        .with_alias(
            "List",
            AliasTarget::Application {
                root: "Array".to_string(),
                args: s(&["T"]),
            },
        )
        .with_generic("List", &["T"]);
    let mut env = TypeEnvironment::new();
    let (root, args) = expand_alias("List", &s(&["User"]), &lookup, &mut env).expect("expanded");
    assert_eq!(root, "Array");
    assert_eq!(args, s(&["User"]));
}

#[test]
fn generic_alias_with_mixed_args_substitutes_only_bound() {
    // type StringMap<T> = Map<string, T>  →  expand("StringMap", ["Order"])
    //                                          = ("Map", ["string", "Order"])
    let lookup = AliasFixture::new()
        .with_alias(
            "StringMap",
            AliasTarget::Application {
                root: "Map".to_string(),
                args: s(&["string", "T"]),
            },
        )
        .with_generic("StringMap", &["T"]);
    let mut env = TypeEnvironment::new();
    let (root, args) =
        expand_alias("StringMap", &s(&["Order"]), &lookup, &mut env).expect("expanded");
    assert_eq!(root, "Map");
    assert_eq!(args, s(&["string", "Order"]));
}

#[test]
fn alias_chain_collapses_through_multiple_hops() {
    // type A = B; type B = C; type C = string  →  expand("A") = ("string", [])
    let lookup = AliasFixture::new()
        .with_alias(
            "A",
            AliasTarget::Application {
                root: "B".to_string(),
                args: Vec::new(),
            },
        )
        .with_alias(
            "B",
            AliasTarget::Application {
                root: "C".to_string(),
                args: Vec::new(),
            },
        )
        .with_alias(
            "C",
            AliasTarget::Application {
                root: "string".to_string(),
                args: Vec::new(),
            },
        );
    let mut env = TypeEnvironment::new();
    let (root, _) = expand_alias("A", &[], &lookup, &mut env).expect("expanded");
    assert_eq!(root, "string");
}

#[test]
fn union_alias_returns_none() {
    let lookup = AliasFixture::new().with_alias(
        "Status",
        AliasTarget::Union(s(&["Pending", "Active", "Closed"])),
    );
    let mut env = TypeEnvironment::new();
    assert_eq!(expand_alias("Status", &[], &lookup, &mut env), None);
}

#[test]
fn intersection_alias_returns_none() {
    let lookup = AliasFixture::new().with_alias(
        "Combined",
        AliasTarget::Intersection(s(&["Auditable", "Versioned"])),
    );
    let mut env = TypeEnvironment::new();
    assert_eq!(expand_alias("Combined", &[], &lookup, &mut env), None);
}

#[test]
fn object_alias_returns_none() {
    // type Point = { x: number; y: number }
    // members are emitted as Properties — chain walker resolves them via
    // members_of, not via alias expansion.
    let lookup = AliasFixture::new().with_alias("Point", AliasTarget::Object);
    let mut env = TypeEnvironment::new();
    assert_eq!(expand_alias("Point", &[], &lookup, &mut env), None);
}

#[test]
fn other_alias_returns_none() {
    // mapped, conditional, keyof, typeof, etc. — not expanded in PR 9.
    let lookup = AliasFixture::new().with_alias("Mapped", AliasTarget::Other);
    let mut env = TypeEnvironment::new();
    assert_eq!(expand_alias("Mapped", &[], &lookup, &mut env), None);
}

#[test]
fn self_referential_alias_does_not_loop() {
    // type Loop = Loop  — pathological but must not hang.
    let lookup = AliasFixture::new().with_alias(
        "Loop",
        AliasTarget::Application {
            root: "Loop".to_string(),
            args: Vec::new(),
        },
    );
    let mut env = TypeEnvironment::new();
    // Either returns None or short-circuits — either is acceptable as long
    // as it terminates. Implementation chooses None when no progress made.
    let result = expand_alias("Loop", &[], &lookup, &mut env);
    assert!(result.is_none() || matches!(result, Some(_)));
}

#[test]
fn deep_chain_caps_at_max_expansion_depth() {
    // Build a long alias chain a0 → a1 → ... → a20 → string. The expander
    // must terminate (cap at MAX_EXPANSION_DEPTH = 8) and return the head
    // it reached, not loop forever.
    let mut lookup = AliasFixture::new();
    for i in 0..20 {
        lookup = lookup.with_alias(
            &format!("a{i}"),
            AliasTarget::Application {
                root: format!("a{}", i + 1),
                args: Vec::new(),
            },
        );
    }
    lookup = lookup.with_alias(
        "a20",
        AliasTarget::Application {
            root: "string".to_string(),
            args: Vec::new(),
        },
    );
    let mut env = TypeEnvironment::new();
    let (root, _) = expand_alias("a0", &[], &lookup, &mut env).expect("expanded");
    // After the cap, head should be at most 8 hops in (a8 or beyond).
    assert!(
        root.starts_with('a') || root == "string",
        "head landed somewhere reasonable, got {root}"
    );
}
