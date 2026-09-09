//! Cross-file ground truth is authored in the shared compiler-checkable fixture.
use super::fixture_support::{run_selectors, run_selectors_cold, Fixture};
use super::Verdict;
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

fn check(name: &str) {
    let cases: Vec<Case> = serde_json::from_str(include_str!("module_fixtures.json")).unwrap();
    let case = cases
        .into_iter()
        .find(|case| case.name == name)
        .expect("named fixture exists");
    let files: Vec<_> = case
        .files
        .iter()
        .map(|file| Fixture {
            path: &file.path,
            language: "typescript",
            marked_source: &file.source,
        })
        .collect();
    for (mode, report) in [
        ("fresh", run_selectors(&files, &case.labels)),
        ("cold", run_selectors_cold(&files, &case.labels)),
    ] {
        for site in &report.unwrap().references {
            assert_eq!(
                site.verdict,
                if site.expected.target.is_some() {
                    Verdict::Correct
                } else {
                    Verdict::CorrectUnbound
                },
                "{} ({mode}): {site:#?}",
                case.name
            );
        }
    }
}

#[test]
fn source_bound_base_receivers_match_compiler_targets() {
    check_base(include_str!("base_receiver_fixtures.json"), "typescript");
}

#[test]
fn source_bound_javascript_base_receivers_match_compiler_targets() {
    check_base(
        include_str!("javascript_base_receiver_fixtures.json"),
        "javascript",
    );
}

fn check_base(input: &str, language: &'static str) {
    let cases: Vec<Case> = serde_json::from_str(input).unwrap();
    for case in cases {
        let files: Vec<_> = case
            .files
            .iter()
            .map(|file| Fixture {
                path: &file.path,
                language,
                marked_source: &file.source,
            })
            .collect();
        for (mode, cold, poison) in [
            ("fresh", false, false),
            ("cold", true, false),
            ("poisoned fresh", false, true),
            ("poisoned cold", true, true),
        ] {
            let report = super::fixture_support::run_with_mutation(
                &files,
                &case.labels,
                true,
                cold,
                &[],
                |file| {
                    if !poison {
                        return;
                    }
                    for reference in &mut file.refs {
                        if reference.kind == crate::types::EdgeKind::Inherits {
                            reference.target_name = "Unrelated".into();
                        }
                        if let Some(segment) = reference
                            .chain
                            .as_mut()
                            .and_then(|c| c.segments.first_mut())
                        {
                            if segment.kind == crate::types::SegmentKind::BaseRef {
                                segment.name = "this".into();
                            }
                        }
                    }
                },
            );
            for site in report.unwrap().references {
                assert_eq!(
                    site.verdict,
                    if site.expected.target.is_some() {
                        Verdict::Correct
                    } else {
                        Verdict::CorrectUnbound
                    },
                    "{} ({mode}): {site:#?}",
                    case.name
                );
            }
        }
    }
}

macro_rules! fixture {
    ($name:ident) => {
        #[test]
        fn $name() {
            check(stringify!($name));
        }
    };
}
fixture!(named_imports_keep_distinct_type_and_callable_declarations);
fixture!(imported_class_and_function_arguments_keep_generic_ids);
fixture!(renamed_reexports_and_local_export_aliases_keep_identity);
fixture!(non_exported_declarations_cannot_be_imported_by_name);
fixture!(relative_module_paths_do_not_borrow_same_basename_files);
fixture!(wildcard_cycles_terminate_and_explicit_exports_win);
fixture!(named_default_exports_and_type_only_imports_keep_type_identity);
fixture!(local_values_shadow_import_aliases_without_retargeting_types);
fixture!(exported_aliases_and_callback_signatures_keep_imported_type_ids);
fixture!(default_and_namespace_exports_do_not_leak_wildcard_members);
fixture!(broken_explicit_reexport_cannot_borrow_a_wildcard_namesake);
fixture!(declaration_file_exports_preserve_callable_and_value_signatures);
fixture!(imported_overload_groups_preserve_shared_return_members_without_selecting_a_target);
fixture!(namespace_import_calls_and_returns_keep_exact_export_identity);
fixture!(namespace_qualified_types_and_aliases_keep_declaration_identity);
fixture!(nested_namespace_reexports_keep_module_identity);
fixture!(namespace_private_missing_and_type_only_values_cannot_borrow_targets);
fixture!(namespace_value_shadows_do_not_retarget_type_space);
fixture!(namespace_generic_calls_keep_explicit_inferred_and_callback_type_ids);
fixture!(scoped_merge_interfaces_preserve_both_member_declarations);
fixture!(scoped_merge_class_interface_is_independent_of_declaration_order);
fixture!(scoped_merge_block_namesakes_cannot_share_members);
fixture!(scoped_merge_generic_parameters_follow_group_identity);
fixture!(scoped_merge_type_and_value_exports_remain_separate);
fixture!(scoped_merge_constructor_names_do_not_grant_instance_members);
