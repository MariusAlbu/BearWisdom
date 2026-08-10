use super::select_nearest_version;

fn pick(candidates: &[&str], requested: Option<&str>) -> Option<String> {
    let owned: Vec<String> = candidates.iter().map(|s| s.to_string()).collect();
    select_nearest_version(&owned, requested)
}

#[test]
fn same_major_nearest_above_requested() {
    let picked = pick(&["8.0.11", "9.0.3", "10.0.5"], Some("8.0.8"));
    assert_eq!(picked.as_deref(), Some("8.0.11"));
}

#[test]
fn lowest_major_above_requested_when_no_same_major() {
    let picked = pick(&["9.0.3", "10.0.5"], Some("8.0.8"));
    assert_eq!(picked.as_deref(), Some("9.0.3"));
}

#[test]
fn four_part_version_parses_and_orders() {
    let picked = pick(&["1.0.0", "1.0.0.1"], None);
    assert_eq!(picked.as_deref(), Some("1.0.0.1"));
}

#[test]
fn exact_match_wins_over_prerelease() {
    let picked = pick(&["8.0.0-rc.2", "8.0.0"], Some("8.0.0"));
    assert_eq!(picked.as_deref(), Some("8.0.0"));
}

#[test]
fn no_requested_version_picks_highest_not_lexical() {
    let picked = pick(&["2.0.0", "10.0.0", "9.0.0"], None);
    assert_eq!(picked.as_deref(), Some("10.0.0"));
}

#[test]
fn highest_available_when_all_candidates_below_requested_major() {
    let picked = pick(&["6.0.2", "7.0.1"], Some("8.0.0"));
    assert_eq!(picked.as_deref(), Some("7.0.1"));
}

#[test]
fn unparseable_dirs_never_win_over_a_parseable_one() {
    let picked = pick(&["latest", "1.2.3"], None);
    assert_eq!(picked.as_deref(), Some("1.2.3"));
}
