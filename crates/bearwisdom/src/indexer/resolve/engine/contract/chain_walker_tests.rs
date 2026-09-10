use super::*;

// ---------------------------------------------------------------------------
// Go return signatures
// ---------------------------------------------------------------------------

#[test]
fn go_free_function_return_does_not_skip_its_parameter_group() {
    assert_eq!(
        parse_return_type_from_signature_for_lang(
            "func NewWithClient(c *fasthttp.Client) *Client",
            "go",
        )
        .as_deref(),
        Some("Client"),
    );
    assert_eq!(
        parse_return_type_from_signature_for_lang(
            "func (c *Client) SetRetryConfig(config *RetryConfig) *Client",
            "go",
        )
        .as_deref(),
        Some("Client"),
    );
    assert_eq!(
        parse_return_type_from_signature_for_lang("func NewWithoutReturn(c *Client)", "go"),
        None,
    );
}

#[test]
fn go_function_types_keep_their_sole_parameter_group_while_methods_skip_receivers() {
    assert_eq!(
        parse_param_types_from_signature_for_lang("func(T) R", "go"),
        Some(vec!["T".to_string()]),
        "a function type has no receiver group"
    );
    assert_eq!(
        parse_return_type_from_signature_for_lang("func(T) R", "go").as_deref(),
        Some("R")
    );
    assert_eq!(
        parse_return_type_from_signature_for_lang("func(T) func(U) V", "go").as_deref(),
        Some("func(U) V"),
        "a function-valued result is not a method-name plus receiver"
    );
    assert_eq!(
        parse_param_types_from_signature_for_lang("func (r Receiver) Method(T) R", "go"),
        Some(vec!["T".to_string()]),
        "actual method signatures must still skip their receiver"
    );
}

#[test]
fn dart_prefix_parameters_keep_a_function_typed_callback_intact() {
    assert_eq!(
        parse_param_types_from_signature_for_lang(
            "R transform(A value, R Function(A, B) callback)",
            "dart",
        ),
        Some(vec!["A".to_string(), "R Function(A, B)".to_string()]),
    );
}

#[test]
fn php_prefix_parameters_keep_an_enriched_callback_type_intact() {
    assert_eq!(
        parse_param_types_from_signature_for_lang(
            "function use((Item, Context) -> Result $callback, int $limit): void",
            "php",
        ),
        Some(vec![
            "(Item, Context) -> Result".to_string(),
            "int".to_string(),
        ]),
    );
    assert_eq!(
        parse_param_types_from_signature_for_lang(
            "function use(callable $callback, Closure $fallback): void",
            "php",
        ),
        Some(vec!["callable".to_string(), "Closure".to_string()]),
        "native callable markers remain nominal until a supported PHPDoc contract enriches them",
    );
}

#[test]
fn go_callback_signature_preserves_its_full_function_type() {
    assert_eq!(
        parse_param_types_from_signature_for_lang("func Apply(cb func(a, b T) R) R", "go"),
        Some(vec!["func(a, b T) R".to_string()]),
        "the nested Go callback type must reach TypeArena without losing its parameters"
    );
}

// ---------------------------------------------------------------------------
// parse_top_level_conditional
// ---------------------------------------------------------------------------

#[test]
fn splits_a_conditional_return_into_branches() {
    let (t, f) = parse_top_level_conditional(
        "Required<T>[M] extends Constructable | Procedure ? Mock<Required<T>[M]> : never",
    )
    .unwrap();
    assert_eq!(t, "Mock<Required<T>[M]>");
    assert_eq!(f, "never");
}

#[test]
fn declines_text_without_a_conditional() {
    assert_eq!(parse_top_level_conditional("Promise<User>"), None);
    assert_eq!(parse_top_level_conditional("T"), None);
    assert_eq!(parse_top_level_conditional("A | B"), None);
}

#[test]
fn declines_a_conditional_nested_below_top_level() {
    // The conditional sits inside the generic argument — the outer type is a
    // plain application and must stay whole.
    assert_eq!(
        parse_top_level_conditional("Wrapper<T extends U ? A : B>"),
        None
    );
}

#[test]
fn outermost_pair_wins_over_a_nested_conditional() {
    let (t, f) =
        parse_top_level_conditional("T extends U ? A extends B ? C : D : E").unwrap();
    assert_eq!(t, "A extends B ? C : D");
    assert_eq!(f, "E");
}

#[test]
fn function_typed_branch_does_not_derail_the_scan() {
    // The arrow's `>` must not close an angle bracket; the param list's `:`
    // and `?` sit below top level.
    let (t, f) = parse_top_level_conditional(
        "T extends Procedure ? (...args: any[]) => void : never",
    )
    .unwrap();
    assert_eq!(t, "(...args: any[]) => void");
    assert_eq!(f, "never");
}

#[test]
fn extends_requires_identifier_boundaries() {
    // A type named `Textends` must not read as the keyword.
    assert_eq!(parse_top_level_conditional("Textends U ? A : B"), None);
}

// --- parse_declared_type_from_signature_for_lang: initializer split ---------

#[test]
fn declared_type_keeps_generic_defaults_and_arrow() {
    // The `=` of a generic default (`E = string`, inside `<…>`) and the `=` of
    // the `=>` arrow are part of the annotation — only a depth-0 bare `=`
    // starts an initializer.
    let sig = "const make: <A extends B | undefined = undefined, E = string>(opts?: Opts<A, E>) => Client<E, A>";
    assert_eq!(
        parse_declared_type_from_signature_for_lang(sig, "typescript").as_deref(),
        Some("<A extends B | undefined = undefined, E = string>(opts?: Opts<A, E>) => Client<E, A>")
    );
}

#[test]
fn declared_type_still_strips_a_real_initializer() {
    assert_eq!(
        parse_declared_type_from_signature_for_lang("const api: ApiType = make()", "typescript")
            .as_deref(),
        Some("ApiType")
    );
}

#[test]
fn declared_type_keeps_a_bare_arrow_annotation() {
    assert_eq!(
        parse_declared_type_from_signature_for_lang("const h: (e: Event) => Response", "typescript")
            .as_deref(),
        Some("(e: Event) => Response")
    );
}

// --- extension_receiver_type + this-param alignment --------------------------

#[test]
fn extension_receiver_type_reads_the_this_marked_param() {
    assert_eq!(
        extension_receiver_type(
            "void UseSnapshot<T, TEntity>(this ModelBuilder builder, IJsonSerializer json, Action<EntityTypeBuilder<TEntity>>? configure = null)"
        ).as_deref(),
        Some("ModelBuilder")
    );
    assert_eq!(
        extension_receiver_type(
            "PropertyBuilder<DomainId> AsString(this PropertyBuilder<DomainId> propertyBuilder)"
        ).as_deref(),
        Some("PropertyBuilder<DomainId>")
    );
    assert_eq!(extension_receiver_type("void Configure(ModelBuilder builder)"), None);
}

#[test]
fn extension_signature_params_drop_the_receiver_and_align_with_args() {
    // The `this` receiver is not a call argument — pattern positions must
    // match the actual argument list.
    let params = parse_param_types_from_signature(
        "void UseSnapshot<T>(this ModelBuilder builder, IJsonSerializer json, string? col, Action<EntityTypeBuilder<T>>? configure = null)",
    )
    .unwrap();
    assert_eq!(
        params,
        vec!["IJsonSerializer", "string?", "Action<EntityTypeBuilder<T>>?"]
    );
}
