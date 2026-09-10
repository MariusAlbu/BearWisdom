use std::cell::RefCell;

use super::*;
use crate::indexer::resolve::engine::contract::{Symbol as ContractSymbol, SymbolSet};
use crate::type_checker::core::types::Type;

/// Minimal lookup: generic parameters for one owner, plus a recording local
/// cache so a seeded lambda parameter is observable.
struct SeedLookup {
    owner: String,
    params: Vec<String>,
    empty: Vec<ContractSymbol>,
    empty_pairs: Vec<(String, String)>,
    seeded: RefCell<Vec<(String, TypeId)>>,
    contextual: RefCell<Vec<(crate::types::SourceSpan, TypeId)>>,
}

impl SeedLookup {
    fn new(owner: &str, params: &[&str]) -> Self {
        Self {
            owner: owner.to_string(),
            params: params.iter().map(|s| (*s).to_string()).collect(),
            empty: Vec::new(),
            empty_pairs: Vec::new(),
            seeded: RefCell::new(Vec::new()),
            contextual: RefCell::new(Vec::new()),
        }
    }
}

impl crate::indexer::resolve::engine::contract::FlowCacheLookup for SeedLookup {
    fn record_local_type_id(&self, name: String, id: TypeId) {
        self.seeded.borrow_mut().push((name, id));
    }
    fn record_local_type(&self, _: String, _: String) {}
    fn record_contextual_type(&self, parameter: crate::types::SourceSpan, ty: TypeId) {
        self.contextual.borrow_mut().push((parameter, ty));
    }
}

impl SymbolLookup for SeedLookup {
    fn by_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn by_qualified_name(&self, _: &str) -> Option<&ContractSymbol> {
        None
    }
    fn members_of(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn types_by_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn in_namespace(&self, _: &str) -> Vec<&ContractSymbol> {
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
    fn generic_params(&self, type_name: &str) -> Option<Vec<String>> {
        (type_name == self.owner).then(|| self.params.clone())
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &self.empty_pairs
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
}

/// `map(fn: (value: T) => U): Array<U>` declared on `Array<T>`.
fn map_method() -> Symbol {
    Symbol {
        signature: Some("map(fn: (value: T) => U): Array<U>".to_string()),
        ..crate::indexer::resolve::engine::testkit::sym(1, "map", "Array.map", "method", "a.ts")
    }
}

/// `map(value: T, fn: (value: T) => U): Array<U>` declared on `Array<T>`.
fn map_with_value_method() -> Symbol {
    Symbol {
        signature: Some("map(value: T, fn: (value: T) => U): Array<U>".to_string()),
        ..crate::indexer::resolve::engine::testkit::sym(2, "map", "Array.map", "method", "a.ts")
    }
}

fn array_of(arena: &TypeArena, elem: &str) -> TypeId {
    arena.intern(Type::Apply {
        base: arena.class("Array"),
        args: vec![arena.class(elem)],
    })
}

#[test]
fn a_lambda_parameter_is_seeded_from_the_receivers_element_type() {
    let lookup = SeedLookup::new("Array", &["T"]);
    let arena = TypeArena::new();
    let args = vec![CallArg::Lambda {
        params: vec!["x".to_string()],
    }];

    seed_lambda_params(
        &lookup,
        &arena,
        &map_method(),
        &args,
        array_of(&arena, "User"),
        None,
        &FxHashMap::default(),
        &[],
    );

    let seeded = lookup.seeded.borrow();
    assert_eq!(seeded.len(), 1);
    assert_eq!(seeded[0].0, "x");
    assert_eq!(arena.get(seeded[0].1), Type::Class("User".to_string()));
}

#[test]
fn an_unbindable_receiver_seeds_nothing() {
    let lookup = SeedLookup::new("Array", &["T"]);
    let arena = TypeArena::new();
    let args = vec![CallArg::Lambda {
        params: vec!["x".to_string()],
    }];

    // Bare `Array` — no applied argument, so the callback parameter stays `T`.
    seed_lambda_params(
        &lookup,
        &arena,
        &map_method(),
        &args,
        arena.class("Array"),
        None,
        &FxHashMap::default(),
        &[],
    );

    assert!(lookup.seeded.borrow().is_empty());
}

#[test]
fn a_source_addressed_lambda_parameter_records_its_exact_span() {
    let lookup = SeedLookup::new("Array", &["T"]);
    let arena = TypeArena::new();
    let parameter = crate::types::SourceSpan { start: 24, end: 28 };
    let args = vec![CallArg::LambdaAt {
        params: vec![Some(parameter)],
    }];

    seed_lambda_params(
        &lookup,
        &arena,
        &map_method(),
        &args,
        array_of(&arena, "User"),
        None,
        &FxHashMap::default(),
        &[],
    );

    let contextual = lookup.contextual.borrow();
    assert_eq!(contextual.len(), 1);
    assert_eq!(contextual[0].0, parameter);
    assert_eq!(arena.get(contextual[0].1), Type::Class("User".to_string()));
    assert!(lookup.seeded.borrow().is_empty());
}

#[test]
fn a_trailing_block_parameter_records_its_exact_span() {
    let lookup = SeedLookup::new("Array", &["T"]);
    let arena = TypeArena::new();
    let parameter = crate::types::SourceSpan { start: 24, end: 28 };
    let args = vec![CallArg::TrailingBlockAt {
        params: vec![Some(parameter)],
    }];

    seed_lambda_params(
        &lookup,
        &arena,
        &map_method(),
        &args,
        array_of(&arena, "User"),
        None,
        &FxHashMap::default(),
        &[],
    );

    let contextual = lookup.contextual.borrow();
    assert_eq!(contextual.len(), 1);
    assert_eq!(contextual[0].0, parameter);
    assert_eq!(arena.get(contextual[0].1), Type::Class("User".to_string()));
}

#[test]
fn strict_ruby_block_contracts_reject_positional_callbacks_and_accept_trailing_blocks() {
    for extension in ["rbs", "rbi"] {
        let arena = TypeArena::new();
        let mut contract = map_method();
        contract.file_path = format!("contracts/catalog.{extension}").into();
        if extension == "rbi" {
            contract.signature = Some("map(&block: (T) -> U): Array<U>".into());
        }
        let parameter = crate::types::SourceSpan { start: 24, end: 28 };

        for positional in [
            CallArg::Lambda {
                params: vec!["item".to_string()],
            },
            CallArg::LambdaAt {
                params: vec![Some(parameter)],
            },
        ] {
            let lookup = SeedLookup::new("Array", &["T"]);
            seed_lambda_params(
                &lookup,
                &arena,
                &contract,
                &[positional],
                array_of(&arena, "Item"),
                None,
                &FxHashMap::default(),
                &[],
            );
            assert!(lookup.seeded.borrow().is_empty());
            assert!(lookup.contextual.borrow().is_empty());
        }

        let lookup = SeedLookup::new("Array", &["T"]);
        seed_lambda_params(
            &lookup,
            &arena,
            &contract,
            &[CallArg::TrailingBlockAt {
                params: vec![Some(parameter)],
            }],
            array_of(&arena, "Item"),
            None,
            &FxHashMap::default(),
            &[],
        );
        assert_eq!(
            lookup.contextual.borrow().as_slice(),
            &[(parameter, arena.class("Item"))]
        );
    }
}

#[test]
fn legacy_unmarked_rbi_callback_defaults_to_trailing_block_only() {
    let mut contract = map_method();
    contract.file_path = "contracts/catalog.rbi".into();
    contract.signature = Some("map(block: (Item) -> Result): void".into());
    let span = crate::types::SourceSpan { start: 24, end: 28 };

    assert_eq!(
        callback_arg_policy(&contract, 0),
        CallbackArgumentPolicy::TrailingBlockOnly,
    );
    assert!(callback_arg_is_compatible(
        &contract,
        0,
        &CallArg::TrailingBlockAt {
            params: vec![Some(span)],
        },
    ));
    assert!(!callback_arg_is_compatible(
        &contract,
        0,
        &CallArg::LambdaAt {
            params: vec![Some(span)],
        },
    ));
}

#[test]
fn direct_selected_rbi_block_contract_seeding_requires_a_trailing_block() {
    let arena = TypeArena::new();
    let mut contract = map_method();
    contract.file_path = "contracts/catalog.rbi".into();
    contract.signature = Some("map(&block: (Item) -> Result): void".into());
    let callback = arena.intern(Type::Function {
        params: vec![arena.class("Item")],
        return_: arena.class("Result"),
    });
    let parameter = crate::types::SourceSpan { start: 24, end: 28 };

    // `->(item) { ... }` is an ordinary positional argument (`LambdaAt`),
    // not the RBI declaration's attached `&block`.
    let arrow_lookup = SeedLookup::new("Array", &[]);
    seed_patterns_for_callee(
        &arrow_lookup,
        &arena,
        &contract,
        &[CallArg::LambdaAt {
            params: vec![Some(parameter)],
        }],
        &[callback],
        &Default::default(),
        &[],
        true,
    );
    assert!(arrow_lookup.contextual.borrow().is_empty());

    let block_lookup = SeedLookup::new("Array", &[]);
    seed_patterns_for_callee(
        &block_lookup,
        &arena,
        &contract,
        &[CallArg::TrailingBlockAt {
            params: vec![Some(parameter)],
        }],
        &[callback],
        &Default::default(),
        &[],
        true,
    );
    assert_eq!(
        block_lookup.contextual.borrow().as_slice(),
        &[(parameter, arena.class("Item"))]
    );

    let unproven_lookup = SeedLookup::new("Array", &[]);
    seed_patterns_for_callee(
        &unproven_lookup,
        &arena,
        &contract,
        &[CallArg::TrailingBlockAt {
            params: vec![Some(parameter)],
        }],
        &[callback],
        &Default::default(),
        &[],
        false,
    );
    assert!(unproven_lookup.contextual.borrow().is_empty());
}

#[test]
fn direct_selected_rbi_positional_proc_requires_a_source_addressed_lambda() {
    let arena = TypeArena::new();
    let mut contract = map_method();
    contract.file_path = "contracts/catalog.rbi".into();
    contract.signature = Some("visit(^callback: (Item) -> Result): void".into());
    let callback = arena.intern(Type::Function {
        params: vec![arena.class("Item")],
        return_: arena.class("Result"),
    });
    let parameter = crate::types::SourceSpan { start: 24, end: 28 };

    let direct = SeedLookup::new("Array", &[]);
    seed_patterns_for_callee(
        &direct,
        &arena,
        &contract,
        &[CallArg::LambdaAt {
            params: vec![Some(parameter)],
        }],
        &[callback],
        &Default::default(),
        &[],
        true,
    );
    assert_eq!(
        direct.contextual.borrow().as_slice(),
        &[(parameter, arena.class("Item"))]
    );

    for arg in [
        CallArg::Lambda {
            params: vec!["item".into()],
        },
        CallArg::TrailingBlockAt {
            params: vec![Some(parameter)],
        },
    ] {
        let rejected = SeedLookup::new("Array", &[]);
        seed_patterns_for_callee(
            &rejected,
            &arena,
            &contract,
            &[arg],
            &[callback],
            &Default::default(),
            &[],
            true,
        );
        assert!(rejected.contextual.borrow().is_empty());
        assert!(rejected.seeded.borrow().is_empty());
    }

    let unproven = SeedLookup::new("Array", &[]);
    seed_patterns_for_callee(
        &unproven,
        &arena,
        &contract,
        &[CallArg::LambdaAt {
            params: vec![Some(parameter)],
        }],
        &[callback],
        &Default::default(),
        &[],
        false,
    );
    assert!(unproven.contextual.borrow().is_empty());
}

#[test]
fn non_contract_ruby_callbacks_still_seed_positional_lambdas() {
    let lookup = SeedLookup::new("Array", &["T"]);
    let arena = TypeArena::new();
    let mut callee = map_method();
    callee.file_path = "app/catalog.rb".into();
    let parameter = crate::types::SourceSpan { start: 24, end: 28 };

    seed_lambda_params(
        &lookup,
        &arena,
        &callee,
        &[CallArg::LambdaAt {
            params: vec![Some(parameter)],
        }],
        array_of(&arena, "Item"),
        None,
        &FxHashMap::default(),
        &[],
    );

    assert_eq!(
        lookup.contextual.borrow().as_slice(),
        &[(parameter, arena.class("Item"))]
    );
}

#[test]
fn an_open_generic_source_addressed_lambda_parameter_records_nothing() {
    let lookup = SeedLookup::new("Array", &["T"]);
    let arena = TypeArena::new();
    let args = vec![CallArg::LambdaAt {
        params: vec![Some(crate::types::SourceSpan { start: 24, end: 28 })],
    }];

    // Bare `Array` leaves the callback parameter as the open generic `T`.
    seed_lambda_params(
        &lookup,
        &arena,
        &map_method(),
        &args,
        arena.class("Array"),
        None,
        &FxHashMap::default(),
        &[],
    );

    assert!(lookup.contextual.borrow().is_empty());
    assert!(lookup.seeded.borrow().is_empty());
}

#[test]
fn an_unnamed_lambda_binding_is_skipped() {
    let lookup = SeedLookup::new("Array", &["T"]);
    let arena = TypeArena::new();
    // A destructuring binding the extractor could not name.
    let args = vec![CallArg::Lambda {
        params: vec![String::new()],
    }];

    seed_lambda_params(
        &lookup,
        &arena,
        &map_method(),
        &args,
        array_of(&arena, "User"),
        None,
        &FxHashMap::default(),
        &[],
    );

    assert!(lookup.seeded.borrow().is_empty());
}

#[test]
fn a_call_with_no_lambda_argument_seeds_nothing() {
    let lookup = SeedLookup::new("Array", &["T"]);
    let arena = TypeArena::new();
    let args = vec![CallArg::Ident("cb".to_string())];

    seed_lambda_params(
        &lookup,
        &arena,
        &map_method(),
        &args,
        array_of(&arena, "User"),
        None,
        &FxHashMap::default(),
        &[],
    );

    assert!(lookup.seeded.borrow().is_empty());
}

#[test]
fn an_argument_driven_binding_reaches_the_callback_parameter() {
    let lookup = SeedLookup::new("Array", &["T"]);
    let arena = TypeArena::new();
    let args = vec![CallArg::Lambda {
        params: vec!["x".to_string()],
    }];
    // The receiver left `T` open; a sibling argument pinned it.
    let mut arg_env: FxHashMap<String, TypeId> = FxHashMap::default();
    arg_env.insert("T".to_string(), arena.class("Account"));

    seed_lambda_params(
        &lookup,
        &arena,
        &map_method(),
        &args,
        arena.class("Array"),
        None,
        &arg_env,
        &[],
    );

    let seeded = lookup.seeded.borrow();
    assert_eq!(arena.get(seeded[0].1), Type::Class("Account".to_string()));
}

#[test]
fn a_compatible_argument_binding_preserves_the_receivers_callback_type() {
    let lookup = SeedLookup::new("Array", &["T"]);
    let arena = TypeArena::new();
    let args = vec![
        CallArg::Ident("user".to_string()),
        CallArg::Lambda {
            params: vec!["x".to_string()],
        },
    ];
    let mut arg_env = FxHashMap::default();
    arg_env.insert("T".to_string(), arena.class("User"));

    seed_lambda_params(
        &lookup,
        &arena,
        &map_with_value_method(),
        &args,
        array_of(&arena, "User"),
        None,
        &arg_env,
        &[],
    );

    let seeded = lookup.seeded.borrow();
    assert_eq!(seeded.len(), 1);
    assert_eq!(seeded[0].0, "x");
    assert_eq!(arena.get(seeded[0].1), Type::Class("User".to_string()));
}

#[test]
fn an_incompatible_argument_binding_does_not_seed_a_callback_parameter() {
    let lookup = SeedLookup::new("Array", &["T"]);
    let arena = TypeArena::new();
    let args = vec![
        CallArg::Ident("account".to_string()),
        CallArg::Lambda {
            params: vec!["x".to_string()],
        },
    ];
    let mut arg_env = FxHashMap::default();
    arg_env.insert("T".to_string(), arena.class("Account"));

    seed_lambda_params(
        &lookup,
        &arena,
        &map_with_value_method(),
        &args,
        array_of(&arena, "User"),
        None,
        &arg_env,
        &[],
    );

    assert!(lookup.seeded.borrow().is_empty());
}

// --- delegate-wrapper unwrap --------------------------------------------------

/// `void UseSnapshot<TEntity>(this ModelBuilder b, Action<EntityTypeBuilder<TEntity>>? configure = null)`
fn use_snapshot_method() -> Symbol {
    let mut s = crate::indexer::resolve::engine::testkit::sym(
        7,
        "UseSnapshot",
        "EFSnapshotBuilder.UseSnapshot",
        "method",
        "src/EFSnapshotBuilder.cs",
    );
    s.signature = Some(
        "void UseSnapshot<TEntity>(this ModelBuilder builder, Action<EntityTypeBuilder<TEntity>>? configure = null)"
            .to_string(),
    );
    s
}

#[test]
fn a_delegate_wrapped_lambda_seeds_from_the_wrappers_generic_args() {
    use crate::type_checker::profile::language_profile::DelegateShape;
    let lookup = SeedLookup::new("EFSnapshotBuilder.UseSnapshot", &["TEntity"]);
    let arena = TypeArena::new();
    let args = vec![CallArg::Lambda {
        params: vec!["b".to_string()],
    }];
    // Explicit type argument bound: TEntity -> EFAppEntity.
    let mut env = FxHashMap::default();
    env.insert("TEntity".to_string(), arena.class("EFAppEntity"));

    seed_lambda_params(
        &lookup,
        &arena,
        &use_snapshot_method(),
        &args,
        arena.class("ModelBuilder"),
        None,
        &env,
        &[
            ("Action", DelegateShape::AllParams),
            ("Func", DelegateShape::LastIsReturn),
        ],
    );

    let seeded = lookup.seeded.borrow();
    assert_eq!(seeded.len(), 1);
    assert_eq!(seeded[0].0, "b");
    assert_eq!(
        arena.format_type(seeded[0].1),
        "EntityTypeBuilder<EFAppEntity>"
    );
}

#[test]
fn a_nominal_non_delegate_parameter_stays_opaque() {
    let lookup = SeedLookup::new("EFSnapshotBuilder.UseSnapshot", &["TEntity"]);
    let arena = TypeArena::new();
    let args = vec![CallArg::Lambda {
        params: vec!["b".to_string()],
    }];
    // No delegate table — Action is just a nominal type; nothing seeds.
    seed_lambda_params(
        &lookup,
        &arena,
        &use_snapshot_method(),
        &args,
        arena.class("ModelBuilder"),
        None,
        &FxHashMap::default(),
        &[],
    );
    assert!(lookup.seeded.borrow().is_empty());
}

#[test]
fn func_shaped_delegates_drop_the_trailing_return_arg() {
    use crate::type_checker::profile::language_profile::DelegateShape;
    let lookup = SeedLookup::new("Runner.Run", &[]);
    let arena = TypeArena::new();
    let mut callee = crate::indexer::resolve::engine::testkit::sym(
        9,
        "Run",
        "Runner.Run",
        "method",
        "src/Runner.cs",
    );
    callee.signature = Some("void Run(Func<Widget, bool> pred)".to_string());
    let args = vec![CallArg::Lambda {
        params: vec!["w".to_string()],
    }];
    seed_lambda_params(
        &lookup,
        &arena,
        &callee,
        &args,
        arena.class("Runner"),
        None,
        &FxHashMap::default(),
        &[("Func", DelegateShape::LastIsReturn)],
    );
    let seeded = lookup.seeded.borrow();
    assert_eq!(seeded.len(), 1);
    assert_eq!(seeded[0].0, "w");
    assert_eq!(arena.format_type(seeded[0].1), "Widget");
}

#[test]
fn java_functional_interfaces_seed_only_their_callback_input_slots() {
    let lookup = SeedLookup::new("Runner.Use", &[]);
    let arena = TypeArena::new();
    let mut callee = crate::indexer::resolve::engine::testkit::sym(
        10,
        "Use",
        "Runner.Use",
        "method",
        "src/Runner.java",
    );
    callee.signature =
        Some("void Use(java.util.function.BiFunction<Left, Right, Result> callback)".to_string());
    let left = crate::types::SourceSpan { start: 20, end: 24 };
    let right = crate::types::SourceSpan { start: 26, end: 31 };
    let args = vec![CallArg::LambdaAt {
        params: vec![Some(left), Some(right)],
    }];

    seed_lambda_params(
        &lookup,
        &arena,
        &callee,
        &args,
        arena.class("Runner"),
        None,
        &FxHashMap::default(),
        crate::languages::java::JAVA_PROFILE.delegate_wrappers,
    );

    let contextual = lookup.contextual.borrow();
    assert_eq!(contextual.len(), 2);
    assert_eq!(contextual[0].0, left);
    assert_eq!(arena.format_type(contextual[0].1), "Left");
    assert_eq!(contextual[1].0, right);
    assert_eq!(arena.format_type(contextual[1].1), "Right");
}

#[test]
fn java_custom_function_name_is_not_a_jdk_callback_wrapper() {
    let lookup = SeedLookup::new("Runner.Use", &[]);
    let arena = TypeArena::new();
    let mut callee = crate::indexer::resolve::engine::testkit::sym(
        11,
        "Use",
        "Runner.Use",
        "method",
        "src/Runner.java",
    );
    callee.signature = Some("void Use(com.acme.Function<Left, Result> callback)".to_string());
    let args = vec![CallArg::LambdaAt {
        params: vec![Some(crate::types::SourceSpan { start: 20, end: 21 })],
    }];

    seed_lambda_params(
        &lookup,
        &arena,
        &callee,
        &args,
        arena.class("Runner"),
        None,
        &FxHashMap::default(),
        crate::languages::java::JAVA_PROFILE.delegate_wrappers,
    );

    assert!(lookup.contextual.borrow().is_empty());
}
