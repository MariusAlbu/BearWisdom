use super::*;

#[test]
fn extracts_pinned_packages() {
    let r = parse_requirements("numpy==1.20\nrequests>=2.28\nflask\n");
    assert!(r.contains(&"numpy".to_string()));
    assert!(r.contains(&"requests".to_string()));
    assert!(r.contains(&"flask".to_string()));
}

#[test]
fn skips_comments_and_options() {
    let r = parse_requirements("# comment\n-r dev.txt\n--index-url https://x\nnumpy\n");
    assert_eq!(r, vec!["numpy".to_string()]);
}

#[test]
fn handles_extras() {
    let r = parse_requirements("uvicorn[standard]>=0.20\n");
    assert_eq!(r, vec!["uvicorn".to_string()]);
}

#[test]
fn handles_environment_markers() {
    let r = parse_requirements("black ; python_version >= '3.7'\n");
    assert_eq!(r, vec!["black".to_string()]);
}

#[test]
fn extracts_editable_install_egg_name() {
    let r = parse_requirements("-e git+https://github.com/foo/bar@v1#egg=bar\n");
    assert_eq!(r, vec!["bar".to_string()]);
}

#[test]
fn normalises_pep503_names() {
    let r = parse_requirements("Django_Rest_Framework\nzope.interface\n");
    assert!(r.contains(&"django-rest-framework".to_string()));
    assert!(r.contains(&"zope-interface".to_string()));
}
