use super::*;

#[test]
fn lexical_normalize_strips_dot_segments() {
    let p = lexical_normalize(Path::new("/repo/.github/workflows/./../actions/setup"));
    assert_eq!(p.to_string_lossy().replace('\\', "/"), "/repo/.github/actions/setup");
}

#[test]
fn reusable_workflow_target_is_used_verbatim() {
    let cands = path_candidates(
        Path::new("/repo/.github/workflows"),
        "./reusable.yml",
    );
    let strs: Vec<String> = cands
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    // For a `.yml`-suffixed target the candidate is the file itself.
    assert!(strs.contains(&"/repo/.github/workflows/reusable.yml".to_string()));
    // No `action.yml` probing for a workflow ref.
    assert!(!strs.iter().any(|s| s.ends_with("/reusable.yml/action.yml")));
}

#[test]
fn composite_action_target_probes_action_yml() {
    let cands = path_candidates(
        Path::new("/repo/.github/workflows"),
        "../actions/setup",
    );
    let strs: Vec<String> = cands
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(strs.contains(&"/repo/.github/actions/setup/action.yml".to_string()));
    assert!(strs.contains(&"/repo/.github/actions/setup/action.yaml".to_string()));
}

#[test]
fn composite_action_falls_back_to_yml_alongside_directory() {
    // A few projects ship local helpers as `<dir>/<name>.yml` instead
    // of `<dir>/<name>/action.yml` — the candidate list covers both.
    let cands = path_candidates(
        Path::new("/repo/.github/workflows"),
        "./helpers/check",
    );
    let strs: Vec<String> = cands
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(strs.contains(&"/repo/.github/workflows/helpers/check.yml".to_string()));
}
