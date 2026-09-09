//! Compiler labels stay independent of the engine's observed ambient gaps.
use super::fixture_support::{run_selectors, run_selectors_cold, Fixture};
use super::{compare, OracleReport, Verdict};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    files: Vec<File>,
    labels: Vec<(u32, Option<u32>)>,
}
#[derive(Deserialize)]
struct File {
    path: String,
    source: String,
}
#[derive(Deserialize)]
struct Observed {
    name: String,
    report: OracleReport,
}

#[test]
fn configured_ambient_binding_ignores_signature_and_type_display_payloads() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("ambient_fixtures.json")).unwrap();
    for case in cases {
        let files: Vec<_> = case
            .files
            .iter()
            .map(|file| Fixture {
                path: &file.path,
                language: "typescript",
                marked_source: &file.source,
            })
            .collect();
        let expected =
            super::fixture_support::run_program_selectors(&files, &case.labels, false, |_| {})
                .unwrap();
        for cold in [false, true] {
            let poisoned =
                super::fixture_support::run_program_selectors(&files, &case.labels, cold, |file| {
                    for symbol in &mut file.symbols {
                        symbol.qualified_name = "poisoned.display".into();
                        symbol.signature = None;
                    }
                    if let Some(graph) = &mut file.flow.lexical {
                        for binding in &mut graph.bindings {
                            if binding.annotation.is_some() {
                                binding.annotation = Some("Poison".into());
                            }
                        }
                        for recipes in graph.call_type_args.values_mut() {
                            recipes.fill("Poison".into());
                        }
                    }
                    for reference in &mut file.refs {
                        if let Some(chain) = &mut reference.chain {
                            for segment in &mut chain.segments {
                                segment.type_args.clear();
                                segment.name = "poisoned.selector".into();
                            }
                        }
                    }
                })
                .unwrap();
            assert_eq!(
                poisoned, expected,
                "{}: display leaked into binding (cold={cold})",
                case.name
            );
        }
    }
}

#[test]
fn configured_ambient_calls_match_every_independent_target_fresh_and_cold() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("ambient_fixtures.json")).unwrap();
    assert_configured_cases(cases);
}

#[test]
fn configured_program_source_forms_match_compiler_targets_fresh_and_cold() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("program_fixtures.json")).unwrap();
    assert_configured_cases(cases);
}

#[test]
fn configured_ambient_module_cascades_match_independent_compiler_targets() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("ambient_module_fixtures.json")).unwrap();
    assert_configured_cases(cases);
}

#[test]
fn configured_export_entities_preserve_compiler_targets_and_cascades() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("export_entity_fixtures.json")).unwrap();
    assert_configured_cases(cases);
}

#[test]
fn configured_rich_interface_merges_match_compiler_targets() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("rich_merge_fixtures.json")).unwrap();
    assert_configured_cases(cases);
}

#[test]
fn configured_interface_heritage_matches_compiler_targets() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("interface_heritage_fixtures.json")).unwrap();
    assert_configured_cases(cases);
}

#[test]
fn configured_inherited_calls_preserve_compiler_targets_and_generic_cascades() {
    let mut cases: Vec<Case> =
        serde_json::from_str(include_str!("base_receiver_fixtures.json")).unwrap();
    cases.extend(
        serde_json::from_str::<Vec<Case>>(include_str!("configured_inheritance_fixtures.json"))
            .unwrap(),
    );
    assert_configured_cases(cases);
}

#[test]
fn configured_ambient_module_cascades_ignore_display_names_fresh_and_cold() {
    let mut cases: Vec<Case> =
        serde_json::from_str(include_str!("ambient_module_fixtures.json")).unwrap();
    cases.extend(
        serde_json::from_str::<Vec<Case>>(include_str!("export_entity_fixtures.json")).unwrap(),
    );
    cases.extend(
        serde_json::from_str::<Vec<Case>>(include_str!("base_receiver_fixtures.json")).unwrap(),
    );
    cases.extend(
        serde_json::from_str::<Vec<Case>>(include_str!("configured_inheritance_fixtures.json"))
            .unwrap(),
    );
    cases.extend(
        serde_json::from_str::<Vec<Case>>(include_str!("rich_merge_fixtures.json")).unwrap(),
    );
    cases.extend(
        serde_json::from_str::<Vec<Case>>(include_str!("interface_heritage_fixtures.json"))
            .unwrap(),
    );
    for case in cases {
        let files: Vec<_> = case
            .files
            .iter()
            .map(|file| Fixture {
                path: &file.path,
                language: "typescript",
                marked_source: &file.source,
            })
            .collect();
        let expected =
            super::fixture_support::run_program_selectors(&files, &case.labels, false, |_| {})
                .unwrap();
        for cold in [false, true] {
            let actual =
                super::fixture_support::run_program_selectors(&files, &case.labels, cold, |file| {
                    for symbol in &mut file.symbols {
                        symbol.qualified_name = "poisoned.display".into();
                        symbol.signature = None;
                    }
                    for reference in &mut file.refs {
                        if let Some(chain) = &mut reference.chain {
                            for segment in &mut chain.segments {
                                segment.name = "poisoned.selector".into();
                            }
                        }
                    }
                })
                .unwrap();
            assert_eq!(actual, expected, "{}: cold={cold}", case.name);
        }
    }
}

fn assert_configured_cases(cases: Vec<Case>) {
    let mut failures = Vec::new();
    for case in cases {
        let files: Vec<_> = case
            .files
            .iter()
            .map(|file| Fixture {
                path: &file.path,
                language: "typescript",
                marked_source: &file.source,
            })
            .collect();
        let fresh =
            super::fixture_support::run_program_selectors(&files, &case.labels, false, |_| {})
                .unwrap();
        let cold =
            super::fixture_support::run_program_selectors(&files, &case.labels, true, |_| {})
                .unwrap();
        assert_eq!(fresh, cold, "{}: snapshot divergence", case.name);
        if fresh
            .references
            .iter()
            .any(|o| !matches!(o.verdict, Verdict::Correct | Verdict::CorrectUnbound))
        {
            failures.push((case.name, fresh));
        }
    }
    assert!(
        failures.is_empty(),
        "Configured binding gaps: {failures:#?}"
    );
}

#[test]
fn ambient_compiler_cohort_preserves_known_targets_and_records_open_gaps() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("ambient_fixtures.json")).unwrap();
    let baseline: Vec<Observed> =
        serde_json::from_str(include_str!("ambient_observed_baseline.json")).unwrap();
    assert_eq!(cases.len(), baseline.len());
    for case in cases {
        let files: Vec<_> = case
            .files
            .iter()
            .map(|file| Fixture {
                path: &file.path,
                language: "typescript",
                marked_source: &file.source,
            })
            .collect();
        let fresh = run_selectors(&files, &case.labels).unwrap();
        let cold = run_selectors_cold(&files, &case.labels).unwrap();
        assert_eq!(fresh, cold, "{}: snapshot divergence", case.name);
        let before = &baseline
            .iter()
            .find(|b| b.name == case.name)
            .expect("Recorded cohort")
            .report;
        // Preserve the original failure evidence. A changed observation must
        // become compiler-correct, never a different wrong or missing target.
        for change in compare(before, &fresh).unwrap() {
            assert!(
                matches!(
                    change.after.verdict,
                    Verdict::Correct | Verdict::CorrectUnbound
                ),
                "{}: non-improving retarget {change:#?}",
                case.name
            );
        }
        eprintln!(
            "AMBIENT {} {}",
            case.name,
            serde_json::to_string(&fresh).unwrap()
        );
        assert_eq!(fresh.counts.labelled, case.labels.len() as u64);
        assert_eq!(fresh.counts.not_extracted, 0);
    }
}
