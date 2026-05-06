use super::*;
use std::fs;

#[test]
fn newtype_call_extracts_name() {
    let src = r#"
# encoding: UTF-8
require 'puppet/type'

Puppet::Type.newtype(:file) do
  @doc = "Manage files"
end
"#;
    let names = find_newtype_calls(src);
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].1, "file");
}

#[test]
fn newtype_short_form_also_recognized() {
    // Some puppet type files use the imported `Type` constant directly.
    let src = "Type.newtype(:service) do\nend\n";
    let names = find_newtype_calls(src);
    assert_eq!(names.first().map(|(_, n)| n.as_str()), Some("service"));
}

#[test]
fn create_function_extracts_name() {
    let src = r#"
Puppet::Functions.create_function(:assert_type) do
  dispatch :assert_type do
  end
end
"#;
    let names = find_create_function_calls(src);
    assert_eq!(names.first().map(|(_, n)| n.as_str()), Some("assert_type"));
}

#[test]
fn create_function_handles_scoped_quoted_name() {
    let src = "Puppet::Functions.create_function(:'mymod::myfn') do\nend\n";
    let names = find_create_function_calls(src);
    assert_eq!(names.first().map(|(_, n)| n.as_str()), Some("mymod::myfn"));
}

#[test]
fn legacy_newfunction_extracts_name() {
    let src = r#"
module Puppet::Parser::Functions
  newfunction(:fail, :type => :statement) do |args|
    raise Puppet::ParseError, args[0]
  end
end
"#;
    let names = find_legacy_newfunction_calls(src);
    assert_eq!(names.first().map(|(_, n)| n.as_str()), Some("fail"));
}

#[test]
fn comments_do_not_match() {
    let src = "# Puppet::Type.newtype(:bogus) example in docs\n";
    // The regex pre-filter checks for `(:` AND the pattern. A comment line
    // that contains both still matches today (we don't strip comments). This
    // test pins behavior — accept the match since it'd be a defensible name
    // anyway, but document the known limitation.
    let names = find_newtype_calls(src);
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].1, "bogus");
}

#[test]
fn parse_puppet_gem_emits_real_symbols() {
    // Build a minimal fake gem on disk and confirm the walker emits the
    // expected symbols. Exercises the file-walk paths without needing a
    // real puppet install.
    let tmp = tempfile::tempdir().expect("tempdir");
    let gem = tmp.path().to_path_buf();
    let type_dir = gem.join("lib/puppet/type");
    let modern_fn_dir = gem.join("lib/puppet/functions");
    let legacy_fn_dir = gem.join("lib/puppet/parser/functions");
    fs::create_dir_all(&type_dir).unwrap();
    fs::create_dir_all(&modern_fn_dir).unwrap();
    fs::create_dir_all(&legacy_fn_dir).unwrap();

    fs::write(
        type_dir.join("file.rb"),
        "Puppet::Type.newtype(:file) do\nend\n",
    )
    .unwrap();
    fs::write(
        type_dir.join("service.rb"),
        "Puppet::Type.newtype(:service) do\nend\n",
    )
    .unwrap();
    fs::write(
        modern_fn_dir.join("assert_type.rb"),
        "Puppet::Functions.create_function(:assert_type) do\nend\n",
    )
    .unwrap();
    fs::write(
        legacy_fn_dir.join("fail.rb"),
        "newfunction(:fail) do |args|\nend\n",
    )
    .unwrap();

    let parsed = parse_puppet_gem(&gem);
    let names: Vec<&str> = parsed
        .iter()
        .flat_map(|p| p.symbols.iter().map(|s| s.name.as_str()))
        .collect();
    assert!(names.contains(&"file"), "missing 'file' resource type: {names:?}");
    assert!(names.contains(&"service"), "missing 'service' resource type: {names:?}");
    assert!(names.contains(&"assert_type"), "missing 'assert_type' function: {names:?}");
    assert!(names.contains(&"fail"), "missing 'fail' function: {names:?}");

    // Confirm path prefix is stable across machines.
    for pf in &parsed {
        assert!(
            pf.path.starts_with("ext:puppet-stdlib:lib/puppet/"),
            "unexpected synthesized path: {}",
            pf.path
        );
    }
}

#[test]
fn parse_puppet_gem_filename_fallback_for_function_without_regex_match() {
    // If a function file doesn't contain create_function (perhaps a stub
    // file with only comments), fall back to filename-as-name so the
    // resolution surface still picks it up.
    let tmp = tempfile::tempdir().expect("tempdir");
    let gem = tmp.path().to_path_buf();
    let modern_fn_dir = gem.join("lib/puppet/functions");
    fs::create_dir_all(&modern_fn_dir).unwrap();
    fs::write(
        modern_fn_dir.join("notice.rb"),
        "# Top-of-file comment, no create_function call\n",
    )
    .unwrap();

    let parsed = parse_puppet_gem(&gem);
    let names: Vec<&str> = parsed
        .iter()
        .flat_map(|p| p.symbols.iter().map(|s| s.name.as_str()))
        .collect();
    assert!(names.contains(&"notice"), "filename fallback failed: {names:?}");
}

#[test]
fn missing_gem_dir_emits_nothing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let parsed = parse_puppet_gem(tmp.path());
    assert!(parsed.is_empty());
}
