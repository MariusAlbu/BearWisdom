// =============================================================================
// ecosystem/puppet_forge_tests.rs — sibling tests for puppet_forge.rs
// =============================================================================

use super::*;
use crate::ecosystem::manifest::ManifestReader;

#[test]
fn ecosystem_identity() {
    let e = PuppetForgeEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Package);
    assert_eq!(Ecosystem::languages(&e), &["puppet"]);
}

#[test]
fn metadata_json_deps_parsed() {
    let json = r#"{
  "name": "myorg-mymodule",
  "version": "1.0.0",
  "dependencies": [
    {"name": "puppetlabs-stdlib", "version_requirement": ">= 4.13.1"},
    {"name": "puppetlabs/apache", "version_requirement": ">= 5.0.0"}
  ]
}"#;
    let mut deps = Vec::new();
    _test_parse_metadata_json_deps(json, &mut deps);
    assert!(deps.contains(&"puppetlabs-stdlib".to_string()));
    assert!(deps.contains(&"puppetlabs/apache".to_string()));
}

#[test]
fn puppetfile_deps_parsed() {
    let pf = r#"
# r10k Puppetfile
forge "https://forgeapi.puppet.com"

mod 'puppetlabs/stdlib', '>= 4.13.1'
mod 'puppetlabs-apache', '5.0.0'
mod "camptocamp/systemd"
"#;
    let mut deps = Vec::new();
    _test_parse_puppetfile_deps(pf, &mut deps);
    assert!(deps.contains(&"puppetlabs/stdlib".to_string()));
    assert!(deps.contains(&"puppetlabs-apache".to_string()));
    assert!(deps.contains(&"camptocamp/systemd".to_string()));
}

#[test]
fn extract_quoted_string_handles_both_quote_styles() {
    assert_eq!(_test_extract_quoted_string(r#""puppetlabs/stdlib""#), "puppetlabs/stdlib");
    assert_eq!(_test_extract_quoted_string("'camptocamp/systemd'"), "camptocamp/systemd");
}

#[test]
fn scan_puppet_header_extracts_names() {
    let src = r#"
class apache (
  $port = 80,
) {
}

define apache::vhost (
  $docroot,
) {
}

function apache::params() {
}
"#;
    let names = scan_puppet_header(src);
    assert!(names.contains(&"apache".to_string()));
    assert!(names.contains(&"vhost".to_string()));
    assert!(names.contains(&"params".to_string()));
}

#[test]
fn scan_puppet_header_skips_comments() {
    let src = "# class notaclass {\nclass realclass {\n";
    let names = scan_puppet_header(src);
    assert_eq!(names, vec!["realclass"]);
}

#[test]
fn collect_declared_modules_from_temp_dir() {
    let tmp = std::env::temp_dir().join("bw-puppet-forge-test");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(
        tmp.join("metadata.json"),
        "{\n  \"name\": \"myorg-mymod\",\n  \"version\": \"1.0.0\",\n  \"dependencies\": [\n    {\"name\": \"puppetlabs-stdlib\", \"version_requirement\": \">=4.13.1\"}\n  ]\n}",
    ).unwrap();
    let deps = collect_declared_modules(&tmp);
    assert!(deps.contains(&"puppetlabs-stdlib".to_string()));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn manifest_reader_emits_bare_module_names() {
    let tmp = std::env::temp_dir().join("bw-puppet-manifest-bare");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(
        tmp.join("metadata.json"),
        "{\n  \"name\": \"myorg-mymod\",\n  \"dependencies\": [\n    {\"name\": \"puppetlabs-stdlib\"},\n    {\"name\": \"puppetlabs/apache\"}\n  ]\n}",
    ).unwrap();
    std::fs::write(
        tmp.join("Puppetfile"),
        "mod 'camptocamp/systemd', '>= 1.0'\n",
    ).unwrap();

    let data = PuppetMetadataManifest.read(&tmp).expect("manifest data present");
    assert!(data.dependencies.contains("stdlib"));
    assert!(data.dependencies.contains("apache"));
    assert!(data.dependencies.contains("systemd"));
    // Slugs must NOT leak through — the resolver compares against bare prefixes.
    assert!(!data.dependencies.contains("puppetlabs-stdlib"));
    assert!(!data.dependencies.contains("puppetlabs/apache"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn manifest_reader_returns_none_when_no_manifests() {
    let tmp = std::env::temp_dir().join("bw-puppet-manifest-empty");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(PuppetMetadataManifest.read(&tmp).is_none());
    let _ = std::fs::remove_dir_all(&tmp);
}
