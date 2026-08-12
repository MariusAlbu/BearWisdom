use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::{DEFAULT_PROFILE, LanguageProfile};

fn resolve_for(lookup: &Lookup, target: &str, profile: &LanguageProfile) -> Option<i64> {
    let r = call_ref(target);
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind: &kind,
        profile,
    };
    match ImplicitPreludeRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

static JAVA_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    implicit_prelude_namespaces: &["java.lang"],
    id: "java",
    ..DEFAULT_PROFILE
};

static KOTLIN_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    implicit_prelude_namespaces: &["kotlin", "kotlin.collections"],
    id: "kotlin",
    ..DEFAULT_PROFILE
};

static BICEP_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    implicit_prelude_namespaces: &["bicep.builtins", "bicep.decorators"],
    id: "bicep",
    ..DEFAULT_PROFILE
};

#[test]
fn java_lang_string_resolves() {
    // `java.lang.String` is in the Java implicit prelude; bare `String` must bind.
    let lookup = Lookup::new().with(sym(1, "String", "java.lang.String", "class", "ext:java:jdk/src/String.java"));
    assert_eq!(resolve_for(&lookup, "String", &JAVA_PROFILE), Some(1));
}

#[test]
fn empty_prelude_profile_declines() {
    // A profile with no implicit_prelude_namespaces (the DEFAULT_PROFILE case) —
    // the rule must pass regardless of language id.
    let lookup = Lookup::new().with(sym(2, "String", "java.lang.String", "class", "ext:java:jdk/src/String.java"));
    assert_eq!(resolve_for(&lookup, "String", &DEFAULT_PROFILE), None);
}

#[test]
fn nested_member_declined() {
    // `java.lang.reflect.Method` has an extra segment beyond `java.lang` — not a
    // direct member, so the rule declines it.
    let lookup = Lookup::new().with(sym(3, "Method", "java.lang.reflect.Method", "class", "ext:java:jdk/src/Method.java"));
    assert_eq!(resolve_for(&lookup, "Method", &JAVA_PROFILE), None);
}

#[test]
fn bicep_bare_arm_builtin_resolves_over_same_name_internal_declaration() {
    // Bicep resource/variable declarations often share a name with an ARM
    // builtin (`resource resourceGroup 'Microsoft.Resources/...' = {...}`).
    // The internal declaration's qname is the bare name itself — it doesn't
    // match the `bicep.builtins.<name>` pattern, so only the stub binds.
    let lookup = Lookup::new()
        .with(sym(20, "resourceGroup", "bicep.builtins.resourceGroup", "function", "ext:bicep-runtime:namespace.bicep"))
        .with(sym(21, "resourceGroup", "resourceGroup", "class", "main.bicep"));
    assert_eq!(resolve_for(&lookup, "resourceGroup", &BICEP_PROFILE), Some(20));
}

#[test]
fn bicep_bare_decorator_resolves_through_decorators_namespace() {
    // `@description(...)` has no `sys.`/`az.` prefix; the decorator namespace
    // is a second listed prelude namespace, distinct from `bicep.builtins`.
    let lookup = Lookup::new().with(sym(22, "description", "bicep.decorators.description", "function", "ext:bicep-runtime:namespace.bicep"));
    assert_eq!(resolve_for(&lookup, "description", &BICEP_PROFILE), Some(22));
}

#[test]
fn ambiguous_two_prelude_members_declines() {
    // Two distinct qnames (kotlin.String vs kotlin.collections.String) both
    // matching — the rule must decline rather than guess.
    let lookup = Lookup::new()
        .with(sym(10, "String", "kotlin.String", "class", "ext:kotlin:a.kt"))
        .with(sym(11, "String", "kotlin.collections.String", "class", "ext:kotlin:b.kt"));
    // kotlin has both `kotlin` and `kotlin.collections` in its prelude.
    assert_eq!(resolve_for(&lookup, "String", &KOTLIN_PROFILE), None);
}
