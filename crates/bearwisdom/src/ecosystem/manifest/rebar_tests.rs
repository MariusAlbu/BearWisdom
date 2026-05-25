use super::*;

#[test]
fn parses_simple_dep_list() {
    let config = r#"
{deps, [
    {jsx, "3.1.0"},
    {cowboy, "2.10.0"},
    cowlib
]}.
"#;
    let deps = parse_rebar_deps(config);
    assert!(deps.contains(&"jsx".to_string()));
    assert!(deps.contains(&"cowboy".to_string()));
    assert!(deps.contains(&"cowlib".to_string()));
}

#[test]
fn handles_git_dep() {
    let config = r#"
{deps, [
    {cowboy, {git, "https://github.com/ninenines/cowboy", {tag, "2.10"}}}
]}.
"#;
    let deps = parse_rebar_deps(config);
    assert_eq!(deps, vec!["cowboy".to_string()]);
}

#[test]
fn empty_on_no_deps_block() {
    let deps = parse_rebar_deps("{erl_opts, [debug_info]}.");
    assert!(deps.is_empty());
}

#[test]
fn handles_comments() {
    let config = r#"
{deps, [
    {jsx, "3.1.0"}  %% JSON library
    , cowlib       %% HTTP cowlib
]}.
"#;
    let deps = parse_rebar_deps(config);
    assert!(deps.contains(&"jsx".to_string()));
    assert!(deps.contains(&"cowlib".to_string()));
}
