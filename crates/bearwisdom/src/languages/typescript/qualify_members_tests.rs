use super::*;
use crate::languages::typescript::extract::extract;

/// `(name, qualified_name)` for every extracted symbol, in extraction order.
fn qnames(src: &str) -> Vec<(String, String)> {
    extract(src, false)
        .symbols
        .into_iter()
        .map(|s| (s.name, s.qualified_name))
        .collect()
}

fn qname_of(src: &str, name: &str) -> String {
    qnames(src)
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no symbol named {name}"))
        .1
}

#[test]
fn an_anonymous_union_arm_member_is_named_under_the_alias() {
    // A bare `any` in the qname index becomes the exact-qname hit for every
    // receiver whose head is `any`.
    let src = "export type Cond =\n  | { all: Cond[] }\n  | { any: Cond[] }\n  | { not: Cond };\n";
    assert_eq!(qname_of(src, "any"), "Cond.any");
    assert_eq!(qname_of(src, "all"), "Cond.all");
    assert_eq!(qname_of(src, "not"), "Cond.not");
}

#[test]
fn an_inline_object_type_member_flattens_onto_the_declaring_type() {
    // `turbo`'s type is anonymous, so no receiver can name it — `createProject`
    // is reachable only as a member of the interface that declares it.
    let src = "interface Binding {\n  turbo: { createProject(): void };\n}\n";
    assert_eq!(qname_of(src, "turbo"), "Binding.turbo");
    assert_eq!(qname_of(src, "createProject"), "Binding.createProject");
}

#[test]
fn an_inline_object_member_of_a_type_alias_is_named_under_the_alias() {
    let src = "type Cfg = { nested: { deep: string } };\n";
    assert_eq!(qname_of(src, "nested"), "Cfg.nested");
    assert_eq!(qname_of(src, "deep"), "Cfg.deep");
}

#[test]
fn a_class_member_is_unchanged() {
    let src = "export class Repo {\n  db: string;\n  find(): void {}\n}\n";
    assert_eq!(qname_of(src, "db"), "Repo.db");
    assert_eq!(qname_of(src, "find"), "Repo.find");
}

#[test]
fn a_namespaced_interface_member_is_unchanged() {
    let src = "namespace N {\n  export interface I {\n    m(): void;\n  }\n}\n";
    assert_eq!(qname_of(src, "m"), "N.I.m");
}

#[test]
fn a_top_level_declaration_keeps_its_bare_qname() {
    let src = "export type Cond = { any: string };\nexport function any2(): void {}\n";
    assert_eq!(qname_of(src, "Cond"), "Cond");
    assert_eq!(qname_of(src, "any2"), "any2");
}

#[test]
fn a_flattened_member_keeps_scope_path_at_its_immediate_parent() {
    // SYM-002: scope_path == symbols[parent_index].qualified_name, even when
    // the qname is flattened past an inline-object property.
    let syms = extract("interface Binding {\n  turbo: { createProject(): void };\n}\n", false).symbols;
    for s in &syms {
        if let Some(p) = s.parent_index {
            assert_eq!(
                s.scope_path.as_deref(),
                Some(syms[p].qualified_name.as_str()),
                "SYM-002 broken for '{}'",
                s.name
            );
        }
    }
    let cp = syms.iter().find(|s| s.name == "createProject").unwrap();
    assert_eq!(cp.qualified_name, "Binding.createProject");
    assert_eq!(cp.scope_path.as_deref(), Some("Binding.turbo"));
}

#[test]
fn name_under_parents_is_idempotent() {
    let mut syms = extract("interface B {\n  t: { c(): void };\n}\n", false).symbols;
    let before: Vec<String> = syms.iter().map(|s| s.qualified_name.clone()).collect();
    name_under_parents(&mut syms);
    let after: Vec<String> = syms.iter().map(|s| s.qualified_name.clone()).collect();
    assert_eq!(before, after);
}
