//! Expected targets live in compiler-validated source markers, not this snapshot.
use super::fixture_support::{run_with_mutation, Fixture};
use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    entry: String,
    files: Vec<File>,
    labels: Vec<(u32, Option<u32>)>,
}
#[derive(Deserialize)]
struct File {
    path: String,
    source: String,
}

fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!("rust_fixtures.json")).unwrap()
}

fn receiver_cases() -> Vec<Case> {
    serde_json::from_str(include_str!("rust_receiver_fixtures.json")).unwrap()
}

fn trait_cases() -> Vec<Case> {
    serde_json::from_str(include_str!("rust_trait_fixtures.json")).unwrap()
}

fn selection_cases() -> Vec<Case> {
    serde_json::from_str(include_str!("rust_trait_selection_fixtures.json")).unwrap()
}

fn qualified_cases() -> Vec<Case> {
    serde_json::from_str(include_str!("rust_qualified_fixtures.json")).unwrap()
}

fn borrow_cases() -> Vec<Case> {
    serde_json::from_str(include_str!("rust_borrow_fixtures.json")).unwrap()
}

fn argument_cases() -> Vec<Case> {
    serde_json::from_str(include_str!("rust_argument_fixtures.json")).unwrap()
}

fn local_value_cases() -> Vec<Case> {
    serde_json::from_str(include_str!("rust_local_value_fixtures.json")).unwrap()
}

fn place_cases() -> Vec<Case> {
    serde_json::from_str(include_str!("rust_place_fixtures.json")).unwrap()
}

fn pattern_cases() -> Vec<Case> {
    serde_json::from_str(include_str!("rust_pattern_fixtures.json")).unwrap()
}

#[test]
fn enum_pattern_payloads_bind_compiler_targets_and_return_cascades() {
    let cases = pattern_cases()
        .into_iter()
        .chain(
            cases()
                .into_iter()
                .filter(|c| c.name == "match_pattern_payload_provenance"),
        )
        .collect::<Vec<_>>();
    assert_eq!(cases.len(), 8);
    for case in cases {
        let fresh = run_case(&case, false);
        assert_eq!(
            fresh,
            run_case(&case, true),
            "{}: fresh/cold disagreement",
            case.name
        );
        assert_eq!(
            (fresh.counts.correct + fresh.counts.correct_unbound) as usize,
            case.labels.len(),
            "{}: {fresh:#?}",
            case.name
        );
        assert_eq!(fresh.counts.unlabelled_observations, 0, "{}", case.name);
    }
}

#[test]
fn source_owned_place_operands_preserve_compiler_targets_and_return_cascades() {
    let cases = place_cases();
    assert_eq!(cases.len(), 12);
    for case in cases {
        let fresh = run_case(&case, false);
        assert_eq!(
            fresh,
            run_case(&case, true),
            "{}: fresh/cold disagreement",
            case.name
        );
        assert_eq!(
            (fresh.counts.correct + fresh.counts.correct_unbound) as usize,
            case.labels.len(),
            "{}: {fresh:#?}",
            case.name
        );
        assert_eq!(fresh.counts.unlabelled_observations, 0, "{}", case.name);
    }
}

#[test]
fn source_owned_borrow_locals_preserve_compiler_targets_and_return_cascades() {
    let cases = local_value_cases();
    assert_eq!(cases.len(), 9);
    for case in cases {
        let fresh = run_case(&case, false);
        assert_eq!(
            fresh,
            run_case(&case, true),
            "{}: fresh/cold disagreement",
            case.name
        );
        assert_eq!(
            (fresh.counts.correct + fresh.counts.correct_unbound) as usize,
            case.labels.len(),
            "{}: {fresh:#?}",
            case.name
        );
        assert_eq!(fresh.counts.unlabelled_observations, 0, "{}", case.name);
    }
}

#[test]
fn source_call_operands_bind_bare_member_and_namespace_return_cascades() {
    let cases = argument_cases();
    assert_eq!(cases.len(), 8);
    for case in cases {
        let fresh = run_case(&case, false);
        assert_eq!(
            fresh,
            run_case(&case, true),
            "{}: fresh/cold disagreement",
            case.name
        );
        assert_eq!(
            (fresh.counts.correct + fresh.counts.correct_unbound) as usize,
            case.labels.len(),
            "{}: {fresh:#?}",
            case.name
        );
        assert_eq!(fresh.counts.unlabelled_observations, 0, "{}", case.name);
    }
}

#[test]
fn explicit_borrow_arguments_preserve_compiler_targets_and_return_cascades() {
    let cases = borrow_cases();
    assert_eq!(cases.len(), 8);
    for case in cases {
        let fresh = run_case(&case, false);
        assert_eq!(
            fresh,
            run_case(&case, true),
            "{}: fresh/cold disagreement",
            case.name
        );
        assert_eq!(
            (fresh.counts.correct + fresh.counts.correct_unbound) as usize,
            case.labels.len(),
            "{}: {fresh:#?}",
            case.name
        );
        assert_eq!(fresh.counts.unlabelled_observations, 0, "{}", case.name);
    }
}

#[test]
fn qualified_trait_call_cascades_preserve_receivers_arguments_and_negatives() {
    let cases = qualified_cases();
    assert_eq!(cases.len(), 8);
    for case in cases {
        let fresh = run_case(&case, false);
        assert_eq!(
            fresh,
            run_case(&case, true),
            "{}: fresh/cold disagreement",
            case.name
        );
        assert_eq!(
            (fresh.counts.correct + fresh.counts.correct_unbound) as usize,
            case.labels.len(),
            "{}: {fresh:#?}",
            case.name
        );
        assert_eq!(fresh.counts.unlabelled_observations, 0, "{}", case.name);
    }
}

#[test]
fn qualified_trait_calls_bind_exact_compiler_targets_fresh_and_cold() {
    let cases: Vec<_> = cases()
        .into_iter()
        .chain(trait_cases())
        .filter(|c| {
            matches!(
                c.name.as_str(),
                "trait_default_and_generic_dispatch"
                    | "trait_override_static_declaration_targets"
                    | "renamed_cross_file_trait_and_receiver"
            )
        })
        .collect();
    assert_eq!(cases.len(), 3);
    for case in cases {
        for cold in [false, true] {
            let result = run_case(&case, cold);
            assert_eq!(
                result.counts.correct as usize,
                case.labels.len(),
                "{}: {result:#?}",
                case.name
            );
            assert_eq!(result.counts.unlabelled_observations, 0, "{}", case.name);
        }
    }
}

#[test]
fn compiler_labelled_trait_selection_contract_is_complete_fresh_and_cold() {
    let cases = selection_cases();
    assert_eq!(cases.len(), 6);
    for case in cases {
        for cold in [false, true] {
            let result = run_case(&case, cold);
            assert_eq!(
                (result.counts.correct + result.counts.correct_unbound) as usize,
                case.labels.len(),
                "{}: {result:#?}",
                case.name
            );
            assert_eq!(result.counts.unlabelled_observations, 0, "{}", case.name);
        }
    }
}

#[test]
fn compiler_labelled_owned_receiver_cascades_bind_every_occurrence_fresh_and_cold() {
    let cases = receiver_cases();
    assert_eq!(cases.len(), 6);
    for case in cases {
        let fresh = run_case(&case, false);
        assert_eq!(
            fresh,
            run_case(&case, true),
            "{}: fresh/cold disagreement",
            case.name
        );
        assert_eq!(
            fresh.counts.correct as usize,
            case.labels.len(),
            "{}: {fresh:#?}",
            case.name
        );
        assert_eq!(fresh.counts.unlabelled_observations, 0, "{}", case.name);
    }
    let retained = cases_original_owned();
    for cold in [false, true] {
        let result = run_case(&retained, cold);
        assert_eq!(result.counts.correct, 2, "{result:#?}");
    }
}

fn cases_original_owned() -> Case {
    cases()
        .into_iter()
        .find(|case| case.name == "owned_receiver_auto_borrow_region")
        .unwrap()
}

fn run_case(case: &Case, cold: bool) -> OracleReport {
    run_case_mutated(case, cold, |_| {})
}

fn run_case_mutated(
    case: &Case,
    cold: bool,
    mutate: impl FnMut(&mut crate::types::ParsedFile),
) -> OracleReport {
    let files: Vec<_> = case
        .files
        .iter()
        .map(|file| Fixture {
            path: &file.path,
            language: "rust",
            marked_source: &file.source,
        })
        .collect();
    let manifest = format!(
        "[package]\nname='bw_target_oracle'\nversion='0.0.0'\nedition='2021'\n[lib]\npath='{}'\n",
        case.entry
    );
    run_with_mutation(
        &files,
        &case.labels,
        true,
        cold,
        &[("Cargo.toml", &manifest)],
        mutate,
    )
    .unwrap_or_else(|error| panic!("{}: {error:#}", case.name))
}

#[test]
fn source_call_targets_ignore_poisoned_or_empty_display_operands_fresh_and_cold() {
    for case in argument_cases()
        .into_iter()
        .chain(qualified_cases())
        .chain(borrow_cases())
        .chain(local_value_cases())
        .chain(place_cases())
        .chain(pattern_cases())
    {
        let expected = run_case(&case, false);
        for cold in [false, true] {
            for empty in [false, true] {
                let actual = run_case_mutated(&case, cold, |file| {
                    if let Some(graph) = &mut file.flow.lexical {
                        for binding in &mut graph.bindings {
                            if binding.annotation.is_some() {
                                binding.annotation = Some("not_a_source_type".into());
                            }
                        }
                    }
                    for reference in &mut file.refs {
                        let poison = if empty {
                            vec![]
                        } else {
                            vec![crate::types::CallArg::Ident("not_a_source_binding".into()); 3]
                        };
                        reference.call_args = poison.clone();
                        if let Some(chain) = &mut reference.chain {
                            for segment in &mut chain.segments {
                                segment.call_args = poison.clone();
                            }
                        }
                    }
                });
                assert_eq!(
                    actual, expected,
                    "{}: cold={cold}, empty={empty}",
                    case.name
                );
            }
        }
    }
}

#[derive(Deserialize)]
struct Snapshot {
    revision: CorpusRevision,
    observations: Vec<Observation>,
}

#[test]
fn compiler_labelled_rust_baseline_has_no_per_site_regressions() {
    check_baselines(cases(), include_str!("rust_snapshot.json"));
}

#[test]
fn compiler_labelled_trait_baseline_preserves_negatives_and_tracks_wrong_target() {
    let cases = trait_cases();
    assert_eq!(cases.len(), 12);
    check_baselines(cases, include_str!("rust_trait_snapshot.json"));
}

#[test]
fn compiler_labelled_trait_receiver_order_and_output_cascades_are_correct() {
    for case in trait_cases().into_iter().filter(|c| {
        matches!(
            c.name.as_str(),
            "shared_trait_receiver_precedes_mutable_inherent"
                | "trait_generic_output_and_namesake_payload_cascade"
        )
    }) {
        for cold in [false, true] {
            let result = run_case(&case, cold);
            assert_eq!(
                result.counts.correct as usize,
                case.labels.len(),
                "{}: {result:#?}",
                case.name
            );
            assert_eq!(result.counts.incorrect, 0, "{}", case.name);
        }
    }
}

#[test]
fn corrected_rust_sites_cannot_regress_to_the_old_unresolved_baseline() {
    // Independent labels and historical observations remain unchanged.
    // All previously corrected sites, including both match payloads, are strict.
    for case in cases().into_iter().chain(trait_cases()) {
        for cold in [false, true] {
            let report = run_case(&case, cold);
            for reference in &report.references {
                assert!(
                    matches!(
                        reference.verdict,
                        Verdict::Correct | Verdict::CorrectUnbound
                    ),
                    "{}: {reference:#?}",
                    case.name
                );
            }
        }
    }
}

fn check_baselines(cases: Vec<Case>, json: &str) {
    let snapshots: std::collections::BTreeMap<String, Snapshot> =
        serde_json::from_str(json).unwrap();
    assert_eq!(
        cases.len(),
        snapshots.len(),
        "Every case needs an explicitly reviewed observation snapshot"
    );
    let mut names = std::collections::HashSet::new();
    for case in cases {
        assert!(names.insert(case.name.clone()), "Duplicate case name");
        let fresh = run_case(&case, false);
        assert_eq!(
            fresh,
            run_case(&case, true),
            "{}: fresh/cold disagreement",
            case.name
        );
        let snapshot = &snapshots[&case.name];
        assert_eq!(
            fresh.revision, snapshot.revision,
            "{}: fixture/configuration/labels changed; review before rebaseline",
            case.name
        );
        let labels: Vec<_> = fresh
            .references
            .iter()
            .map(|r| r.expected.clone())
            .collect();
        let before = evaluate(snapshot.revision, &labels, &snapshot.observations).unwrap();
        for change in compare(&before, &fresh).unwrap() {
            assert!(
                !change.regressed,
                "{}: per-site regression: {change:#?}",
                case.name
            );
            assert!(
                !(change.retargeted && change.after.verdict == Verdict::Incorrect),
                "{}: wrong-to-wrong retargeting: {change:#?}",
                case.name
            );
        }
        assert_eq!(
            fresh.counts.unlabelled_observations, 0,
            "{}: unlabelled calls must not silently escape the cohort",
            case.name
        );
    }
}

#[test]
fn compiler_rejected_bare_call_cannot_select_an_unrelated_module_function() {
    let case = cases()
        .into_iter()
        .find(|c| c.name == "missing_bare_function_is_not_a_namesake")
        .unwrap();
    for cold in [false, true] {
        let result = run_case(&case, cold);
        assert_eq!(result.counts.correct_unbound, 1, "{result:#?}");
        assert_eq!(result.counts.incorrect, 0);
    }
}

#[test]
#[ignore = "Read-only diagnostic: prints observations; never writes target labels or snapshots"]
fn report_compiler_labelled_rust_cohort() {
    for case in cases()
        .into_iter()
        .chain(receiver_cases())
        .chain(trait_cases())
        .chain(selection_cases())
        .chain(qualified_cases())
        .chain(borrow_cases())
        .chain(argument_cases())
        .chain(local_value_cases())
        .chain(place_cases())
        .chain(pattern_cases())
    {
        let fresh = run_case(&case, false);
        let cold = run_case(&case, true);
        assert_eq!(fresh, cold, "{}: cold snapshot differs", case.name);
        println!("{}: {}", case.name, serde_json::to_string(&fresh).unwrap());
    }
}

#[test]
#[ignore = "Read-only compiler-labelled trait diagnostic; does not rewrite labels or observations"]
fn report_compiler_labelled_trait_cohort() {
    for case in trait_cases() {
        let fresh = run_case(&case, false);
        assert_eq!(
            fresh,
            run_case(&case, true),
            "{}: cold snapshot differs",
            case.name
        );
        println!("{}: {}", case.name, serde_json::to_string(&fresh).unwrap());
    }
}
