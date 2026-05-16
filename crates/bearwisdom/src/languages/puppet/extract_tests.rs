use super::_test_collect_local_var_scopes;
use tree_sitter::Parser;

fn puppet_lang() -> tree_sitter::Language {
    tree_sitter_puppet::LANGUAGE.into()
}

fn parse_and_collect(src: &str) -> Vec<(String, u32, u32)> {
    let mut parser = Parser::new();
    parser.set_language(&puppet_lang()).unwrap();
    let tree = parser.parse(src, None).unwrap();
    let mut out = Vec::new();
    _test_collect_local_var_scopes(tree.root_node(), src, &mut out);
    out
}

#[test]
fn class_param_scoped_to_class_body() {
    let src = "class foo ($directory = '/tmp') {\n  file { $directory: }\n}\n";
    let scopes = parse_and_collect(src);
    assert!(
        scopes.iter().any(|(n, _, _)| n == "$directory"),
        "expected $directory in scopes: {scopes:?}"
    );
}

#[test]
fn lambda_block_variable_scoped_to_lambda() {
    let src = "class foo {\n  $xs.each |$x| {\n    notify { $x: }\n  }\n}\n";
    let scopes = parse_and_collect(src);
    assert!(
        scopes.iter().any(|(n, _, _)| n == "$x"),
        "expected $x from lambda: {scopes:?}"
    );
}

#[test]
fn typed_lambda_variable_captured() {
    // Puppet lambda with a type annotation: `|Hash $directory|`
    let src = "class foo {\n  $xs.each |Hash $directory| {\n    notify { $directory: }\n  }\n}\n";
    let scopes = parse_and_collect(src);
    assert!(
        scopes.iter().any(|(n, _, _)| n == "$directory"),
        "typed lambda var should be captured: {scopes:?}"
    );
}

#[test]
fn out_of_scope_variable_not_captured_wrongly() {
    // `$other` is not a parameter anywhere; should NOT appear.
    let src = "class foo ($directory) { notify { $other: } }\n";
    let scopes = parse_and_collect(src);
    assert!(
        !scopes.iter().any(|(n, _, _)| n == "$other"),
        "$other must not be captured: {scopes:?}"
    );
}

#[test]
fn body_assignment_in_define_captured() {
    // Mirrors the puppet-apache pattern:
    //   define apache::mod (...) {
    //     $mod_libs = $apache::mod_libs
    //     if $mod in $mod_libs { $x = $mod_libs[$mod] }
    //   }
    // `$mod_libs` is a body-local assignment; its subscript use
    // `$mod_libs[$mod]` is emitted as a resource_reference ref and
    // should be suppressed.
    let src = concat!(
        "define apache::mod (\n",
        "  Optional[String] $package = undef,\n",
        ") {\n",
        "  $mod_libs = $apache::mod_libs\n",
        "  if $mod in $mod_libs {\n",
        "    $_lib = $mod_libs[$mod]\n",
        "  }\n",
        "}\n",
    );
    let scopes = parse_and_collect(src);
    assert!(
        scopes.iter().any(|(n, _, _)| n == "$mod_libs"),
        "body-assignment $mod_libs should be captured: {scopes:?}"
    );
}

#[test]
fn body_assignment_in_nested_if_captured() {
    // Assignments inside if/elsif sub-blocks are also function-scoped in
    // Puppet, so they too should suppress refs throughout the declaration.
    let src = concat!(
        "class apache::mod::php (\n",
        "  Optional[String] $package_name = undef,\n",
        ") {\n",
        "  $mod_packages = $apache::mod_packages\n",
        "  if $package_name {\n",
        "    $_pkg = $package_name\n",
        "  } elsif $mod in $mod_packages {\n",
        "    $_pkg = $mod_packages[$mod]\n",
        "  }\n",
        "}\n",
    );
    let scopes = parse_and_collect(src);
    assert!(
        scopes.iter().any(|(n, _, _)| n == "$mod_packages"),
        "body-assignment $mod_packages should be captured: {scopes:?}"
    );
    // $package_name is a formal parameter — should also be present
    assert!(
        scopes.iter().any(|(n, _, _)| n == "$package_name"),
        "formal param $package_name should be captured: {scopes:?}"
    );
}
