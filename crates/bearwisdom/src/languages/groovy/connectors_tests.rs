use super::*;

#[test]
fn groovy_get_mapping_scan() {
    let src = r#"@GetMapping("/api/users")\ndef list() {}"#;
    let re_method = build_method_mapping_regex();
    let re_request = build_request_mapping_regex();
    let out = scan_groovy_file(src, &re_method, &re_request);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].path, "/api/users");
    assert_eq!(out[0].http_method, "GET");
}

#[test]
fn groovy_class_level_request_mapping_prefixes_method_path() {
    let src = r#"@RequestMapping("/api")
class UsersController {
    @GetMapping("/users") def list() {}
}"#;
    let re_method = build_method_mapping_regex();
    let re_request = build_request_mapping_regex();
    let out = scan_groovy_file(src, &re_method, &re_request);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].path, "/api/users");
}
