use super::fixture_support::{run, Fixture};
use super::*;

fn report(language: &str, result: &OracleReport) {
    println!("{language}: {}", serde_json::to_string(result).unwrap());
    assert_eq!(result.counts.not_extracted, 0, "{result:#?}");
    assert_eq!(result.counts.missing_resolution, 0, "{result:#?}");
    assert_eq!(result.counts.unlabelled_observations, 0, "{result:#?}");
    #[derive(serde::Deserialize)]
    struct Snapshot {
        revision: CorpusRevision,
        observations: Vec<Observation>,
    }
    let snapshots: std::collections::BTreeMap<String, Snapshot> =
        serde_json::from_str(include_str!("corpus_snapshot.json")).unwrap();
    let snapshot = &snapshots[language];
    assert_eq!(
        result.revision, snapshot.revision,
        "Fixture/labels changed: review and explicitly rebaseline"
    );
    // Snapshot records observed behavior, NOT ground truth. Both runs are
    // evaluated against the separately authored, revision-pinned labels.
    let labels: Vec<_> = result
        .references
        .iter()
        .map(|r| r.expected.clone())
        .collect();
    let before = evaluate(snapshot.revision, &labels, &snapshot.observations).unwrap();
    for change in compare(&before, result).unwrap() {
        assert!(!change.regressed, "Per-site regression: {change:#?}");
        assert!(
            !(change.retargeted && change.after.verdict == Verdict::Incorrect),
            "Wrong-to-wrong retargeting requires review: {change:#?}"
        );
    }
}

#[test]
fn typescript_sibling_parameters_have_independent_expected_targets() {
    let result = run(
        &[Fixture {
            path: "siblings.ts",
            language: "typescript",
            marked_source: r#"class Alpha {
/*@decl:1:method*/save(): void {}
}
class Beta {
/*@decl:2:method*/save(): void {}
}
function first(value: Alpha) {
/*@ref:11*/value.save();
/*@ref:12*/value.save();
}
function second(value: Beta) {
/*@ref:21*/value.save();
/*@ref:22*/value.save();
}
"#,
        }],
        &[(11, Some(1)), (12, Some(1)), (21, Some(2)), (22, Some(2))],
    )
    .unwrap();
    report("typescript", &result);
    // F1 fixed the two wrong bindings in the pinned historical snapshot.
    // Require all four correct now; do not permit the bug to return.
    assert_eq!(result.counts.correct, 4, "{result:#?}");
}

#[test]
fn javascript_same_line_calls_and_explicit_negative() {
    let result = run(
        &[Fixture {
            path: "calls.js",
            language: "javascript",
            marked_source: r#"/*@decl:1:function*/function helper() {}
function caller() {
/*@ref:11*/helper(); /*@ref:12*/helper();
/*@ref:13*/missing_xyz();
}
"#,
        }],
        &[(11, Some(1)), (12, Some(1)), (13, None)],
    )
    .unwrap();
    report("javascript", &result);
    assert_eq!(result.counts.correct, 2, "{result:#?}");
    assert_eq!(result.counts.correct_unbound, 1, "{result:#?}");
    assert_ne!(
        result.references[0].expected.site,
        result.references[1].expected.site
    );
}

#[test]
fn python_same_line_calls() {
    let result = run(&[Fixture {
        path: "calls.py", language: "python", marked_source: "/*@decl:1:function*/def helper():\n    pass\n\ndef caller():\n    /*@ref:11*/helper(); /*@ref:12*/helper()\n",
    }], &[(11, Some(1)), (12, Some(1))]).unwrap();
    report("python", &result);
    assert_eq!(result.counts.correct, 2, "{result:#?}");
}

#[test]
fn csharp_same_line_calls() {
    let result = run(
        &[Fixture {
            path: "Calls.cs",
            language: "csharp",
            marked_source: r#"class Demo {
/*@decl:1:method*/public static void Helper() {}
public static void Caller() {
/*@ref:11*/Helper(); /*@ref:12*/Helper();
}
}
"#,
        }],
        &[(11, Some(1)), (12, Some(1))],
    )
    .unwrap();
    report("csharp", &result);
    assert_eq!(result.counts.correct, 2, "{result:#?}");
}
