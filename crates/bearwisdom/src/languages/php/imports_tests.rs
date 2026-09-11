use super::*;
use crate::types::{EdgeKind, SegmentKind};

fn imports_refs(source: &str) -> Vec<crate::types::ExtractedRef> {
    crate::languages::php::extract::extract(source)
        .refs
        .into_iter()
        .filter(|r| r.kind == EdgeKind::Imports)
        .collect()
}

#[test]
fn aliased_use_carries_original_as_chain_and_alias_as_target() {
    let source = "<?php\nuse App\\Contracts\\Auth\\Factory as FactoryContract;\n";
    let refs = imports_refs(source);
    let imp = refs.iter().find(|r| r.target_name == "FactoryContract");
    let imp = imp.expect("expected an Imports ref bound to the alias");
    assert_eq!(imp.module.as_deref(), Some("App\\Contracts\\Auth"));
    let chain = imp.chain.as_ref().expect("expected a rename chain");
    assert_eq!(chain.segments.len(), 1);
    assert_eq!(chain.segments[0].name, "Factory");
    assert_eq!(chain.segments[0].kind, SegmentKind::TypeAccess);
}

#[test]
fn non_aliased_class_use_preserves_type_binding_shape() {
    let source = "<?php\nuse App\\Models\\User;\n";
    let refs = imports_refs(source);
    let imp = refs
        .iter()
        .find(|r| r.target_name == "User")
        .expect("expected an Imports ref");
    assert_eq!(imp.module.as_deref(), Some("App\\Models"));
    let chain = imp.chain.as_ref().expect("expected class import shape");
    assert_eq!(chain.segments.len(), 1);
    assert_eq!(chain.segments[0].name, "User");
    assert_eq!(chain.segments[0].kind, SegmentKind::TypeAccess);
}

#[test]
fn grouped_use_emits_one_ref_per_member() {
    let source = "<?php\nuse App\\{Models\\User, Services\\Logger as Log};\n";
    let refs = imports_refs(source);
    let user = refs
        .iter()
        .find(|r| r.target_name == "User")
        .expect("expected a User import from the group");
    assert_eq!(user.module.as_deref(), Some("App\\Models"));
    assert_eq!(
        user.chain
            .as_ref()
            .expect("grouped class import shape")
            .segments[0]
            .kind,
        SegmentKind::TypeAccess
    );

    let log = refs
        .iter()
        .find(|r| r.target_name == "Log")
        .expect("expected a Log import from the group");
    assert_eq!(log.module.as_deref(), Some("App\\Services"));
    let chain = log.chain.as_ref().expect("expected a rename chain");
    assert_eq!(chain.segments[0].name, "Logger");
    assert_eq!(chain.segments[0].kind, SegmentKind::TypeAccess);
}

#[test]
fn use_function_and_use_const_extract_target_and_module() {
    let source =
        "<?php\nuse function App\\Support\\enum_value;\nuse const App\\Support\\MAX_SIZE;\n";
    let refs = imports_refs(source);
    let func = refs
        .iter()
        .find(|r| r.target_name == "enum_value")
        .expect("expected the function import");
    assert_eq!(func.module.as_deref(), Some("App\\Support"));
    assert!(
        func.chain.is_none(),
        "unaliased function import is value-shaped"
    );

    let konst = refs
        .iter()
        .find(|r| r.target_name == "MAX_SIZE")
        .expect("expected the const import");
    assert_eq!(konst.module.as_deref(), Some("App\\Support"));
    assert!(
        konst.chain.is_none(),
        "unaliased const import is value-shaped"
    );
}

#[test]
fn value_import_alias_cannot_authorize_a_qualified_type_demand() {
    let source = r#"<?php
use function Vendor\EloquentFactory as Eloquent;
class Service { public function run(): void { Eloquent\Builder::query(); } }
"#;
    let result = crate::languages::php::extract::extract(source);
    let import = result
        .refs
        .iter()
        .find(|reference| reference.kind == EdgeKind::Imports)
        .expect("function import");
    assert_eq!(
        import
            .chain
            .as_ref()
            .expect("aliased value binding")
            .segments[0]
            .kind,
        SegmentKind::Identifier
    );
    assert!(!result.refs.iter().any(|reference| {
        reference.kind == EdgeKind::TypeRef
            && reference.module.as_deref() == Some("Vendor\\EloquentFactory")
            && reference.target_name == "Builder"
    }));
}

#[test]
fn aliased_implements_rewrites_to_the_original_declared_name() {
    let source = r#"<?php
use App\Contracts\Auth\Factory as FactoryContract;
class AuthManager implements FactoryContract {}
"#;
    let r = crate::languages::php::extract::extract(source);
    let imp = r
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Implements)
        .expect("expected an Implements ref");
    assert_eq!(
        imp.target_name, "Factory",
        "expected the alias rewritten to the original class name: {:?}",
        imp.target_name
    );
}

#[test]
fn aliased_instantiation_rewrites_to_the_original_declared_name() {
    let source = r#"<?php
use App\Models\User as UserModel;
function make() {
    return new UserModel();
}
"#;
    let r = crate::languages::php::extract::extract(source);
    let inst = r
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Instantiates)
        .expect("expected an Instantiates ref");
    assert_eq!(
        inst.target_name, "User",
        "expected the alias rewritten to the original class name: {:?}",
        inst.target_name
    );
}

#[test]
fn aliased_parent_rewrite_carries_the_import_namespace() {
    let source = r#"<?php
namespace App\Support;
use Carbon\Carbon as BaseCarbon;
class Carbon extends BaseCarbon {}
"#;
    let refs = crate::languages::php::extract::extract(source).refs;
    let inherits = refs
        .iter()
        .find(|r| r.kind == EdgeKind::Inherits)
        .expect("inherits ref");
    assert_eq!(inherits.target_name, "Carbon");
    assert_eq!(inherits.module.as_deref(), Some("Carbon"));
}
