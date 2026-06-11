// Sibling test file for `main.rs`. Covers the corpus-class grouping used by
// `quality-check` to report the application rate apart from framework-source
// and fixture-heavy repos, with duplicates excluded from every total.

use super::*;

fn entry(project: &str, class: Option<&str>) -> serde_json::Value {
    match class {
        Some(c) => serde_json::json!({ "project": project, "corpus_class": c }),
        None => serde_json::json!({ "project": project }),
    }
}

#[test]
fn corpus_class_of_defaults_to_none_when_absent_or_blank() {
    assert_eq!(corpus_class_of(&entry("app", None)), None);
    assert_eq!(corpus_class_of(&entry("blank", Some(""))), None);
    assert_eq!(
        corpus_class_of(&entry("fw", Some("framework-source"))),
        Some("framework-source")
    );
}

#[test]
fn corpus_group_key_routes_each_class() {
    // Absent => application.
    assert_eq!(corpus_group_key(None), Some("application"));
    // Named classes are their own group.
    assert_eq!(
        corpus_group_key(Some("framework-source")),
        Some("framework-source")
    );
    assert_eq!(
        corpus_group_key(Some("fixture-heavy")),
        Some("fixture-heavy")
    );
    // Duplicates contribute to no group.
    assert_eq!(corpus_group_key(Some("duplicate-of:r-shiny")), None);
}

#[test]
fn corpus_class_report_pools_per_group() {
    // Two application projects pool together; a framework-source project is
    // reported apart. 90/100 vs 30/100 application → 120/(120+? ) etc.
    let mut groups: std::collections::BTreeMap<String, (i64, i64)> =
        std::collections::BTreeMap::new();
    groups.insert("application".into(), (90, 10)); // 90%
    groups.insert("framework-source".into(), (30, 70)); // 30%

    let report = corpus_class_report(&groups);

    // BTreeMap order: "application" before "framework-source".
    assert_eq!(report.len(), 2);
    assert_eq!(report[0], ("application".into(), 90, 10, 90.0));
    assert_eq!(report[1], ("framework-source".into(), 30, 70, 30.0));
}

#[test]
fn corpus_class_report_rounds_two_decimals_and_handles_empty() {
    let mut groups: std::collections::BTreeMap<String, (i64, i64)> =
        std::collections::BTreeMap::new();
    groups.insert("application".into(), (1, 2)); // 33.33%
    groups.insert("fixture-heavy".into(), (0, 0)); // empty → 100.0

    let report = corpus_class_report(&groups);
    let app = report.iter().find(|(c, ..)| c == "application").unwrap();
    assert_eq!(app.3, 33.33);
    let fix = report.iter().find(|(c, ..)| c == "fixture-heavy").unwrap();
    assert_eq!(fix.3, 100.0);
}

#[test]
fn duplicate_class_excluded_from_application_total() {
    // A `duplicate-of:` project must not inflate the application group. Two
    // projects with identical counts — one application, one duplicate — yield
    // an application total of only the application project's counts.
    let mut groups: std::collections::BTreeMap<String, (i64, i64)> =
        std::collections::BTreeMap::new();
    for proj in [
        entry("r-shiny", None),
        entry("rmarkdown-shiny", Some("duplicate-of:r-shiny")),
    ] {
        let class = corpus_class_of(&proj);
        if let Some(group) = corpus_group_key(class) {
            let e = groups.entry(group.to_string()).or_insert((0, 0));
            e.0 += 50;
            e.1 += 50;
        }
    }

    // Only r-shiny landed in "application"; rmarkdown-shiny went nowhere.
    assert_eq!(groups.get("application").copied(), Some((50, 50)));
    assert_eq!(groups.len(), 1, "duplicate created no group");
}
