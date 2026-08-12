use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::type_checker::profile::language_profile::{
    ModuleAnchor, ModuleAnchorBind, ModulePrefixRewrites, DEFAULT_PROFILE,
};
use crate::types::{EdgeKind, ExtractedRef};

/// Build an `ExtractedRef` for `target` with `module` set.
fn module_ref(target: &str, module: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: Some(module.to_string()),
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    }
}

fn resolve(
    lookup: &Lookup,
    target: &str,
    module: &str,
    profile: &crate::type_checker::profile::language_profile::LanguageProfile,
) -> Option<i64> {
    let r = module_ref(target, module);
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
    match ModuleAnchorRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

/// Gate is `Off` by default — rule always passes.
#[test]
fn passes_when_gate_off() {
    let lookup = Lookup::new().with(sym(1, "map", "models.map", "function", "src/models.py"));
    assert_eq!(resolve(&lookup, "map", "models", &DEFAULT_PROFILE), None);
}

/// `ByNameUnderModuleDir`: qname probe `{module}.{target}` resolves.
#[test]
fn by_name_under_module_dir_qname_probe() {
    static PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
        crate::type_checker::profile::language_profile::LanguageProfile {
            implicit_root_types: &[],
            imports: crate::type_checker::profile::language_profile::ImportAxes {
                module_anchor: ModuleAnchor::On(ModuleAnchorBind::ByNameUnderModuleDir),
                module_prefix_rewrites: ModulePrefixRewrites::Off,
                ..DEFAULT_PROFILE.imports
            },
            ..DEFAULT_PROFILE
        };
    let lookup = Lookup::new().with(sym(10, "User", "models.User", "class", "src/models/user.py"));
    assert_eq!(resolve(&lookup, "User", "models", &PROFILE), Some(10));
}

/// `ByNameUnderModuleDir`: path-containment fallback when qname probe misses.
#[test]
fn by_name_under_module_dir_path_containment() {
    static PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
        crate::type_checker::profile::language_profile::LanguageProfile {
            implicit_root_types: &[],
            imports: crate::type_checker::profile::language_profile::ImportAxes {
                module_anchor: ModuleAnchor::On(ModuleAnchorBind::ByNameUnderModuleDir),
                module_prefix_rewrites: ModulePrefixRewrites::Off,
                ..DEFAULT_PROFILE.imports
            },
            ..DEFAULT_PROFILE
        };
    // The qname is `TextChoices` (no `models.` prefix) but the file contains `models/`.
    let lookup =
        Lookup::new().with(sym(20, "TextChoices", "TextChoices", "class", "app/models/enums.py"));
    assert_eq!(resolve(&lookup, "TextChoices", "models", &PROFILE), Some(20));
}

/// `ByNameUnderModuleDir` path-containment: a top-level declaration
/// (qname == bare target) outranks a member declaration of the same name
/// registered earlier in the same located file set — a module-qualified
/// target names a top-level item, never a member of a sibling type.
#[test]
fn path_containment_prefers_top_level_over_member() {
    static PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
        crate::type_checker::profile::language_profile::LanguageProfile {
            implicit_root_types: &[],
            imports: crate::type_checker::profile::language_profile::ImportAxes {
                module_anchor: ModuleAnchor::On(ModuleAnchorBind::ByNameUnderModuleDir),
                module_prefix_rewrites: ModulePrefixRewrites::Off,
                ..DEFAULT_PROFILE.imports
            },
            ..DEFAULT_PROFILE
        };
    // The member registers first under the name; the top-level fn must win.
    let lookup = Lookup::new()
        .with(sym(30, "parse", "Reader.parse", "method", "ext:rust:ser_x/src/de.rs"))
        .with(sym(31, "parse", "parse", "function", "ext:rust:ser_x/src/de.rs"));
    assert_eq!(resolve(&lookup, "parse", "ser_x", &PROFILE), Some(31));
}

/// `ByNameUnderModuleDir` path-containment: with no top-level declaration of
/// the name, a member declaration still binds — the second pass keeps the
/// pre-existing first-match fallback.
#[test]
fn path_containment_falls_back_to_member_when_no_top_level() {
    static PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
        crate::type_checker::profile::language_profile::LanguageProfile {
            implicit_root_types: &[],
            imports: crate::type_checker::profile::language_profile::ImportAxes {
                module_anchor: ModuleAnchor::On(ModuleAnchorBind::ByNameUnderModuleDir),
                module_prefix_rewrites: ModulePrefixRewrites::Off,
                ..DEFAULT_PROFILE.imports
            },
            ..DEFAULT_PROFILE
        };
    let lookup =
        Lookup::new().with(sym(32, "parse", "Reader.parse", "method", "ext:rust:ser_x/src/de.rs"));
    assert_eq!(resolve(&lookup, "parse", "ser_x", &PROFILE), Some(32));
}

/// `MemberOfModuleType`: `members_of(module)` → normalized name match.
#[test]
fn member_of_module_type_resolves() {
    static PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
        crate::type_checker::profile::language_profile::LanguageProfile {
            implicit_root_types: &[],
            imports: crate::type_checker::profile::language_profile::ImportAxes {
                module_anchor: ModuleAnchor::On(ModuleAnchorBind::MemberOfModuleType),
                ..DEFAULT_PROFILE.imports
            },
            ..DEFAULT_PROFILE
        };
    let member = sym(30, "Latitude", "Point.Latitude", "field", "src/geo.f90");
    let lookup = Lookup::new().with_member("Point", member);
    assert_eq!(resolve(&lookup, "Latitude", "Point", &PROFILE), Some(30));
}

/// No `module` on the ref — rule passes regardless of gate.
#[test]
fn passes_when_no_module_on_ref() {
    static PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
        crate::type_checker::profile::language_profile::LanguageProfile {
            implicit_root_types: &[],
            imports: crate::type_checker::profile::language_profile::ImportAxes {
                module_anchor: ModuleAnchor::On(ModuleAnchorBind::ByNameUnderModuleDir),
                module_prefix_rewrites: ModulePrefixRewrites::Off,
                ..DEFAULT_PROFILE.imports
            },
            ..DEFAULT_PROFILE
        };
    let lookup = Lookup::new().with(sym(40, "User", "models.User", "class", "src/models.py"));
    // No module on the ref — use call_ref which sets module to None.
    use crate::indexer::resolve::engine::testkit::call_ref;
    let r = call_ref("User");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &PROFILE,
    };
    assert!(matches!(ModuleAnchorRule.apply(&ctx), LookupResult::Pass));
}
