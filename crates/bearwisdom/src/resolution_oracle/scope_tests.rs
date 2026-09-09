use super::fixture_support::{run_selectors as run, Fixture};
use super::*;

const CLASSES: &str = "class Alpha {\n/*@decl:1:method*/save(): void {}\n}\nclass Beta {\n/*@decl:2:method*/save(): void {}\n}\n";

#[test]
fn type_only_alias_cannot_become_a_runtime_static_receiver() {
    check("class Holder { /*@decl:3:method*/static save(): void {} }\ntype Alias = Holder;\nfunction f() { Alias./*@ref:11*/save(); }\nfunction g() { const Alias = Holder; Alias./*@ref:12*/save(); }\n", &[(11, None), (12, Some(3))]);
}

fn check(body: &str, labels: &[(u32, Option<u32>)]) -> OracleReport {
    let source = format!("{CLASSES}{body}");
    let result = run(
        &[Fixture {
            path: "scopes.ts",
            language: "typescript",
            marked_source: &source,
        }],
        labels,
    )
    .unwrap();
    println!("{}", serde_json::to_string(&result).unwrap());
    assert_eq!(result.counts.incorrect, 0, "{result:#?}");
    assert_eq!(result.counts.not_extracted, 0, "{result:#?}");
    assert_eq!(result.counts.missing_resolution, 0, "{result:#?}");
    for reference in &result.references {
        assert_eq!(
            reference.verdict,
            if reference.expected.target.is_some() {
                Verdict::Correct
            } else {
                Verdict::CorrectUnbound
            },
            "{reference:#?}"
        );
    }
    result
}

#[test]
fn same_line_sibling_parameter_scopes_do_not_merge() {
    check("function first(value: Alpha) { value./*@ref:11*/save(); } function second(value: Beta) { value./*@ref:12*/save(); }\n",
        &[(11, Some(1)), (12, Some(2))]);
}

#[test]
fn block_shadowing_restores_outer_binding_after_the_block() {
    check("function f(value: Alpha) {\nvalue./*@ref:11*/save();\n{ let value: Beta; value./*@ref:12*/save(); }\nvalue./*@ref:13*/save();\n}\n",
        &[(11, Some(1)), (12, Some(2)), (13, Some(1))]);
}

#[test]
fn annotated_callback_parameter_does_not_overwrite_captured_outer_parameter() {
    check("function f(value: Alpha) {\nconst cb = (value: Beta) => { value./*@ref:11*/save(); };\nconst capture = () => { value./*@ref:12*/save(); };\nvalue./*@ref:13*/save();\n}\n",
        &[(11, Some(2)), (12, Some(1)), (13, Some(1))]);
}

#[test]
fn untyped_parameter_cannot_borrow_a_siblings_annotation() {
    check("function first(value: Alpha) { value./*@ref:11*/save(); }\nfunction second(value) { value./*@ref:12*/save(); }\n",
        &[(11, Some(1)), (12, None)]);
}

#[test]
fn before_initialization_cannot_fall_back_to_outer_parameter() {
    check("function f(value: Alpha) {\n{ value./*@ref:11*/save(); let value: Beta; value./*@ref:12*/save(); }\nvalue./*@ref:13*/save();\n}\n",
        &[(11, Some(2)), (12, Some(2)), (13, Some(1))]);
}

#[test]
fn rhs_inference_and_nested_function_writes_stay_with_their_binding() {
    check("function makeA(): Alpha { return new Alpha(); }\nfunction makeB(): Beta { return new Beta(); }\nfunction f() {\nlet value = makeA();\nvalue./*@ref:11*/save();\nconst cb = () => { value = makeB(); value./*@ref:12*/save(); };\nvalue./*@ref:13*/save();\n}\n",
        &[(11, Some(1)), (12, Some(1)), (13, Some(1))]);
}

#[test]
fn bare_call_context_types_only_its_callback_binding() {
    check("function consume(callback: (value: Alpha) => void): void {}\nfunction f(value: Beta) { consume(value => { value./*@ref:11*/save(); }); value./*@ref:12*/save(); }\n",
        &[(11, Some(1)), (12, Some(2))]);
}

#[test]
fn generic_member_callbacks_with_identical_names_keep_distinct_types() {
    check("class Channel<T> { each(callback: (value: T) => void): void {} }\nfunction f(a: Channel<Alpha>, b: Channel<Beta>) { a.each(value => { value./*@ref:11*/save(); }); b.each(value => { value./*@ref:12*/save(); }); }\n",
        &[(11, Some(1)), (12, Some(2))]);
}

#[test]
fn nested_contextual_callbacks_cannot_overwrite_the_outer_callback() {
    check("function consumeA(callback: (value: Alpha) => void): void {}\nfunction consumeB(callback: (value: Beta) => void): void {}\nfunction f() { consumeA(value => { consumeB(value => { value./*@ref:11*/save(); }); value./*@ref:12*/save(); }); }\n",
        &[(11, Some(2)), (12, Some(1))]);
}

#[test]
fn explicit_callback_annotation_precedes_contextual_type() {
    check("function consume(callback: (value: Alpha) => void): void {}\nfunction f() { consume((value: Beta) => { value./*@ref:11*/save(); }); }\n",
        &[(11, Some(2))]);
}

#[test]
fn nested_calls_sharing_a_start_byte_have_distinct_callback_declarations() {
    check("class End { finish(callback: (value: Beta) => void): void {} }\nclass Start { begin(callback: (value: Alpha) => void): End { return new End(); } }\nfunction f(start: Start) { start.begin(value => { value./*@ref:11*/save(); }).finish(value => { value./*@ref:12*/save(); }); }\n",
        &[(11, Some(1)), (12, Some(2))]);
}

#[test]
fn repeated_callee_spelling_does_not_collapse_nested_call_occurrences() {
    check("class End { /*@decl:3:method*/go(callback: (value: Beta) => void): void {} }\nclass Start { /*@decl:4:method*/go(callback: (value: Alpha) => void): End { return new End(); } }\nfunction f(start: Start) { start./*@ref:13*/go(value => { value./*@ref:11*/save(); })./*@ref:14*/go(value => { value./*@ref:12*/save(); }); }\n",
        &[(11, Some(1)), (12, Some(2)), (13, Some(4)), (14, Some(3))]);
}

#[test]
fn bare_parameter_calls_keep_same_line_sibling_declaration_identity() {
    check("function callback(): void {}\nfunction first(/*@decl:3:parameter*/callback: () => void) { /*@ref:11*/callback(); } function second(/*@decl:4:parameter*/callback: () => void) { /*@ref:12*/callback(); }\n",
        &[(11, Some(3)), (12, Some(4))]);
}

#[test]
fn bare_parameter_calls_inside_closures_keep_the_captured_declaration() {
    check("function callback(): void {}\nfunction f(/*@decl:3:parameter*/callback: () => void) { const nested = () => { /*@ref:11*/callback(); }; /*@ref:12*/callback(); }\n",
        &[(11, Some(3)), (12, Some(3))]);
}

#[test]
fn bare_untyped_parameter_does_not_bind_a_callable_namesake() {
    check("function callback(): void {}\nfunction f(/*@decl:3:parameter*/callback) { /*@ref:11*/callback(); }\n",
        &[(11, Some(3))]);
}

#[test]
fn bare_block_local_identity_survives_tdz_and_scope_exit() {
    check("function f(/*@decl:3:parameter*/callback: () => void) { { /*@ref:11*/callback(); let /*@decl:4:variable*/callback: () => void; /*@ref:12*/callback(); } /*@ref:13*/callback(); }\n",
        &[(11, Some(4)), (12, Some(4)), (13, Some(3))]);
}

#[test]
fn bare_callable_parameter_return_types_seed_downstream_receivers() {
    check("function make(): Beta { return new Beta(); }\nfunction f(/*@decl:3:parameter*/make: () => Alpha) { const value = /*@ref:11*/make(); value./*@ref:12*/save(); }\n",
        &[(11, Some(3)), (12, Some(1))]);
}

#[test]
fn destructured_call_reference_binds_the_local_not_its_runtime_implementation() {
    check("function makeLogger() { return { info(message: string): void {} }; }\nfunction f() { const { /*@decl:3:variable*/info } = makeLogger(); /*@ref:11*/info(\"hello\"); }\n",
        &[(11, Some(3))]);
}

#[test]
fn block_function_shadows_parameter_and_preserves_its_return() {
    check("function f(/*@decl:3:parameter*/callback: () => Alpha) { { /*@decl:4:function*/function callback(): Beta { return new Beta(); } const value = /*@ref:11*/callback(); value./*@ref:12*/save(); callback()./*@ref:13*/save(); } /*@ref:14*/callback(); }\n",
        &[(11, Some(4)), (12, Some(2)), (13, Some(2)), (14, Some(3))]);
}

#[test]
fn sibling_local_classes_keep_distinct_constructor_and_member_identity() {
    check("function first() { class Model { /*@decl:3:method*/save(): void {} } const value = new Model(); value./*@ref:11*/save(); }\nfunction second() { class Model { /*@decl:4:method*/save(): void {} } const value = new Model(); value./*@ref:12*/save(); }\n",
        &[(11, Some(3)), (12, Some(4))]);
}

#[test]
fn hoisted_local_function_return_binds_before_its_declaration() {
    check("function make(): Beta { return new Beta(); }\nfunction f() { const value = /*@ref:11*/make(); value./*@ref:12*/save(); /*@decl:3:function*/function make(): Alpha { return new Alpha(); } }\n",
        &[(11, Some(3)), (12, Some(1))]);
}

#[test]
fn local_class_static_member_does_not_bind_a_siblings_class() {
    check("function first() { class Model { /*@decl:3:method*/static save(): void {} } Model./*@ref:11*/save(); }\nfunction second() { class Model { /*@decl:4:method*/static save(): void {} } Model./*@ref:12*/save(); }\n",
        &[(11, Some(3)), (12, Some(4))]);
}

#[test]
fn callable_parameter_chain_root_yields_its_return_type() {
    check("function make(): Beta { return new Beta(); }\nfunction f(make: () => Alpha) { make()./*@ref:11*/save(); }\n",
        &[(11, Some(1))]);
}

#[test]
fn lexical_value_cannot_fall_through_to_a_same_named_namespace() {
    check("namespace value { export function save(): void {} }\nfunction f(value) { value./*@ref:11*/save(); }\n",
        &[(11, None)]);
}

#[test]
fn constructed_lexical_class_keeps_generic_callback_arguments() {
    check("class Channel<T> { each(callback: (value: T) => void): void {} }\nfunction f() { const channel = new Channel<Alpha>(); channel.each(value => { value./*@ref:11*/save(); }); new Channel<Beta>().each(value => { value./*@ref:12*/save(); }); }\n",
        &[(11, Some(1)), (12, Some(2))]);
}

#[test]
fn bare_lexical_function_keeps_explicit_return_type_arguments() {
    check("function make<T>(): T { throw 0; }\nfunction f() { const value = make<Alpha>(); value./*@ref:11*/save(); make<Beta>()./*@ref:12*/save(); }\n",
        &[(11, Some(1)), (12, Some(2))]);
}

#[test]
fn same_named_wrappers_write_returns_to_their_own_declaration_ids() {
    check("function first() { function make(): Alpha { return new Alpha(); } function wrap() { return make(); } wrap()./*@ref:11*/save(); }\nfunction second() { function make(): Beta { return new Beta(); } function wrap() { return make(); } wrap()./*@ref:12*/save(); }\n",
        &[(11, Some(1)), (12, Some(2))]);
}

#[test]
fn named_function_expression_has_a_private_recursive_binding_and_return() {
    check("function f(/*@decl:4:parameter*/self: () => Beta) { const run = /*@decl:3:function*/function self(): Alpha { /*@ref:11*/self(); self()./*@ref:12*/save(); return new Alpha(); }; /*@ref:13*/self(); }\n",
        &[(11, Some(3)), (12, Some(1)), (13, Some(4))]);
}

#[test]
fn named_function_expression_parameters_shadow_the_private_name() {
    check("function f() { const run = function self(/*@decl:3:parameter*/self: () => Beta): Alpha { /*@ref:11*/self(); self()./*@ref:12*/save(); return new Alpha(); }; }\n",
        &[(11, Some(3)), (12, Some(2))]);
}

#[test]
fn named_function_expression_name_is_not_visible_after_the_expression() {
    check("function f() { const run = function hidden(): Alpha { return new Alpha(); }; /*@ref:11*/hidden(); }\n",
        &[(11, None)]);
}

#[test]
fn function_expression_variable_keeps_its_declaration_and_callable_return() {
    check("function f() { const /*@decl:3:variable*/run = function self(): Alpha { return new Alpha(); }; /*@ref:11*/run()./*@ref:12*/save(); const value = run(); value./*@ref:13*/save(); }\n",
        &[(11, Some(3)), (12, Some(1)), (13, Some(1))]);
}

#[test]
fn named_class_expression_self_reference_does_not_escape_its_scope() {
    check("function f() { const C = class Hidden { /*@decl:3:method*/static save(): void {} method(): void { Hidden./*@ref:11*/save(); } }; Hidden./*@ref:12*/save(); }\n",
        &[(11, Some(3)), (12, None)]);
}

#[test]
fn named_class_expression_constructor_value_keeps_member_identity() {
    check("function f() { const C = class Model { /*@decl:3:method*/save(): void {} method(): void { new Model()./*@ref:11*/save(); } }; const value = new C(); value./*@ref:12*/save(); new C()./*@ref:13*/save(); }\n",
        &[(11, Some(3)), (12, Some(3)), (13, Some(3))]);
}

#[test]
fn named_function_expression_variable_keeps_explicit_generic_returns() {
    check("function f() { const run = function self<T>(): T { throw 0; }; const value = run<Alpha>(); value./*@ref:11*/save(); run<Beta>()./*@ref:12*/save(); }\n",
        &[(11, Some(1)), (12, Some(2))]);
}

#[test]
fn named_class_expression_variable_keeps_constructor_type_arguments() {
    check("function f() { const C = class Channel<T> { each(callback: (value: T) => void): void {} }; const channel = new C<Alpha>(); channel.each(value => { value./*@ref:11*/save(); }); new C<Beta>().each(value => { value./*@ref:12*/save(); }); }\n",
        &[(11, Some(1)), (12, Some(2))]);
}

#[test]
fn lexical_type_annotations_keep_sibling_local_class_identity() {
    check("function first() { class Model { /*@decl:3:method*/save(): void {} } let value: Model; value./*@ref:11*/save(); }\nfunction second() { class Model { /*@decl:4:method*/save(): void {} } let value: Model; value./*@ref:12*/save(); }\n",
        &[(11, Some(3)), (12, Some(4))]);
}

#[test]
fn lexical_type_arguments_keep_sibling_local_class_identity() {
    check("function make<T>(): T { throw 0; }\nfunction first() { class Model { /*@decl:3:method*/save(): void {} } const value = make<Model>(); value./*@ref:11*/save(); }\nfunction second() { class Model { /*@decl:4:method*/save(): void {} } make<Model>()./*@ref:12*/save(); }\n",
        &[(11, Some(3)), (12, Some(4))]);
}

#[test]
fn lexical_type_namespace_is_not_shadowed_by_a_value_only_declaration() {
    check("class Model { /*@decl:3:method*/save(): void {} }\nfunction f() { const Model = 0; let value: Model; value./*@ref:11*/save(); }\n",
        &[(11, Some(3))]);
}

#[test]
fn lexical_type_annotations_inside_private_classes_keep_self_identity() {
    check("class Model { /*@decl:3:method*/save(): void {} }\nfunction f() { const C = class Model { /*@decl:4:method*/save(): void {} test(value: Model): void { value./*@ref:11*/save(); } }; }\n",
        &[(11, Some(4))]);
}

#[test]
fn lexical_type_return_annotations_keep_the_declaration_environment() {
    check("function first() { class Model { /*@decl:3:method*/save(): void {} } function make(): Model { throw 0; } make()./*@ref:11*/save(); }\nfunction second() { class Model { /*@decl:4:method*/save(): void {} } function make(): Model { throw 0; } const value = make(); value./*@ref:12*/save(); }\n",
        &[(11, Some(3)), (12, Some(4))]);
}

#[test]
fn lexical_type_fields_and_method_returns_preserve_local_type_heads() {
    check("function first() { class Model { /*@decl:3:method*/save(): void {} } class Box { item: Model; get(): Model { throw 0; } } let box: Box; box.item./*@ref:11*/save(); box.get()./*@ref:12*/save(); }\nfunction second() { class Model { /*@decl:4:method*/save(): void {} } class Box { item: Model; get(): Model { throw 0; } } let box: Box; box.item./*@ref:13*/save(); box.get()./*@ref:14*/save(); }\n",
        &[(11, Some(3)), (12, Some(3)), (13, Some(4)), (14, Some(4))]);
}

#[test]
fn lexical_type_aliases_and_interfaces_preserve_sibling_identity() {
    check("function first() { interface Model { /*@decl:3:method*/save(): void; } type View = Model; let value: View; value./*@ref:11*/save(); }\nfunction second() { interface Model { /*@decl:4:method*/save(): void; } type View = Model; let value: View; value./*@ref:12*/save(); }\n",
        &[(11, Some(3)), (12, Some(4))]);
}

#[test]
fn lexical_type_parameter_does_not_borrow_an_outer_nominal_namesake() {
    check(
        "class T { save(): void {} }\nfunction f<T>(value: T) { value./*@ref:11*/save(); }\n",
        &[(11, None)],
    );
}

#[test]
fn lexical_type_composites_keep_callable_return_and_applied_argument_heads() {
    check("class Channel<T> { each(callback: (value: T) => void): void {} }\nfunction f() { class Model { /*@decl:3:method*/save(): void {} } let make: () => Model; make()./*@ref:11*/save(); let channel: Channel<Model>; channel.each(value => value./*@ref:12*/save()); }\n",
        &[(11, Some(3)), (12, Some(3))]);
}

#[test]
fn lexical_type_generic_aliases_substitute_ids_through_alias_chains() {
    check("function first() { class Model { /*@decl:3:method*/save(): void {} } type Identity<T> = T; type View = Identity<Model>; let value: View; value./*@ref:11*/save(); }\nfunction second() { class Model { /*@decl:4:method*/save(): void {} } type Identity<T> = T; let value: Identity<Model>; value./*@ref:12*/save(); }\n",
        &[(11, Some(3)), (12, Some(4))]);
}

#[test]
fn bound_signature_member_arguments_keep_sibling_local_type_identity() {
    check("class Factory { make<T>(): T { throw 0; } }\nfunction first(factory: Factory) { class Model { /*@decl:3:method*/save(): void {} } const value = factory.make<Model>(); value./*@ref:11*/save(); }\nfunction second(factory: Factory) { class Model { /*@decl:4:method*/save(): void {} } factory.make<Model>()./*@ref:12*/save(); }\n",
        &[(11, Some(3)), (12, Some(4))]);
}

#[test]
fn bound_signature_callbacks_keep_the_callees_local_type_environment() {
    check("function first() { class Model { /*@decl:3:method*/save(): void {} } function consume(callback: (value: Model) => void): void {} consume(value => value./*@ref:11*/save()); }\nfunction second() { class Model { /*@decl:4:method*/save(): void {} } function consume(callback: (value: Model) => void): void {} consume(value => value./*@ref:12*/save()); }\n",
        &[(11, Some(3)), (12, Some(4))]);
}

#[test]
fn bound_signature_explicit_callback_arguments_keep_local_identity() {
    check("class Channel { each<T>(callback: (value: T) => void): void {} }\nfunction f(channel: Channel) { class Model { /*@decl:3:method*/save(): void {} } channel.each<Model>(value => value./*@ref:11*/save()); }\nfunction each<T>(callback: (value: T) => void): void {}\nfunction g() { class Model { /*@decl:4:method*/save(): void {} } each<Model>(value => value./*@ref:12*/save()); }\n",
        &[(11, Some(3)), (12, Some(4))]);
}

#[test]
fn bound_signature_method_parameter_does_not_alias_receiver_parameter() {
    check("class Channel<T> { each<T>(callback: (value: T) => void): void {} }\nfunction f(channel: Channel<Alpha>) { channel.each<Beta>(value => value./*@ref:11*/save()); channel.each(value => value./*@ref:12*/save()); }\n",
        &[(11, Some(2)), (12, None)]);
}

#[test]
fn bound_signature_inferred_arguments_preserve_local_declaration_identity() {
    check("function identity<T>(value: T): T { return value; }\nclass Factory { identity<T>(value: T): T { return value; } }\nfunction f(factory: Factory) { class Model { /*@decl:3:method*/save(): void {} } let model: Model; const value = identity(model); value./*@ref:11*/save(); factory.identity(model)./*@ref:12*/save(); }\n",
        &[(11, Some(3)), (12, Some(3))]);
}

#[test]
fn bound_signature_each_member_hop_keeps_its_own_type_arguments() {
    check("class Factory { make<T>(): T { throw 0; } }\nfunction f(factory: Factory) { class Model { /*@decl:3:method*/save(): void {} } factory.make<Factory>().make<Model>()./*@ref:11*/save(); factory.make<Factory>().make<Beta>()./*@ref:12*/save(); }\n",
        &[(11, Some(3)), (12, Some(2))]);
}

#[test]
fn bound_signature_explicit_arguments_win_over_structurally_assignable_inference() {
    check("class Factory { identity<T>(value: T): T { return value; } }\nfunction identity<T>(value: T): T { return value; }\nfunction f(factory: Factory, beta: Beta) { const value = identity<Alpha>(beta); value./*@ref:11*/save(); factory.identity<Alpha>(beta)./*@ref:12*/save(); }\n",
        &[(11, Some(1)), (12, Some(1))]);
}

#[test]
fn bound_signature_callbacks_on_members_keep_sibling_declaration_environments() {
    check("function first() { class Model { /*@decl:3:method*/save(): void {} } class Channel { each(callback: (value: Model) => void): void {} } let channel: Channel; channel.each(value => value./*@ref:11*/save()); }\nfunction second() { class Model { /*@decl:4:method*/save(): void {} } class Channel { each(callback: (value: Model) => void): void {} } let channel: Channel; channel.each(value => value./*@ref:12*/save()); }\n",
        &[(11, Some(3)), (12, Some(4))]);
}

#[test]
fn bound_signature_inferred_callback_arguments_keep_source_bound_patterns() {
    check("function each<T>(value: T, callback: (item: T) => void): void {}\nfunction f() { class Model { /*@decl:3:method*/save(): void {} } let model: Model; each(model, item => item./*@ref:11*/save()); }\nfunction g() { class Model { /*@decl:4:method*/save(): void {} } let model: Model; each(model, item => item./*@ref:12*/save()); }\n",
        &[(11, Some(3)), (12, Some(4))]);
}

#[test]
fn bound_signature_declaring_type_returns_and_method_returns_keep_distinct_generic_ids() {
    check("class Channel<T> { get(): T { throw 0; } make<T>(): T { throw 0; } }\nfunction f(channel: Channel<Alpha>) { channel.get()./*@ref:11*/save(); channel.make<Beta>()./*@ref:12*/save(); }\n",
        &[(11, Some(1)), (12, Some(2))]);
}

#[test]
fn argument_identity_preserves_local_class_values_through_generic_calls() {
    check("function identity<T>(value: T): T { return value; }\nfunction first() { class Model { /*@decl:3:method*/save(): void {} } const Copy = identity(Model); new Copy()./*@ref:11*/save(); }\nfunction second() { class Model { /*@decl:4:method*/save(): void {} } const Copy = identity(Model); const value = new Copy(); value./*@ref:12*/save(); }\n",
        &[(11, Some(3)), (12, Some(4))]);
}

#[test]
fn argument_identity_preserves_function_value_signatures() {
    check("function invoke<T>(callback: () => T): T { return callback(); }\nfunction first() { class Model { /*@decl:3:method*/save(): void {} } function make(): Model { throw 0; } const value = invoke(make); value./*@ref:11*/save(); }\nfunction second() { class Model { /*@decl:4:method*/save(): void {} } function make(): Model { throw 0; } invoke(make)./*@ref:12*/save(); }\n",
        &[(11, Some(3)), (12, Some(4))]);
}

#[test]
fn argument_identity_named_function_expression_values_keep_private_provenance() {
    check("function invoke<T>(callback: () => T): T { return callback(); }\nfunction f() { const make = function privateName(): Alpha { throw 0; }; invoke(make)./*@ref:11*/save(); }\n",
        &[(11, Some(1))]);
}

#[test]
fn argument_identity_parenthesized_and_awaited_reads_keep_their_source_binding() {
    check("function identity<T>(value: T): T { return value; }\nasync function f(alpha: Alpha, beta: Beta) { identity((alpha))./*@ref:11*/save(); identity(await beta)./*@ref:12*/save(); }\n",
        &[(11, Some(1)), (12, Some(2))]);
}

#[test]
fn argument_identity_recursive_array_and_ternary_leaves_keep_binding_ids() {
    check("function head<T>(values: T[]): T { throw 0; }\nfunction identity<T>(value: T): T { return value; }\nfunction f(alpha: Alpha, beta: Beta, flag: boolean) { head([/* argument comment */ alpha])./*@ref:11*/save(); identity(flag ? beta : beta)./*@ref:12*/save(); }\n",
        &[(11, Some(1)), (12, Some(2))]);
}

#[test]
fn argument_identity_callback_reads_keep_sibling_contexts() {
    check("function each<T>(callback: (value: T) => void): void {}\nfunction identity<T>(value: T): T { return value; }\nfunction f() { each<Alpha>(value => identity(value)./*@ref:11*/save()); each<Beta>(value => identity(value)./*@ref:12*/save()); }\n",
        &[(11, Some(1)), (12, Some(2))]);
}

#[test]
fn argument_identity_missing_local_types_do_not_borrow_a_namesake() {
    check("function identity<T>(value: T): T { return value; }\nlet value: Alpha;\nfunction f(value) { identity(value)./*@ref:11*/save(); }\nfunction g(value: Beta) { identity(value)./*@ref:12*/save(); }\n",
        &[(11, None), (12, Some(2))]);
}

#[test]
fn argument_identity_member_call_arguments_preserve_function_signatures() {
    check("class Factory { invoke<T>(callback: () => T): T { return callback(); } }\nfunction f(factory: Factory) { function make(): Alpha { throw 0; } factory.invoke(make)./*@ref:11*/save(); const value = factory.invoke(make); value./*@ref:12*/save(); }\n",
        &[(11, Some(1)), (12, Some(1))]);
}

#[test]
fn argument_identity_field_initializer_pass_uses_the_files_bound_arguments() {
    check("function identity<T>(value: T): T { return value; }\nlet alpha: Alpha;\nclass Holder { item = identity(alpha); }\nfunction f(holder: Holder) { holder.item./*@ref:11*/save(); }\n",
        &[(11, Some(1))]);
}

#[test]
fn argument_identity_chain_initializer_pass_uses_bound_argument_reads() {
    check("class Factory { identity<T>(value: T): T { return value; } }\nlet factory: Factory;\nlet beta: Beta;\nclass Holder { item = factory.identity(beta); }\nfunction f(holder: Holder) { holder.item./*@ref:11*/save(); }\n",
        &[(11, Some(2))]);
}

#[test]
fn argument_identity_nested_calls_are_not_the_fields_initializer_value() {
    check("function identity<T>(value: T): T { return value; }\nclass Factory { identity<T>(value: T): T { return value; } }\nlet alpha: Alpha; let factory: Factory; let beta: Beta;\nclass Holder { values = [identity(alpha)]; callback = () => factory.identity(beta); }\nfunction f(holder: Holder) { holder.values./*@ref:11*/save(); holder.callback./*@ref:12*/save(); }\n",
        &[(11, None), (12, None)]);
}
