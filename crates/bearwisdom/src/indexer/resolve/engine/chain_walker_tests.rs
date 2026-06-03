use super::{merge_where_bounds, parse_generic_param_clause, parse_return_type_from_signature, parse_return_type_positional, parse_type_head_and_args};

#[test]
fn names_only_when_unbounded() {
    let parsed = parse_generic_param_clause("K, V");
    assert_eq!(
        parsed,
        vec![("K".to_string(), None), ("V".to_string(), None)]
    );
}

#[test]
fn ts_extends_bound() {
    let parsed = parse_generic_param_clause("T extends Animal");
    assert_eq!(parsed, vec![("T".to_string(), Some("Animal".to_string()))]);
}

#[test]
fn colon_bound() {
    let parsed = parse_generic_param_clause("T: Animal");
    assert_eq!(parsed, vec![("T".to_string(), Some("Animal".to_string()))]);
}

#[test]
fn mixed_bounded_and_unbounded() {
    let parsed = parse_generic_param_clause("T extends Animal, U");
    assert_eq!(
        parsed,
        vec![
            ("T".to_string(), Some("Animal".to_string())),
            ("U".to_string(), None),
        ]
    );
}

#[test]
fn higher_kinded_marker_has_no_bound() {
    // Scala `F[_]` — the bracketed shape is not a bound.
    let parsed = parse_generic_param_clause("F[_]");
    assert_eq!(parsed, vec![("F".to_string(), None)]);
}

#[test]
fn rust_multibound_keeps_first() {
    let parsed = parse_generic_param_clause("T: Clone + Send");
    assert_eq!(parsed, vec![("T".to_string(), Some("Clone".to_string()))]);
}

#[test]
fn default_value_dropped_from_bound() {
    let parsed = parse_generic_param_clause("T extends Animal = Dog");
    assert_eq!(parsed, vec![("T".to_string(), Some("Animal".to_string()))]);
}

#[test]
fn scala_upper_bound_is_caught_via_colon() {
    // Scala `[T <: Animal]` — the `:` in `<:` already triggers the bound
    // branch, and the name split on `<` keeps the name clean.
    let parsed = parse_generic_param_clause("T <: Animal");
    assert_eq!(parsed, vec![("T".to_string(), Some("Animal".to_string()))]);
}

#[test]
fn generic_bound_preserved() {
    let parsed = parse_generic_param_clause("T extends Repository<User>");
    assert_eq!(
        parsed,
        vec![("T".to_string(), Some("Repository<User>".to_string()))]
    );
}

#[test]
fn go_space_separated_constraint() {
    // Go `[T Ordered]` — the constraint is a space-separated second token.
    let parsed = parse_generic_param_clause("T Ordered");
    assert_eq!(parsed, vec![("T".to_string(), Some("Ordered".to_string()))]);
}

#[test]
fn go_mixed_constrained_and_bare() {
    let parsed = parse_generic_param_clause("S, T Stringer");
    assert_eq!(
        parsed,
        vec![
            ("S".to_string(), None),
            ("T".to_string(), Some("Stringer".to_string())),
        ]
    );
}

#[test]
fn ts_default_without_extends_has_no_bound() {
    // `<T = string>` — the `=` default is not a bound.
    let parsed = parse_generic_param_clause("T = string");
    assert_eq!(parsed, vec![("T".to_string(), None)]);
}

#[test]
fn declaration_variance_keyword_is_not_the_name() {
    // C#/Kotlin `<out T>` / `<in T>` — the variance keyword is dropped and the
    // parameter carries no bound.
    assert_eq!(parse_generic_param_clause("out T"), vec![("T".to_string(), None)]);
    assert_eq!(parse_generic_param_clause("in T"), vec![("T".to_string(), None)]);
}

#[test]
fn where_clause_fills_unbounded_param() {
    // C#: `class Box<T> where T : IComparable` — `<T>` has no inline bound.
    let mut params = parse_generic_param_clause("T");
    merge_where_bounds(&mut params, "class Box<T> where T : IComparable");
    assert_eq!(params, vec![("T".to_string(), Some("IComparable".to_string()))]);
}

#[test]
fn where_clause_skips_special_constraints() {
    // C#: `where T : class, IFoo, new()` — keyword constraints aren't types;
    // the first nominal interface wins.
    let mut params = parse_generic_param_clause("T");
    merge_where_bounds(&mut params, "void M<T>() where T : class, IFoo, new()");
    assert_eq!(params, vec![("T".to_string(), Some("IFoo".to_string()))]);
}

#[test]
fn where_clause_per_param_multiple_clauses() {
    // C# uses one `where` per parameter.
    let mut params = parse_generic_param_clause("T, U");
    merge_where_bounds(&mut params, "void M<T, U>() where T : IA where U : IB");
    assert_eq!(
        params,
        vec![
            ("T".to_string(), Some("IA".to_string())),
            ("U".to_string(), Some("IB".to_string())),
        ]
    );
}

#[test]
fn rust_single_line_where_clause() {
    // Rust `where T: Clone + Send, U: Debug` — first nominal bound per param.
    let mut params = parse_generic_param_clause("T, U");
    merge_where_bounds(&mut params, "fn f<T, U>(x: T, y: U) where T: Clone + Send, U: Debug");
    assert_eq!(
        params,
        vec![
            ("T".to_string(), Some("Clone".to_string())),
            ("U".to_string(), Some("Debug".to_string())),
        ]
    );
}

#[test]
fn inline_bound_wins_over_where() {
    // An inline bound is the more local declaration; `where` does not override.
    let mut params = parse_generic_param_clause("T extends Animal");
    merge_where_bounds(&mut params, "class C<T extends Animal> where T : Other");
    assert_eq!(params, vec![("T".to_string(), Some("Animal".to_string()))]);
}

#[test]
fn no_where_clause_leaves_params_untouched() {
    let mut params = parse_generic_param_clause("T");
    merge_where_bounds(&mut params, "class Box<T>");
    assert_eq!(params, vec![("T".to_string(), None)]);
}

// --- parse_return_type_from_signature: colon form (regression) ---

#[test]
fn return_type_colon_form() {
    assert_eq!(
        parse_return_type_from_signature("findUnique(args): Prisma.User"),
        Some("Prisma.User".to_string())
    );
}

#[test]
fn return_type_colon_form_with_generics() {
    assert_eq!(
        parse_return_type_from_signature("get(): Promise<User>"),
        Some("Promise<User>".to_string())
    );
}

#[test]
fn return_type_arrow_in_params_does_not_shadow_colon() {
    // The `=> void` lives inside the parameter list; the real return is `Ret`.
    assert_eq!(
        parse_return_type_from_signature("on(cb: () => void): Ret"),
        Some("Ret".to_string())
    );
}

// --- parse_return_type_from_signature: arrow forms (new) ---

#[test]
fn return_type_python_arrow() {
    assert_eq!(
        parse_return_type_from_signature("find_one(self, id) -> User"),
        Some("User".to_string())
    );
}

#[test]
fn return_type_python_arrow_trailing_colon() {
    assert_eq!(
        parse_return_type_from_signature("query(self) -> Dict[str, int]:"),
        Some("Dict[str, int]".to_string())
    );
}

#[test]
fn return_type_ts_fat_arrow() {
    assert_eq!(
        parse_return_type_from_signature("(x: number) => string"),
        Some("string".to_string())
    );
}

#[test]
fn return_type_rust_arrow_with_generics() {
    assert_eq!(
        parse_return_type_from_signature("fn find(&self) -> Result<T, E>"),
        Some("Result<T, E>".to_string())
    );
}

#[test]
fn return_type_rust_arrow_strips_where_clause() {
    assert_eq!(
        parse_return_type_from_signature("fn f(&self) -> Foo where T: Clone"),
        Some("Foo".to_string())
    );
}

#[test]
fn return_type_rust_arrow_strips_block() {
    assert_eq!(
        parse_return_type_from_signature("fn f() -> Foo {"),
        Some("Foo".to_string())
    );
}

#[test]
fn return_type_none_when_no_return() {
    assert_eq!(parse_return_type_from_signature("void Foo()"), None);
    assert_eq!(parse_return_type_from_signature(""), None);
}

// --- parse_return_type_positional: leading-form (Java/C#) ---

#[test]
fn positional_java_leading_generic() {
    // `{ret} {name}{params}` — the first depth-0 token is the return type.
    assert_eq!(
        parse_return_type_positional("List<User> getItems()"),
        Some("List<User>".to_string())
    );
}

#[test]
fn positional_java_leading_nongeneric() {
    assert_eq!(
        parse_return_type_positional("String getName()"),
        Some("String".to_string())
    );
}

#[test]
fn positional_generic_with_spaced_args() {
    // The space inside `<>` is depth-1, so the whole type stays one token.
    assert_eq!(
        parse_return_type_positional("Map<String, Integer> getMap()"),
        Some("Map<String, Integer>".to_string())
    );
}

#[test]
fn positional_java_generic_method_returns_param() {
    // `{ret} {type_params}{name}{params}` — ret precedes the method's own `<T>`.
    assert_eq!(
        parse_return_type_positional("T <T>get(int i)"),
        Some("T".to_string())
    );
}

#[test]
fn positional_csharp_generic_method_with_constraints() {
    assert_eq!(
        parse_return_type_positional("Task<int> GetAsync<T>() where T : class"),
        Some("Task<int>".to_string())
    );
}

#[test]
fn positional_rejects_go_func_keyword() {
    // Go's trailing-return form leads with `func` — must not be read as a type.
    assert_eq!(parse_return_type_positional("func (s *S) F(a A) Ret"), None);
}

#[test]
fn positional_rejects_modifier_prefix() {
    // A builder that prefixes a modifier fails safe (over-rejection is harmless).
    assert_eq!(parse_return_type_positional("static int foo()"), None);
}

#[test]
fn positional_rejects_no_return_token() {
    // First token opens the param list → the signature carries no return type.
    assert_eq!(parse_return_type_positional("getItems()"), None);
    assert_eq!(parse_return_type_positional(""), None);
}

#[test]
fn positional_rejects_void() {
    // `void` is no return value — a void setter must carry no return type.
    assert_eq!(parse_return_type_positional("void setName(String n)"), None);
}

#[test]
fn parse_type_head_plain() {
    let (head, args) = parse_type_head_and_args("User");
    assert_eq!(head, "User");
    assert!(args.is_empty());
}

#[test]
fn parse_type_head_single_arg() {
    let (head, args) = parse_type_head_and_args("List<User>");
    assert_eq!(head, "List");
    assert_eq!(args, vec!["User"]);
}

#[test]
fn parse_type_head_two_args() {
    let (head, args) = parse_type_head_and_args("Map<String, User>");
    assert_eq!(head, "Map");
    assert_eq!(args, vec!["String", "User"]);
}

#[test]
fn parse_type_head_nested_arg_flattened() {
    // Nested generics: inner args are not recursed into.
    let (head, args) = parse_type_head_and_args("Map<String, List<User>>");
    assert_eq!(head, "Map");
    // The second arg is the head of `List<User>` — just `List`.
    assert_eq!(args, vec!["String", "List"]);
}

#[test]
fn parse_type_head_unclosed_angle() {
    // Malformed input — no crash, head extracted, args empty.
    let (head, args) = parse_type_head_and_args("List<User");
    assert_eq!(head, "List");
    assert!(args.is_empty());
}
