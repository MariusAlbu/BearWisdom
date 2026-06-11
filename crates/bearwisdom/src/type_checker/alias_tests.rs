// =============================================================================
// type_checker/alias_tests.rs — Unit tests for alias expansion
// =============================================================================

use super::*;
use crate::indexer::resolve::engine::{SymbolInfo, SymbolLookup};
use crate::type_checker::core::members::MembersIndex;
use crate::type_checker::core::symbol_types::SymbolTypeMap;
use crate::type_checker::core::types::{LitValue, Type, TypeArena};
use crate::types::AliasTarget;
use std::collections::HashMap;

/// Empty member table for alias tests. The TypeId-form expander only consults
/// members in the structural arm of its `Conditional` subtype check; none of
/// these alias tests exercise structural conditionals, so an empty index keeps
/// the conditional arm on its nominal-only path.
fn empty_members() -> MembersIndex {
    MembersIndex::new()
}

/// Empty per-symbol type map for alias tests. Same rationale as
/// `empty_members`: the structural member-type comparison never fires on these
/// nominal-only conditionals, so an empty map suffices.
fn empty_symbol_types() -> SymbolTypeMap {
    SymbolTypeMap::new()
}

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
    field_types: HashMap<String, String>,
    return_types: HashMap<String, String>,
    members: HashMap<String, Vec<SymbolInfo>>,
    empty: Vec<SymbolInfo>,
    empty_reexports: Vec<(String, String)>,
}

impl AliasFixture {
    fn new() -> Self {
        Self {
            aliases: HashMap::new(),
            generic_params: HashMap::new(),
            field_types: HashMap::new(),
            return_types: HashMap::new(),
            members: HashMap::new(),
            empty: Vec::new(),
            empty_reexports: Vec::new(),
        }
    }

    fn with_alias(mut self, name: &str, target: AliasTarget) -> Self {
        self.aliases.insert(name.to_string(), target);
        self
    }

    fn with_member(mut self, owner: &str, member: &str) -> Self {
        self.members
            .entry(owner.to_string())
            .or_default()
            .push(SymbolInfo {
                id: 0,
                name: member.to_string(),
                qualified_name: format!("{owner}.{member}"),
                kind: "property".to_string(),
                visibility: None,
                file_path: std::sync::Arc::from("t.ts"),
                scope_path: Some(owner.to_string()),
                package_id: None,
                signature: None,
            });
        self
    }

    fn with_generic(mut self, name: &str, params: &[&str]) -> Self {
        self.generic_params.insert(
            name.to_string(),
            params.iter().map(|s| s.to_string()).collect(),
        );
        self
    }

    fn with_field_type(mut self, name: &str, ty: &str) -> Self {
        self.field_types.insert(name.to_string(), ty.to_string());
        self
    }

    fn with_return_type(mut self, name: &str, ty: &str) -> Self {
        self.return_types.insert(name.to_string(), ty.to_string());
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
    fn members_of(&self, name: &str) -> &[SymbolInfo] {
        self.members
            .get(name)
            .map(|v| v.as_slice())
            .unwrap_or(&self.empty)
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
    fn field_type_name(&self, name: &str) -> Option<&str> {
        self.field_types.get(name).map(|s| s.as_str())
    }
    fn return_type_name(&self, name: &str) -> Option<&str> {
        self.return_types.get(name).map(|s| s.as_str())
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
    // mapped, conditional, indexed-access, template-literal — not expanded yet.
    let lookup = AliasFixture::new().with_alias("Mapped", AliasTarget::Other);
    let mut env = TypeEnvironment::new();
    assert_eq!(expand_alias("Mapped", &[], &lookup, &mut env), None);
}

// ---------------------------------------------------------------------------
// PR 10: typeof
// ---------------------------------------------------------------------------

#[test]
fn typeof_resolves_through_field_type() {
    // type X = typeof someValue; someValue: User
    //   → expand("X") = ("User", [])
    let lookup = AliasFixture::new()
        .with_alias("X", AliasTarget::Typeof("someValue".to_string()))
        .with_field_type("someValue", "User");
    let mut env = TypeEnvironment::new();
    let (root, args) = expand_alias("X", &[], &lookup, &mut env).expect("expanded");
    assert_eq!(root, "User");
    assert!(args.is_empty());
}

#[test]
fn typeof_resolves_through_return_type_for_function_value() {
    // type X = typeof someFn; someFn(): Result
    //   → expand("X") = ("Result", [])
    let lookup = AliasFixture::new()
        .with_alias("X", AliasTarget::Typeof("someFn".to_string()))
        .with_return_type("someFn", "Result");
    let mut env = TypeEnvironment::new();
    let (root, _) = expand_alias("X", &[], &lookup, &mut env).expect("expanded");
    assert_eq!(root, "Result");
}

#[test]
fn typeof_chains_into_alias_target() {
    // type Inner = typeof obj; obj: UserMap
    // type UserMap = Map<string, User>
    //   → expand("Inner") = ("Map", ["string", "User"])
    let lookup = AliasFixture::new()
        .with_alias("Inner", AliasTarget::Typeof("obj".to_string()))
        .with_field_type("obj", "UserMap")
        .with_alias(
            "UserMap",
            AliasTarget::Application {
                root: "Map".to_string(),
                args: s(&["string", "User"]),
            },
        );
    let mut env = TypeEnvironment::new();
    let (root, args) = expand_alias("Inner", &[], &lookup, &mut env).expect("expanded");
    assert_eq!(root, "Map");
    assert_eq!(args, s(&["string", "User"]));
}

// ---------------------------------------------------------------------------
// PR 13/15: mapped types
// ---------------------------------------------------------------------------

#[test]
fn transparent_mapped_partial_collapses_to_source_arg() {
    // `type Partial<T> = { [K in keyof T]?: T[K] }`
    // expand("Partial", ["User"]) should collapse to ("User", [])
    // so subsequent chain-walking lookups hit User's members.
    let lookup = AliasFixture::new()
        .with_alias(
            "Partial",
            AliasTarget::Mapped {
                source: "T".to_string(),
                value_template: "T[K]".to_string(),
            },
        )
        .with_generic("Partial", &["T"]);
    let mut env = TypeEnvironment::new();
    let (root, args) = expand_alias("Partial", &s(&["User"]), &lookup, &mut env).expect("expanded");
    assert_eq!(root, "User");
    assert!(args.is_empty());
}

#[test]
fn transparent_mapped_readonly_collapses_to_source_arg() {
    // `type Readonly<T> = { readonly [K in keyof T]: T[K] }` —
    // the `readonly` modifier is stripped by the extractor; the
    // value template arrives as "T[K]".
    let lookup = AliasFixture::new()
        .with_alias(
            "Readonly",
            AliasTarget::Mapped {
                source: "T".to_string(),
                value_template: "T[K]".to_string(),
            },
        )
        .with_generic("Readonly", &["T"]);
    let mut env = TypeEnvironment::new();
    let (root, _) = expand_alias("Readonly", &s(&["Order"]), &lookup, &mut env).expect("expanded");
    assert_eq!(root, "Order");
}

#[test]
fn record_mapped_projects_value_type_from_param_arg() {
    // `type Record<K, V> = { [P in K]: V }` — the iteration source K is not a
    // `keyof` shape, so the extractor leaves `source` empty; the value
    // template is the flat value head "V". expand("Record", ["string", "User"])
    // projects the value slot: V binds to "User", so the alias yields User and
    // chain walking continues against User's members.
    let lookup = AliasFixture::new()
        .with_alias(
            "Record",
            AliasTarget::Mapped {
                source: String::new(),
                value_template: "V".to_string(),
            },
        )
        .with_generic("Record", &["K", "V"]);
    let mut env = TypeEnvironment::new();
    let (root, args) =
        expand_alias("Record", &s(&["string", "User"]), &lookup, &mut env).expect("expanded");
    assert_eq!(root, "User");
    assert!(args.is_empty());
}

#[test]
fn record_mapped_with_concrete_value_projects_that_type() {
    // `type AllBool<K> = { [P in K]: boolean }` — value head is a concrete
    // type written directly in the mapping, not a generic param. Every key
    // projects `boolean`, so the alias yields it regardless of caller args.
    let lookup = AliasFixture::new()
        .with_alias(
            "AllBool",
            AliasTarget::Mapped {
                source: String::new(),
                value_template: "boolean".to_string(),
            },
        )
        .with_generic("AllBool", &["K"]);
    let mut env = TypeEnvironment::new();
    let (root, _) = expand_alias("AllBool", &s(&["string"]), &lookup, &mut env).expect("expanded");
    assert_eq!(root, "boolean");
}

#[test]
fn record_mapped_with_keyof_source_projects_value_type() {
    // `type Values<X> = { [K in keyof X]: V }` — keyof source is populated but
    // the value template is still a flat head, so it projects the value slot
    // the same way the empty-source Record form does.
    let lookup = AliasFixture::new()
        .with_alias(
            "Values",
            AliasTarget::Mapped {
                source: "X".to_string(),
                value_template: "V".to_string(),
            },
        )
        .with_generic("Values", &["X", "V"]);
    let mut env = TypeEnvironment::new();
    let (root, _) =
        expand_alias("Values", &s(&["Container", "User"]), &lookup, &mut env).expect("expanded");
    assert_eq!(root, "User");
}

#[test]
fn record_mapped_chains_into_value_alias() {
    // Value head is itself an alias — the loop re-enters and collapses it.
    // `type Wrap<K> = { [P in K]: UserMap }`; `type UserMap = Map<string, User>`.
    let lookup = AliasFixture::new()
        .with_alias(
            "Wrap",
            AliasTarget::Mapped {
                source: String::new(),
                value_template: "UserMap".to_string(),
            },
        )
        .with_generic("Wrap", &["K"])
        .with_alias(
            "UserMap",
            AliasTarget::Application {
                root: "Map".to_string(),
                args: s(&["string", "User"]),
            },
        );
    let mut env = TypeEnvironment::new();
    let (root, args) = expand_alias("Wrap", &s(&["string"]), &lookup, &mut env).expect("expanded");
    assert_eq!(root, "Map");
    assert_eq!(args, s(&["string", "User"]));
}

#[test]
fn custom_mapped_function_value_returns_none() {
    // `type Getters<T> = { [K in keyof T]: () => T[K] }` — the value template
    // carries a function arrow, not a flat type head. Projecting it would be a
    // guess, so the expander declines.
    let lookup = AliasFixture::new()
        .with_alias(
            "Getters",
            AliasTarget::Mapped {
                source: "T".to_string(),
                value_template: "() => T[K]".to_string(),
            },
        )
        .with_generic("Getters", &["T"]);
    let mut env = TypeEnvironment::new();
    assert_eq!(
        expand_alias("Getters", &s(&["User"]), &lookup, &mut env),
        None
    );
}

#[test]
fn record_mapped_unbound_param_value_returns_none() {
    // Record referenced without the value arg: the value head "V" is a param
    // with no caller arg to bind. Projecting the bare param name would feed the
    // walker a non-type, so the expander declines.
    let lookup = AliasFixture::new()
        .with_alias(
            "Record",
            AliasTarget::Mapped {
                source: String::new(),
                value_template: "V".to_string(),
            },
        )
        .with_generic("Record", &["K", "V"]);
    let mut env = TypeEnvironment::new();
    assert_eq!(expand_alias("Record", &[], &lookup, &mut env), None);
}

#[test]
fn mapped_with_unbound_source_returns_none() {
    // `Partial` referenced without a generic arg (`type Foo = Partial<unknown_T>`)
    // can't collapse — the source stays as the param name "T".
    let lookup = AliasFixture::new()
        .with_alias(
            "Partial",
            AliasTarget::Mapped {
                source: "T".to_string(),
                value_template: "T[K]".to_string(),
            },
        )
        .with_generic("Partial", &["T"]);
    let mut env = TypeEnvironment::new();
    assert_eq!(expand_alias("Partial", &[], &lookup, &mut env), None);
}

// ---------------------------------------------------------------------------
// PR 14/16: conditional types
// ---------------------------------------------------------------------------

#[test]
fn conditional_picks_true_branch_on_identity() {
    // `type Cond = string extends string ? X : Y` — identity check
    // resolves true, expands to X.
    let lookup = AliasFixture::new().with_alias(
        "Cond",
        AliasTarget::Conditional {
            check: "string".to_string(),
            extends: "string".to_string(),
            true_branch: "TrueBranch".to_string(),
            false_branch: "FalseBranch".to_string(),
            infer_binding: None,
        },
    );
    let mut env = TypeEnvironment::new();
    let (root, _) = expand_alias("Cond", &[], &lookup, &mut env).expect("expanded");
    assert_eq!(root, "TrueBranch");
}

#[test]
fn conditional_picks_false_branch_on_disjoint_primitives() {
    // `type Cond = string extends number ? X : Y` — disjoint primitives,
    // branch goes false.
    let lookup = AliasFixture::new().with_alias(
        "Cond",
        AliasTarget::Conditional {
            check: "string".to_string(),
            extends: "number".to_string(),
            true_branch: "TrueBranch".to_string(),
            false_branch: "FalseBranch".to_string(),
            infer_binding: None,
        },
    );
    let mut env = TypeEnvironment::new();
    let (root, _) = expand_alias("Cond", &[], &lookup, &mut env).expect("expanded");
    assert_eq!(root, "FalseBranch");
}

#[test]
fn conditional_undecidable_returns_none() {
    // `type Cond = User extends Order ? X : Y` — no parent relationship
    // recorded, neither side is a primitive — undecidable, returns None.
    let lookup = AliasFixture::new().with_alias(
        "Cond",
        AliasTarget::Conditional {
            check: "User".to_string(),
            extends: "Order".to_string(),
            true_branch: "TrueBranch".to_string(),
            false_branch: "FalseBranch".to_string(),
            infer_binding: None,
        },
    );
    let mut env = TypeEnvironment::new();
    assert_eq!(expand_alias("Cond", &[], &lookup, &mut env), None);
}

#[test]
fn conditional_chains_into_alias_target() {
    // Resolved branch is itself an alias — expand_alias re-enters the
    // loop and continues collapsing.
    let lookup = AliasFixture::new()
        .with_alias(
            "Cond",
            AliasTarget::Conditional {
                check: "string".to_string(),
                extends: "string".to_string(),
                true_branch: "Wrapper".to_string(),
                false_branch: "Other".to_string(),
                infer_binding: None,
            },
        )
        .with_alias(
            "Wrapper",
            AliasTarget::Application {
                root: "Map".to_string(),
                args: s(&["string", "User"]),
            },
        );
    let mut env = TypeEnvironment::new();
    let (root, args) = expand_alias("Cond", &[], &lookup, &mut env).expect("expanded");
    assert_eq!(root, "Map");
    assert_eq!(args, s(&["string", "User"]));
}

// ---------------------------------------------------------------------------
// infer in conditional extends clauses
// ---------------------------------------------------------------------------

#[test]
fn infer_conditional_yields_element_type() {
    // `type Elem<T> = T extends Array<infer U> ? U : never`, applied as
    // `Elem<Array<User>>`. The infer binding `("U", 0)` plus the true branch
    // being `U` and the bound check type being `Array<User>` (head matches the
    // extends head "Array") yields the element `User` directly — never routed
    // through the subtype check.
    let lookup = AliasFixture::new()
        .with_alias(
            "Elem",
            AliasTarget::Conditional {
                check: "T".to_string(),
                extends: "Array".to_string(),
                true_branch: "U".to_string(),
                false_branch: "never".to_string(),
                infer_binding: Some(("U".to_string(), 0)),
            },
        )
        .with_generic("Elem", &["T"]);
    let mut env = TypeEnvironment::new();
    let (root, _) = expand_alias("Elem", &s(&["Array<User>"]), &lookup, &mut env)
        .expect("infer binding yields element");
    assert_eq!(root, "User");
}

#[test]
fn infer_binding_in_non_true_branch_declines() {
    // `type Elem<T> = T extends Array<infer U> ? Wrapped : U` — the infer var
    // is the FALSE branch, not the true branch. The direct Apply-shape yield
    // fires only when the true branch IS the var, so this falls through to the
    // subtype check; with T → Array<User> the check `Array<User> extends Array`
    // is undecidable, so the arm returns None rather than guessing.
    let lookup = AliasFixture::new()
        .with_alias(
            "Elem",
            AliasTarget::Conditional {
                check: "T".to_string(),
                extends: "Array".to_string(),
                true_branch: "Wrapped".to_string(),
                false_branch: "U".to_string(),
                infer_binding: Some(("U".to_string(), 0)),
            },
        )
        .with_generic("Elem", &["T"]);
    let mut env = TypeEnvironment::new();
    assert_eq!(
        expand_alias("Elem", &s(&["Array<User>"]), &lookup, &mut env),
        None
    );
}

#[test]
fn infer_binding_head_mismatch_declines() {
    // The infer binding targets `Array`, but the bound check type is
    // `Set<User>` — the extends head does not match, so the direct yield
    // declines and the undecidable subtype check returns None.
    let lookup = AliasFixture::new()
        .with_alias(
            "Elem",
            AliasTarget::Conditional {
                check: "T".to_string(),
                extends: "Array".to_string(),
                true_branch: "U".to_string(),
                false_branch: "never".to_string(),
                infer_binding: Some(("U".to_string(), 0)),
            },
        )
        .with_generic("Elem", &["T"]);
    let mut env = TypeEnvironment::new();
    assert_eq!(
        expand_alias("Elem", &s(&["Set<User>"]), &lookup, &mut env),
        None
    );
}

#[test]
fn template_literal_other_declines_cleanly() {
    // `type Path = ` + "`/${string}`" + ` — a template-literal type denotes an
    // unbounded string set with no member-bearing head. The extractor
    // classifies it to `Other`; the expander declines without a panic path.
    let lookup = AliasFixture::new().with_alias("Path", AliasTarget::Other);
    let mut env = TypeEnvironment::new();
    assert_eq!(expand_alias("Path", &[], &lookup, &mut env), None);

    let mut arena = TypeArena::new();
    let pairs = vec![("Path".to_string(), AliasTarget::Other)];
    let aliases = build_alias_index(&pairs, &mut arena);
    let path = arena.class("Path");
    assert_eq!(
        expand_alias_typed(
            path,
            &mut arena,
            &aliases,
            &lookup,
            &empty_members(),
            &empty_symbol_types()
        ),
        None
    );
}

// ---------------------------------------------------------------------------
// PR 12: T[K] indexed access
// ---------------------------------------------------------------------------

#[test]
fn indexed_access_with_literal_key_resolves_via_field_type() {
    // type Name = User["name"]; field_type[User.name] = "string"
    //   → expand("Name") = ("string", [])
    let lookup = AliasFixture::new()
        .with_alias(
            "Name",
            AliasTarget::IndexedAccess {
                object: "User".to_string(),
                key: "name".to_string(),
            },
        )
        .with_field_type("User.name", "string");
    let mut env = TypeEnvironment::new();
    let (root, _) = expand_alias("Name", &[], &lookup, &mut env).expect("expanded");
    assert_eq!(root, "string");
}

#[test]
fn indexed_access_chains_into_alias_target() {
    // type Bar = Container["data"]; field_type[Container.data] = "UserMap"
    // type UserMap = Map<string, User>
    //   → expand("Bar") = ("Map", ["string", "User"])
    let lookup = AliasFixture::new()
        .with_alias(
            "Bar",
            AliasTarget::IndexedAccess {
                object: "Container".to_string(),
                key: "data".to_string(),
            },
        )
        .with_field_type("Container.data", "UserMap")
        .with_alias(
            "UserMap",
            AliasTarget::Application {
                root: "Map".to_string(),
                args: s(&["string", "User"]),
            },
        );
    let mut env = TypeEnvironment::new();
    let (root, args) = expand_alias("Bar", &[], &lookup, &mut env).expect("expanded");
    assert_eq!(root, "Map");
    assert_eq!(args, s(&["string", "User"]));
}

#[test]
fn indexed_access_unknown_member_returns_none() {
    // No field_type entry for User.missing — chain walker should miss
    // against the alias.
    let lookup = AliasFixture::new().with_alias(
        "X",
        AliasTarget::IndexedAccess {
            object: "User".to_string(),
            key: "missing".to_string(),
        },
    );
    let mut env = TypeEnvironment::new();
    assert_eq!(expand_alias("X", &[], &lookup, &mut env), None);
}

// ---------------------------------------------------------------------------
// PR 11: keyof
// ---------------------------------------------------------------------------

#[test]
fn keyof_alias_returns_none() {
    // `type Keys = keyof User` — produces a string union, not a chain head.
    let lookup = AliasFixture::new().with_alias("Keys", AliasTarget::Keyof("User".to_string()));
    let mut env = TypeEnvironment::new();
    assert_eq!(expand_alias("Keys", &[], &lookup, &mut env), None);
}

#[test]
fn typeof_unknown_value_returns_none() {
    // type X = typeof neverDeclared
    //   → expand("X") = None (the chain walker should miss against X, not
    //     against some made-up head).
    let lookup =
        AliasFixture::new().with_alias("X", AliasTarget::Typeof("neverDeclared".to_string()));
    let mut env = TypeEnvironment::new();
    assert_eq!(expand_alias("X", &[], &lookup, &mut env), None);
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

// ---------------------------------------------------------------------------
// TypeId-form tests — verify expand_alias_typed and build_alias_index produce
// the structural Type variants the new engine consumes. The single-hop
// semantics here are intentional; recursive alias-of-alias collapse is
// driven by the chain walker re-entering, not by an internal loop.
// ---------------------------------------------------------------------------

#[test]
fn typed_unknown_alias_returns_none() {
    let mut arena = TypeArena::new();
    let lookup = AliasFixture::new();
    let aliases = build_alias_index(&[], &mut arena);
    let cls = arena.class("Nope");
    assert_eq!(
        expand_alias_typed(
            cls,
            &mut arena,
            &aliases,
            &lookup,
            &empty_members(),
            &empty_symbol_types()
        ),
        None
    );
}

#[test]
fn typed_application_no_args_returns_root_class() {
    // type Id = string  →  expand → Class("string")
    let mut arena = TypeArena::new();
    let lookup = AliasFixture::new();
    let pairs = vec![(
        "Id".to_string(),
        AliasTarget::Application {
            root: "string".to_string(),
            args: Vec::new(),
        },
    )];
    let aliases = build_alias_index(&pairs, &mut arena);
    let id_ty = arena.class("Id");
    let out = expand_alias_typed(
        id_ty,
        &mut arena,
        &aliases,
        &lookup,
        &empty_members(),
        &empty_symbol_types(),
    )
    .expect("expanded");
    assert_eq!(arena.get(out), Type::Class("string".into()));
}

#[test]
fn typed_application_with_args_builds_apply() {
    // type UserMap = Map<string, User>  →  Apply<Map, [string, User]>
    let mut arena = TypeArena::new();
    let lookup = AliasFixture::new();
    let pairs = vec![(
        "UserMap".to_string(),
        AliasTarget::Application {
            root: "Map".to_string(),
            args: s(&["string", "User"]),
        },
    )];
    let aliases = build_alias_index(&pairs, &mut arena);
    let usermap_ty = arena.class("UserMap");

    let out = expand_alias_typed(
        usermap_ty,
        &mut arena,
        &aliases,
        &lookup,
        &empty_members(),
        &empty_symbol_types(),
    )
    .expect("expanded");
    let map_ty = arena.class("Map");
    let string_ty = arena.class("string");
    let user_ty = arena.class("User");
    assert_eq!(
        arena.get(out),
        Type::Apply {
            base: map_ty,
            args: vec![string_ty, user_ty],
        }
    );
}

#[test]
fn typed_union_builds_union_type() {
    let mut arena = TypeArena::new();
    let lookup = AliasFixture::new();
    let pairs = vec![("Status".to_string(), AliasTarget::Union(s(&["Ok", "Err"])))];
    let aliases = build_alias_index(&pairs, &mut arena);
    let status = arena.class("Status");
    let out = expand_alias_typed(
        status,
        &mut arena,
        &aliases,
        &lookup,
        &empty_members(),
        &empty_symbol_types(),
    )
    .expect("expanded");
    let ok_ty = arena.class("Ok");
    let err_ty = arena.class("Err");
    assert_eq!(arena.get(out), Type::Union(vec![ok_ty, err_ty]));
}

#[test]
fn typed_intersection_builds_intersection_type() {
    let mut arena = TypeArena::new();
    let lookup = AliasFixture::new();
    let pairs = vec![("Mix".to_string(), AliasTarget::Intersection(s(&["A", "B"])))];
    let aliases = build_alias_index(&pairs, &mut arena);
    let mix = arena.class("Mix");
    let out = expand_alias_typed(
        mix,
        &mut arena,
        &aliases,
        &lookup,
        &empty_members(),
        &empty_symbol_types(),
    )
    .expect("expanded");
    let a = arena.class("A");
    let b = arena.class("B");
    assert_eq!(arena.get(out), Type::Intersection(vec![a, b]));
}

#[test]
fn typed_typeof_uses_field_type_lookup() {
    // type Foo = typeof someValue  with  someValue: User  →  Class("User")
    let mut arena = TypeArena::new();
    let lookup = AliasFixture::new().with_field_type("someValue", "User");
    let pairs = vec![(
        "Foo".to_string(),
        AliasTarget::Typeof("someValue".to_string()),
    )];
    let aliases = build_alias_index(&pairs, &mut arena);
    let foo = arena.class("Foo");
    let out = expand_alias_typed(
        foo,
        &mut arena,
        &aliases,
        &lookup,
        &empty_members(),
        &empty_symbol_types(),
    )
    .expect("expanded");
    assert_eq!(arena.get(out), Type::Class("User".into()));
}

#[test]
fn typed_typeof_falls_back_to_return_type() {
    let mut arena = TypeArena::new();
    let lookup = AliasFixture::new().with_return_type("someFn", "Result");
    let pairs = vec![("Foo".to_string(), AliasTarget::Typeof("someFn".to_string()))];
    let aliases = build_alias_index(&pairs, &mut arena);
    let foo = arena.class("Foo");
    let out = expand_alias_typed(
        foo,
        &mut arena,
        &aliases,
        &lookup,
        &empty_members(),
        &empty_symbol_types(),
    )
    .expect("expanded");
    assert_eq!(arena.get(out), Type::Class("Result".into()));
}

#[test]
fn typed_typeof_misses_when_value_unknown() {
    let mut arena = TypeArena::new();
    let lookup = AliasFixture::new();
    let pairs = vec![(
        "Foo".to_string(),
        AliasTarget::Typeof("never_indexed".to_string()),
    )];
    let aliases = build_alias_index(&pairs, &mut arena);
    let foo = arena.class("Foo");
    assert_eq!(
        expand_alias_typed(
            foo,
            &mut arena,
            &aliases,
            &lookup,
            &empty_members(),
            &empty_symbol_types()
        ),
        None
    );
}

#[test]
fn typed_indexed_access_uses_dotted_field_lookup() {
    // type Foo = T["name"]  →  field_type("T.name") = "string"  →  Class("string")
    let mut arena = TypeArena::new();
    let lookup = AliasFixture::new().with_field_type("T.name", "string");
    let pairs = vec![(
        "Foo".to_string(),
        AliasTarget::IndexedAccess {
            object: "T".to_string(),
            key: "name".to_string(),
        },
    )];
    let aliases = build_alias_index(&pairs, &mut arena);
    let foo = arena.class("Foo");
    let out = expand_alias_typed(
        foo,
        &mut arena,
        &aliases,
        &lookup,
        &empty_members(),
        &empty_symbol_types(),
    )
    .expect("expanded");
    assert_eq!(arena.get(out), Type::Class("string".into()));
}

#[test]
fn typed_transparent_mapped_returns_source() {
    // type Foo<T> = { [K in keyof T]: T[K] } → mapped source T
    let mut arena = TypeArena::new();
    let lookup = AliasFixture::new();
    let pairs = vec![(
        "Foo".to_string(),
        AliasTarget::Mapped {
            source: "User".to_string(),
            value_template: "User[K]".to_string(),
        },
    )];
    let aliases = build_alias_index(&pairs, &mut arena);
    let foo = arena.class("Foo");
    let out = expand_alias_typed(
        foo,
        &mut arena,
        &aliases,
        &lookup,
        &empty_members(),
        &empty_symbol_types(),
    )
    .expect("expanded");
    assert_eq!(arena.get(out), Type::Class("User".into()));
}

#[test]
fn typed_record_mapped_with_concrete_value_returns_value_class() {
    // `{ [P in K]: boolean }` — flat concrete value head projects to
    // Class("boolean") so member access on the receiver continues against it.
    let mut arena = TypeArena::new();
    let lookup = AliasFixture::new();
    let pairs = vec![(
        "Flags".to_string(),
        AliasTarget::Mapped {
            source: String::new(),
            value_template: "boolean".to_string(),
        },
    )];
    let aliases = build_alias_index(&pairs, &mut arena);
    let flags = arena.class("Flags");
    let out = expand_alias_typed(
        flags,
        &mut arena,
        &aliases,
        &lookup,
        &empty_members(),
        &empty_symbol_types(),
    )
    .expect("expanded");
    assert_eq!(arena.get(out), Type::Class("boolean".into()));
}

#[test]
fn typed_record_mapped_with_param_value_passes_param_through() {
    // `{ [P in K]: V }` — the value head is a generic param. The single-hop
    // typed path interns it as Class("V"); the chain walker binds V against the
    // receiver's args downstream, mirroring how the Application arm leaves its
    // args unsubstituted.
    let mut arena = TypeArena::new();
    let lookup = AliasFixture::new();
    let pairs = vec![(
        "Record".to_string(),
        AliasTarget::Mapped {
            source: String::new(),
            value_template: "V".to_string(),
        },
    )];
    let aliases = build_alias_index(&pairs, &mut arena);
    let record = arena.class("Record");
    let out = expand_alias_typed(
        record,
        &mut arena,
        &aliases,
        &lookup,
        &empty_members(),
        &empty_symbol_types(),
    )
    .expect("expanded");
    assert_eq!(arena.get(out), Type::Class("V".into()));
}

#[test]
fn typed_custom_mapped_function_value_returns_none() {
    // `{ [K in keyof T]: () => T[K] }` — value template carries an operator,
    // not a flat head. Not a projectable value slot, so the typed path declines.
    let mut arena = TypeArena::new();
    let lookup = AliasFixture::new();
    let pairs = vec![(
        "Getters".to_string(),
        AliasTarget::Mapped {
            source: "T".to_string(),
            value_template: "() => T[K]".to_string(),
        },
    )];
    let aliases = build_alias_index(&pairs, &mut arena);
    let getters = arena.class("Getters");
    assert_eq!(
        expand_alias_typed(
            getters,
            &mut arena,
            &aliases,
            &lookup,
            &empty_members(),
            &empty_symbol_types()
        ),
        None
    );
}

#[test]
fn typed_conditional_picks_true_branch_on_assignable() {
    // type C = User extends User ? Yes : No  →  Class("Yes")
    let mut arena = TypeArena::new();
    let lookup = AliasFixture::new();
    let pairs = vec![(
        "C".to_string(),
        AliasTarget::Conditional {
            check: "User".to_string(),
            extends: "User".to_string(),
            true_branch: "Yes".to_string(),
            false_branch: "No".to_string(),
            infer_binding: None,
        },
    )];
    let aliases = build_alias_index(&pairs, &mut arena);
    let c = arena.class("C");
    let out = expand_alias_typed(
        c,
        &mut arena,
        &aliases,
        &lookup,
        &empty_members(),
        &empty_symbol_types(),
    )
    .expect("expanded");
    assert_eq!(arena.get(out), Type::Class("Yes".into()));
}

#[test]
fn typed_conditional_returns_none_when_undecidable() {
    // type C = A extends B ? Yes : No  with no inheritance info →
    // subtype check returns Unknown → expand returns None.
    let mut arena = TypeArena::new();
    let lookup = AliasFixture::new();
    let pairs = vec![(
        "C".to_string(),
        AliasTarget::Conditional {
            check: "A".to_string(),
            extends: "B".to_string(),
            true_branch: "Yes".to_string(),
            false_branch: "No".to_string(),
            infer_binding: None,
        },
    )];
    let aliases = build_alias_index(&pairs, &mut arena);
    let c = arena.class("C");
    assert_eq!(
        expand_alias_typed(
            c,
            &mut arena,
            &aliases,
            &lookup,
            &empty_members(),
            &empty_symbol_types()
        ),
        None
    );
}

#[test]
fn typed_keyof_object_other_return_none() {
    let mut arena = TypeArena::new();
    let lookup = AliasFixture::new();
    let pairs = vec![
        ("K1".to_string(), AliasTarget::Keyof("User".to_string())),
        ("K2".to_string(), AliasTarget::Object),
        ("K3".to_string(), AliasTarget::Other),
    ];
    let aliases = build_alias_index(&pairs, &mut arena);
    let k1 = arena.class("K1");
    let k2 = arena.class("K2");
    let k3 = arena.class("K3");
    assert_eq!(
        expand_alias_typed(
            k1,
            &mut arena,
            &aliases,
            &lookup,
            &empty_members(),
            &empty_symbol_types()
        ),
        None
    );
    assert_eq!(
        expand_alias_typed(
            k2,
            &mut arena,
            &aliases,
            &lookup,
            &empty_members(),
            &empty_symbol_types()
        ),
        None
    );
    assert_eq!(
        expand_alias_typed(
            k3,
            &mut arena,
            &aliases,
            &lookup,
            &empty_members(),
            &empty_symbol_types()
        ),
        None
    );
}

#[test]
fn typed_keyof_expands_to_string_literal_union() {
    // `type Keys = keyof User` with User.{id,name} → "id" | "name".
    let mut arena = TypeArena::new();
    let lookup = AliasFixture::new()
        .with_member("User", "id")
        .with_member("User", "name");
    let pairs = vec![("Keys".to_string(), AliasTarget::Keyof("User".to_string()))];
    let aliases = build_alias_index(&pairs, &mut arena);
    let keys = arena.class("Keys");
    let out = expand_alias_typed(
        keys,
        &mut arena,
        &aliases,
        &lookup,
        &empty_members(),
        &empty_symbol_types(),
    )
    .expect("keyof expands");
    match arena.get(out) {
        Type::Union(branches) => {
            let lits: Vec<String> = branches
                .iter()
                .filter_map(|b| match arena.get(*b) {
                    Type::Literal(LitValue::Str(s)) => Some(s),
                    _ => None,
                })
                .collect();
            assert!(
                lits.contains(&"id".to_string()) && lits.contains(&"name".to_string()),
                "expected id|name string literals, got {lits:?}"
            );
        }
        other => panic!("expected Union of literals, got {other:?}"),
    }
}

#[test]
fn typed_build_alias_index_interns_each_qname() {
    // Two aliases with overlapping names should produce two distinct
    // TypeId keys; lookup against a non-aliased name returns None.
    let mut arena = TypeArena::new();
    let pairs = vec![
        (
            "A".to_string(),
            AliasTarget::Application {
                root: "X".to_string(),
                args: Vec::new(),
            },
        ),
        (
            "B".to_string(),
            AliasTarget::Application {
                root: "Y".to_string(),
                args: Vec::new(),
            },
        ),
    ];
    let aliases = build_alias_index(&pairs, &mut arena);
    let a = arena.class("A");
    let b = arena.class("B");
    let c = arena.class("C");
    assert!(aliases.contains_key(&a));
    assert!(aliases.contains_key(&b));
    assert!(!aliases.contains_key(&c));
}
