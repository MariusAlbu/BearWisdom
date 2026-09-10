use super::*;
use crate::indexer::{
    resolve::engine::{
        compilation::Compilation,
        contract::{FlowCacheLookup, Symbol, SymbolLookup, SymbolSet, TypeInfo},
        testkit::{sym, sym_with_sig, Lookup},
    },
    symbol_ids::SymbolIds,
};
use crate::type_checker::core::types::{
    GenericParamData, GenericParamKind, Indirection, Lifetime, Mutability, Type, TypeId,
};
use crate::type_checker::profile::language_profile::DelegateShape;
use crate::types::CallArg;
use std::cell::RefCell;
use std::sync::Arc;

/// A legacy-only lookup that exposes one declared generic method and records
/// the name-keyed callback write made by `apply_with_receiver`.
struct CallbackSeedLookup {
    callee: Symbol,
    generic_params: Option<Vec<String>>,
    empty: Vec<Symbol>,
    pairs: Vec<(String, String)>,
    seeded: RefCell<Vec<(String, TypeId)>>,
}

impl CallbackSeedLookup {
    fn new(callee: Symbol) -> Self {
        Self {
            callee,
            generic_params: Some(vec!["T".to_string()]),
            empty: Vec::new(),
            pairs: Vec::new(),
            seeded: RefCell::new(Vec::new()),
        }
    }

    fn without_generic_metadata(callee: Symbol) -> Self {
        Self {
            callee,
            generic_params: None,
            empty: Vec::new(),
            pairs: Vec::new(),
            seeded: RefCell::new(Vec::new()),
        }
    }
}

impl FlowCacheLookup for CallbackSeedLookup {
    fn record_local_type_id(&self, name: String, ty: TypeId) {
        self.seeded.borrow_mut().push((name, ty));
    }
}

impl SymbolLookup for CallbackSeedLookup {
    fn by_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn by_qualified_name(&self, qname: &str) -> Option<&Symbol> {
        (qname == self.callee.qualified_name).then_some(&self.callee)
    }
    fn members_of(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn types_by_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn in_namespace(&self, _: &str) -> Vec<&Symbol> {
        Vec::new()
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn field_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn return_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn generic_params(&self, qname: &str) -> Option<Vec<String>> {
        (qname == self.callee.qualified_name)
            .then(|| self.generic_params.clone())
            .flatten()
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &self.pairs
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
}

#[test]
fn zero_argument_calls_substitute_only_attested_matching_receiver_regions() {
    let arena = Arc::new(TypeArena::new());
    let mut lookup = Compilation::build(&[], &SymbolIds::default(), Arc::clone(&arena));
    let region = |owner| {
        arena.intern_generic(GenericParamData {
            name: "'a".into(),
            kind: GenericParamKind::Lifetime,
            owner_symbol_index: owner,
            bound: None,
        })
    };
    let method = region(7);
    let caller = region(8);
    let other = region(9);
    let nominal = arena.decl("C", 41);
    let twin = arena.decl("C", 42);
    let reference = |region, mutability, inner| {
        arena.intern(Type::Indirect {
            kind: Indirection::Reference(region),
            mutability,
            inner,
        })
    };
    let yielded = arena.generic_type(method);
    let unknown = arena.intern(Type::Region(Lifetime::Unknown));
    lookup.type_info_by_id.insert(
        7,
        TypeInfo {
            receiver_type_id: Some(reference(
                Lifetime::Parameter(method),
                Mutability::Shared,
                nominal,
            )),
            elided_input_params: vec![(17, 0, method)],
            parameter_type_ids: Some(vec![]),
            return_type_id: Some(yielded),
            ..Default::default()
        },
    );
    let callee = sym(7, "make", "C.make", "method", "lib.rs");
    let correct = reference(Lifetime::Parameter(caller), Mutability::Shared, nominal);
    for (borrowed, expected) in [
        (Some(correct), arena.generic_type(caller)),
        (
            Some(reference(
                Lifetime::Inference {
                    owner: 100,
                    byte: 17,
                },
                Mutability::Shared,
                nominal,
            )),
            arena.intern(Type::Region(Lifetime::Inference {
                owner: 100,
                byte: 17,
            })),
        ),
        (
            Some(reference(Lifetime::Static, Mutability::Shared, nominal)),
            arena.intern(Type::Region(Lifetime::Static)),
        ),
        (None, unknown),
        (
            Some(reference(Lifetime::Unknown, Mutability::Shared, nominal)),
            unknown,
        ),
        (
            Some(reference(
                Lifetime::Parameter(caller),
                Mutability::Shared,
                twin,
            )),
            unknown,
        ),
        (
            Some(reference(
                Lifetime::Parameter(caller),
                Mutability::Mutable,
                nominal,
            )),
            unknown,
        ),
    ] {
        let got = apply_with_receiver(
            &lookup,
            &arena,
            &callee,
            0,
            &[],
            &[],
            nominal,
            Some(41),
            Some(yielded),
            &[],
            borrowed,
        );
        assert_eq!(got, Some(expected));
    }
    // An ordinary argument with the same display name owns a DIFFERENT ID.
    lookup
        .type_info_by_id
        .get_mut(&7)
        .unwrap()
        .parameter_type_ids = Some(vec![reference(
        Lifetime::Parameter(other),
        Mutability::Shared,
        nominal,
    )]);
    lookup
        .type_info_by_id
        .get_mut(&7)
        .unwrap()
        .elided_input_params
        .push((27, 0, other));
    let actual = reference(Lifetime::Static, Mutability::Shared, nominal);
    let env = bound_call::environment_with_receiver(
        &lookup,
        &arena,
        &callee,
        nominal,
        Some(41),
        &[],
        &[actual],
        Some(correct),
    )
    .unwrap();
    assert_eq!(env[&method], arena.generic_type(caller));
    assert_eq!(env[&other], arena.intern(Type::Region(Lifetime::Static)));
    let env = bound_call::environment_with_receiver(
        &lookup,
        &arena,
        &callee,
        nominal,
        Some(41),
        &[],
        &[actual],
        None,
    )
    .unwrap();
    assert_eq!(
        env[&method], unknown,
        "ordinary input must not impersonate receiver evidence"
    );
}

#[test]
fn legacy_callback_seeding_uses_the_one_callback_shaped_overload() {
    let non_callback = sym_with_sig(
        30,
        "Entity",
        "ModelBuilder.Entity",
        "method",
        "src/ModelBuilder.cs",
        "Entity(string, string): EntityTypeBuilder",
    );
    let callback = sym_with_sig(
        31,
        "Entity",
        "ModelBuilder.Entity",
        "method",
        "src/ModelBuilder.cs",
        "Entity(string, Action<EntityTypeBuilder>): EntityTypeBuilder",
    );
    let lookup = Lookup::new().with(non_callback.clone()).with(callback);
    let arena = lookup.type_arena().expect("arena");
    let args = vec![
        CallArg::StringLit("users".into()),
        CallArg::Lambda {
            params: vec!["builder".into()],
        },
    ];

    let selected = super::_test_uniquely_contextual_callback_callee(
        &lookup,
        arena,
        &non_callback,
        &args,
        &[("Action", DelegateShape::AllParams)],
    );
    assert_eq!(selected.map(|symbol| symbol.id), Some(31));
}

#[test]
fn legacy_callback_seeding_abstains_for_equal_arity_callback_overloads() {
    let first = sym_with_sig(
        30,
        "Entity",
        "ModelBuilder.Entity",
        "method",
        "src/ModelBuilder.cs",
        "Entity(string, Action<FirstBuilder>): EntityTypeBuilder",
    );
    let second = sym_with_sig(
        31,
        "Entity",
        "ModelBuilder.Entity",
        "method",
        "src/ModelBuilder.cs",
        "Entity(string, Action<SecondBuilder>): EntityTypeBuilder",
    );
    let lookup = Lookup::new().with(first.clone()).with(second);
    let arena = lookup.type_arena().expect("arena");
    let args = vec![
        CallArg::StringLit("users".into()),
        CallArg::Lambda {
            params: vec!["builder".into()],
        },
    ];

    assert!(super::_test_uniquely_contextual_callback_callee(
        &lookup,
        arena,
        &first,
        &args,
        &[("Action", DelegateShape::AllParams)],
    )
    .is_none());
}

#[test]
fn java_callback_pattern_requires_the_exact_jdk_functional_interface_name() {
    let custom = sym_with_sig(
        32,
        "Use",
        "Runner.Use",
        "method",
        "src/Runner.java",
        "void Use(com.acme.Function<Left, Result> callback)",
    );
    let jdk = sym_with_sig(
        33,
        "Use",
        "Runner.UseJdk",
        "method",
        "src/Runner.java",
        "void Use(java.util.function.BiFunction<Left, Right, Result> callback)",
    );
    let lookup = Lookup::new().with(custom.clone()).with(jdk.clone());
    let arena = lookup.type_arena().expect("arena");
    let args = vec![CallArg::Lambda {
        params: vec!["left".into(), "right".into()],
    }];
    let wrappers = crate::languages::java::JAVA_PROFILE.delegate_wrappers;

    assert!(super::_test_uniquely_contextual_callback_callee(
        &lookup, arena, &custom, &args, wrappers,
    )
    .is_none());
    assert_eq!(
        super::_test_uniquely_contextual_callback_callee(&lookup, arena, &jdk, &args, wrappers)
            .map(|symbol| symbol.id),
        Some(jdk.id)
    );
}

#[test]
fn explicit_method_arguments_bind_legacy_callback_generics_by_declaration_order() {
    let callee = sym_with_sig(
        40,
        "Use",
        "Runner.Use",
        "method",
        "src/Runner.cs",
        "void Use(Action<T> callback)",
    );
    let lookup = Lookup::new()
        .with(callee.clone())
        .with_generics("Runner.Use", &["T"]);
    let arena = lookup.type_arena().expect("arena");
    let account = arena.class("Account");

    let env = super::legacy_callback_env(&lookup, arena, &callee, &[], &[account])
        .unwrap()
        .bindings;

    assert_eq!(env.get("T"), Some(&account));
}

#[test]
fn missing_explicit_method_arguments_leave_later_callback_generics_open() {
    let callee = sym_with_sig(
        40,
        "Use",
        "Runner.Use",
        "method",
        "src/Runner.cs",
        "void Use(Action<T> first, Action<U> second)",
    );
    let lookup = Lookup::new()
        .with(callee.clone())
        .with_generics("Runner.Use", &["T", "U"]);
    let arena = lookup.type_arena().expect("arena");
    let account = arena.class("Account");

    let env = super::legacy_callback_env(&lookup, arena, &callee, &[], &[account])
        .unwrap()
        .bindings;

    assert_eq!(env.get("T"), Some(&account));
    assert!(env.get("U").is_none());
}

#[test]
fn explicit_method_arguments_override_inferred_legacy_callback_generics() {
    let callee = sym_with_sig(
        40,
        "Use",
        "Runner.Use",
        "method",
        "src/Runner.cs",
        "void Use(T value, Action<T> callback)",
    );
    let lookup = Lookup::new()
        .with(callee.clone())
        .with_generics("Runner.Use", &["T"])
        .with_local_type("user", "User");
    let arena = lookup.type_arena().expect("arena");
    let args = vec![
        CallArg::Ident("user".into()),
        CallArg::Lambda {
            params: vec!["x".into()],
        },
    ];
    let actual = super::resolve_arg_types(&lookup, arena, &args);

    let account = arena.class("Account");
    let env = super::legacy_callback_env(&lookup, arena, &callee, &actual, &[account])
        .unwrap()
        .bindings;

    assert_eq!(env["T"], account);
}

#[test]
fn explicit_method_argument_seeds_the_legacy_lambda_parameter_through_apply_with_receiver() {
    let callee = sym_with_sig(
        50,
        "Use",
        "Runner.Use",
        "method",
        "src/Runner.cs",
        "T Use(Action<T> callback)",
    );
    let lookup = CallbackSeedLookup::new(callee.clone());
    let arena = TypeArena::new();
    let account = arena.class("Account");
    let args = vec![CallArg::Lambda {
        params: vec!["x".into()],
    }];

    let yielded = apply_with_receiver(
        &lookup,
        &arena,
        &callee,
        0,
        &args,
        &[account],
        arena.intern(Type::Unknown),
        None,
        Some(arena.class("T")),
        &[("Action", DelegateShape::AllParams)],
        None,
    );

    assert_eq!(yielded, Some(account));
    let seeded = lookup.seeded.borrow();
    assert_eq!(seeded.len(), 1);
    assert_eq!(seeded[0].0, "x");
    assert_eq!(seeded[0].1, account);
}

#[test]
fn explicit_method_arguments_override_inferred_callback_types_through_apply_with_receiver() {
    let callee = sym_with_sig(
        50,
        "Use",
        "Runner.Use",
        "method",
        "src/Runner.cs",
        "void Use(T value, Action<T> callback)",
    );
    let lookup = CallbackSeedLookup::new(callee.clone());
    let arena = TypeArena::new();
    let args = vec![
        CallArg::StringLit("user".into()),
        CallArg::Lambda {
            params: vec!["x".into()],
        },
    ];

    let account = arena.class("Account");
    let yielded = apply_with_receiver(
        &lookup,
        &arena,
        &callee,
        0,
        &args,
        &[account],
        arena.intern(Type::Unknown),
        None,
        Some(arena.class("T")),
        &[("Action", DelegateShape::AllParams)],
        None,
    );

    assert_eq!(yielded, Some(account));
    let seeded = lookup.seeded.borrow();
    assert_eq!(seeded.len(), 1);
    assert_eq!(seeded[0], ("x".to_string(), account));
}

#[test]
fn excess_explicit_arguments_do_not_seed_legacy_callback_parameters() {
    let callee = sym_with_sig(
        50,
        "Use",
        "Runner.Use",
        "method",
        "src/Runner.cs",
        "T Use(Action<T> callback)",
    );
    let lookup = CallbackSeedLookup::new(callee.clone());
    let arena = TypeArena::new();
    let args = vec![CallArg::Lambda {
        params: vec!["x".into()],
    }];

    let yielded = apply_with_receiver(
        &lookup,
        &arena,
        &callee,
        0,
        &args,
        &[arena.class("Account"), arena.class("Unexpected")],
        arena.intern(Type::Unknown),
        None,
        Some(arena.class("T")),
        &[("Action", DelegateShape::AllParams)],
        None,
    );

    assert_eq!(yielded, None);
    assert!(lookup.seeded.borrow().is_empty());
}

#[test]
fn explicit_legacy_call_without_a_declared_generic_clause_preserves_its_open_yield_without_seeding()
{
    let callee = sym_with_sig(
        50,
        "Use",
        "Runner.Use",
        "method",
        "src/Runner.cs",
        "T Use(Action<T> callback)",
    );
    let lookup = CallbackSeedLookup::without_generic_metadata(callee.clone());
    let arena = TypeArena::new();
    let open = arena.class("T");
    let args = vec![CallArg::Lambda {
        params: vec!["x".into()],
    }];

    let yielded = apply_with_receiver(
        &lookup,
        &arena,
        &callee,
        0,
        &args,
        &[arena.class("Account")],
        arena.intern(Type::Unknown),
        None,
        Some(open),
        &[("Action", DelegateShape::AllParams)],
        None,
    );

    assert_eq!(yielded, Some(open));
    assert!(lookup.seeded.borrow().is_empty());
}

#[test]
fn explicit_type_args_bind_a_zero_argument_legacy_generic_call() {
    let callee = sym_with_sig(
        50,
        "Get",
        "Runner.Get",
        "method",
        "src/Runner.cs",
        "T Get<T>()",
    );
    let lookup = CallbackSeedLookup::without_generic_metadata(callee.clone());
    let arena = TypeArena::new();
    let account = arena.class("Account");

    let yielded = apply_with_receiver(
        &lookup,
        &arena,
        &callee,
        0,
        &[],
        &[account],
        arena.intern(Type::Unknown),
        None,
        Some(arena.class("T")),
        &[],
        None,
    );

    assert_eq!(yielded, Some(account));
}

#[test]
fn open_signature_generic_without_explicit_args_does_not_seed_a_legacy_callback() {
    let callee = sym_with_sig(
        50,
        "Use",
        "Runner.Use",
        "method",
        "src/Runner.cs",
        "T Use<T>(Action<T> callback)",
    );
    let lookup = CallbackSeedLookup::without_generic_metadata(callee.clone());
    let arena = TypeArena::new();
    let open = arena.class("T");
    let args = vec![CallArg::Lambda {
        params: vec!["x".into()],
    }];

    let yielded = apply_with_receiver(
        &lookup,
        &arena,
        &callee,
        0,
        &args,
        &[],
        arena.intern(Type::Unknown),
        None,
        Some(open),
        &[("Action", DelegateShape::AllParams)],
        None,
    );

    assert_eq!(yielded, Some(open));
    assert!(lookup.seeded.borrow().is_empty());
}

#[test]
fn concrete_legacy_callback_without_generic_metadata_still_seeds() {
    let callee = sym_with_sig(
        50,
        "Use",
        "Runner.Use",
        "method",
        "src/Runner.cs",
        "void Use(Action<Account> callback)",
    );
    let lookup = CallbackSeedLookup::without_generic_metadata(callee.clone());
    let arena = TypeArena::new();
    let account = arena.class("Account");
    let args = vec![CallArg::Lambda {
        params: vec!["x".into()],
    }];

    apply_with_receiver(
        &lookup,
        &arena,
        &callee,
        0,
        &args,
        &[],
        arena.intern(Type::Unknown),
        None,
        None,
        &[("Action", DelegateShape::AllParams)],
        None,
    );

    let seeded = lookup.seeded.borrow();
    assert_eq!(seeded.len(), 1);
    assert_eq!(seeded[0], ("x".to_string(), account));
}
