use super::signature_generic_params;

fn names(sig: &str, name: &str) -> Vec<String> {
    signature_generic_params(sig, name)
        .into_iter()
        .map(|(n, _, _)| n)
        .collect()
}

#[test]
fn a_generic_return_type_on_a_non_generic_declaration_yields_no_params() {
    // C#-shaped return-type-first signature: the first `<` belongs to the
    // RETURN type, not the declaration. The declaration name (the last
    // occurrence) carries no clause, so the extraction must be empty —
    // a `DomainId` "parameter" here rewrites legitimate type mentions.
    assert!(names("NamedId<DomainId> NamedId(this IAppEntity app)", "NamedId").is_empty());
}

#[test]
fn a_generic_method_clause_is_read_off_the_declaration_name() {
    assert_eq!(
        names("TResult Sync<TSource, T>(Func<TSource, T> resolver)", "Sync"),
        vec!["TSource".to_string(), "T".to_string()],
    );
}

#[test]
fn a_type_declaration_clause_with_a_where_constraint_is_read() {
    assert_eq!(names("class NamedId<T> where T : notnull", "NamedId"), vec!["T".to_string()]);
}

#[test]
fn a_single_occurrence_name_reads_its_own_clause() {
    assert_eq!(names("function useQuery<TData>(opts)", "useQuery"), vec!["TData".to_string()]);
}

#[test]
fn a_signature_without_the_name_falls_back_to_the_first_bracket() {
    assert_eq!(names("<T>(input: T) => T", "transform"), vec!["T".to_string()]);
}

#[test]
fn a_declaration_name_repeated_with_a_clause_reads_the_last_occurrence() {
    // `Foo<T> Foo<T>(...)` — the return type is the earlier occurrence; the
    // declaration clause is anchored on the last one.
    assert_eq!(names("Foo<T> Foo<T>(T value)", "Foo"), vec!["T".to_string()]);
}

#[test]
fn a_free_standing_clause_before_the_name_is_the_declaration_clause() {
    // Prefix-clause languages (`fun <T> listOf(...)`): the bracket group is
    // not glued to an identifier, so it belongs to the declaration, unlike a
    // return-type application (`NamedId<DomainId> NamedId(...)`).
    assert_eq!(
        names("fun <T> listOf(vararg elements: T): List<T>", "listOf"),
        vec!["T".to_string()],
    );
}

#[test]
fn a_glued_bracket_group_before_the_name_is_not_a_declaration_clause() {
    assert!(names("Task<Item> GetItem(int id)", "GetItem").is_empty());
}

#[test]
fn a_square_bracket_clause_anchors_on_the_name() {
    assert_eq!(names("def map[B](f: A => B): List[B]", "map"), vec!["B".to_string()]);
}

#[test]
fn a_partial_identifier_match_does_not_anchor() {
    // `Sync` must not anchor inside `SyncAll` — only a token-boundary
    // occurrence counts, so the locator falls back to the first bracket.
    assert_eq!(names("SyncAll<T>(x: T)", "Sync"), vec!["T".to_string()]);
}

#[test]
fn where_bounds_merge_into_the_anchored_clause() {
    let params = signature_generic_params("class Box<T> where T : IComparable", "Box");
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].0, "T");
    assert_eq!(params[0].1.as_deref(), Some("IComparable"));
}
