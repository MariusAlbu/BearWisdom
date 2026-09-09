use super::*;
use bearwisdom::{
    resolution_oracle::{
        evaluate, DeclarationSite, ExpectedReference, FixtureFileId, ReferenceSite,
    },
    types::{EdgeKind, SymbolKind},
};

fn population() -> (Vec<ProjectCall>, OracleReport) {
    let site = ReferenceSite {
        file: FixtureFileId(1),
        byte_offset: 7,
        kind: EdgeKind::Calls,
    };
    let target = DeclarationSite {
        file: FixtureFileId(2),
        line: 3,
        col: 5,
        kind: SymbolKind::Function,
    };
    let labels = vec![ExpectedReference {
        site,
        target: Some(target),
    }];
    let baseline = evaluate(CorpusRevision([9; 32]), &labels, &[]).unwrap();
    (
        vec![ProjectCall {
            site,
            target: Some(target),
            reason: None,
        }],
        baseline,
    )
}

#[test]
fn valid_pinned_targets_do_not_depend_on_baseline_counts() {
    let (calls, mut baseline) = population();
    baseline.counts.labelled = 999;
    validate_labels(&calls, baseline.revision, &baseline).unwrap();
}

#[test]
fn changed_revision_or_compiler_target_is_rejected() {
    let (calls, mut baseline) = population();
    assert!(validate_labels(&calls, CorpusRevision([8; 32]), &baseline).is_err());
    baseline.references[0].expected.target.as_mut().unwrap().col += 1;
    assert!(validate_labels(&calls, baseline.revision, &baseline).is_err());
}

#[test]
fn omitted_and_duplicated_occurrences_are_rejected() {
    let (mut calls, mut baseline) = population();
    let mut second = calls[0].site;
    second.byte_offset += 2;
    calls.push(ProjectCall {
        site: second,
        target: calls[0].target,
        reason: None,
    });
    assert!(validate_labels(&calls, baseline.revision, &baseline).is_err());
    baseline.references.push(baseline.references[0].clone());
    assert!(validate_labels(&calls, baseline.revision, &baseline).is_err());
}

#[test]
fn utf8_byte_offsets_and_crlf_are_not_character_columns() {
    let source = "é();\r\n  call()";
    assert_eq!(site_line(source, 0).unwrap(), 0);
    assert_eq!(site_line(source, 9).unwrap(), 1);
    assert!(site_line(source, 1).is_err());
    assert!(site_line(source, source.len() as u32).is_err());
    assert!(site_line(source, u32::MAX).is_err());
}

#[test]
fn baseline_binding_mode_is_preserved_and_unknown_modes_are_rejected() {
    let (_, fresh) = population();
    let payload = serde_json::json!({"fresh": fresh});
    let mut baseline: Baseline = serde_json::from_value(payload).unwrap();
    assert_eq!(binding_mode(&baseline).unwrap(), ProjectBindingMode::Legacy);
    baseline.binding_mode = Some("configured_program".into());
    assert_eq!(
        binding_mode(&baseline).unwrap(),
        ProjectBindingMode::ConfiguredProgram
    );
    baseline.binding_mode = Some("legacy".into());
    assert_eq!(binding_mode(&baseline).unwrap(), ProjectBindingMode::Legacy);
    baseline.binding_mode = Some("unknown".into());
    assert!(binding_mode(&baseline).is_err());
}
