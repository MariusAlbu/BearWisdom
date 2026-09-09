use super::*;

#[test]
fn program_inputs_roundtrip_without_collapsing_shared_source_membership() {
    let source = ProgramSource {
        path: "shared.ts".into(),
        content_hash: "source".into(),
        scope: SourceScope::Syntax,
    };
    let a = Program {
        key: "a/config".into(),
        fingerprint: "a".into(),
        complete: true,
        callable_policy: None,
        compiler_intrinsics: None,
        source_binding_order: None,
        sources: vec![source.clone()],
    };
    let b = Program {
        key: "b/config".into(),
        fingerprint: "b".into(),
        complete: false,
        callable_policy: None,
        compiler_intrinsics: None,
        source_binding_order: None,
        sources: vec![ProgramSource {
            scope: SourceScope::Module,
            ..source
        }],
    };
    let inputs = vec![a, b];
    let restored: Vec<Program> =
        serde_json::from_str(&serde_json::to_string(&inputs).unwrap()).unwrap();
    assert_eq!(restored, inputs);
    let context = super::super::project_context::ProjectContext {
        programs: Some(inputs),
        ..Default::default()
    };
    assert_eq!(
        context.clone().programs,
        context.programs,
        "program evidence survives context cloning"
    );
}

#[test]
fn old_program_inputs_do_not_invent_callable_policy() {
    let old_policy: CallablePolicy =
        serde_json::from_str(r#"{"strict_parameters":true,"strict_nulls":true}"#).unwrap();
    assert_eq!(old_policy.bivariant_methods, None);
    let old: Program = serde_json::from_str(
        r#"{"key":"legacy","fingerprint":"hash","complete":true,"sources":[]}"#,
    )
    .unwrap();
    assert_eq!(old.callable_policy, None);
    assert_eq!(old.compiler_intrinsics, None);
    assert_eq!(old.source_binding_order, None);
    let configured = Program {
        callable_policy: Some(CallablePolicy {
            strict_parameters: true,
            strict_nulls: false,
            bivariant_methods: None,
        }),
        compiler_intrinsics: Some(CompilerIntrinsicPolicy {
            strict_iterator_return: true,
        }),
        ..old
    };
    assert_eq!(
        serde_json::from_str::<Program>(&serde_json::to_string(&configured).unwrap()).unwrap(),
        configured
    );
}
