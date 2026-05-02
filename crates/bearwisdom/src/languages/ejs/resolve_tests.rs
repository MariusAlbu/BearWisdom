use super::*;

#[test]
fn lexical_normalize_strips_dot_segments() {
    let p = lexical_normalize(Path::new("/a/./b/../c/file.ejs"));
    assert_eq!(p.to_string_lossy().replace('\\', "/"), "/a/c/file.ejs");
}

#[test]
fn path_candidates_appends_ejs_when_missing() {
    let cands = path_candidates(Path::new("/views"), "./partials/header");
    let strs: Vec<String> = cands
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(strs.contains(&"/views/partials/header".to_string()));
    assert!(strs.contains(&"/views/partials/header.ejs".to_string()));
    assert!(strs.contains(&"/views/partials/header.html".to_string()));
    assert!(strs.contains(&"/views/partials/header/index.ejs".to_string()));
}

#[test]
fn path_candidates_keeps_explicit_ejs_extension() {
    let cands = path_candidates(Path::new("/views"), "./layout.ejs");
    // The bare candidate is included; we don't pile on more `.ejs` suffixes.
    let strs: Vec<String> = cands
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(strs.contains(&"/views/layout.ejs".to_string()));
    assert!(!strs.iter().any(|s| s.ends_with("/layout.ejs.ejs")));
}

#[test]
fn path_candidates_ascends_with_dotdot() {
    let cands = path_candidates(Path::new("/views/admin"), "../partials/header");
    let strs: Vec<String> = cands
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(strs.contains(&"/views/partials/header.ejs".to_string()));
}
