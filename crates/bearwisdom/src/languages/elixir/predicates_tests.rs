// Tests for Elixir external-module classification after the Hex-package purge.
// The stdlib/OTP runtime set stays in `is_external_elixir_module`; Hex-package
// modules are classified from `mix.exs` deps at the resolver hooks via
// `is_mix_dep_match` (CamelCase module root ↔ snake_case dep atom).

use super::predicates::{is_external_elixir_module, is_mix_dep_match};
use std::collections::HashSet;

fn deps(names: &[&str]) -> HashSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn mix_declared_dep_classifies_external() {
    // mix.exs stores snake_case atoms; the module root is CamelCase.
    let d = deps(&["phoenix", "ecto_sql", "jason"]);
    assert!(is_mix_dep_match("Phoenix", &d));
    assert!(is_mix_dep_match("Ecto", &d)); // ecto_sql → first segment "ecto"
    assert!(is_mix_dep_match("Jason", &d));
}

#[test]
fn dep_without_manifest_is_not_external() {
    // Empty mix deps + purged predicate → Hex module is not classified external.
    let d = deps(&[]);
    assert!(!is_mix_dep_match("Phoenix", &d));
    assert!(!is_mix_dep_match("Ecto", &d));
    assert!(!is_external_elixir_module("Phoenix"));
    assert!(!is_external_elixir_module("Ecto.Changeset"));
    assert!(!is_external_elixir_module("Oban"));
}

#[test]
fn stdlib_and_otp_still_classify() {
    // Elixir stdlib (CamelCase) + OTP (lowercase atoms) + toolchain modules.
    assert!(is_external_elixir_module("Enum"));
    assert!(is_external_elixir_module("Map.put"));
    assert!(is_external_elixir_module("String"));
    assert!(is_external_elixir_module("Kernel"));
    assert!(is_external_elixir_module("GenServer"));
    assert!(is_external_elixir_module("ExUnit"));
    assert!(is_external_elixir_module("Mix"));
    // OTP lowercase atoms.
    assert!(is_external_elixir_module("erlang"));
    assert!(is_external_elixir_module("lists"));
    assert!(is_external_elixir_module("gen_server"));
}

#[test]
fn purged_hex_roots_absent_from_predicate() {
    for name in [
        "Phoenix",
        "Ecto",
        "Plug",
        "Tesla",
        "Jason",
        "Poison",
        "Swoosh",
        "Oban",
        "Broadway",
        "Absinthe",
        "Ash",
        "Finch",
        "Req",
        "Mint",
        "Bandit",
        "Cowboy",
        "HTTPoison",
        "Postgrex",
        "Redix",
        "Floki",
        "Mox",
        "Faker",
        "Credo",
        "Gettext",
        "Timex",
        "Decimal",
        "Bamboo",
        "Guardian",
        "Pow",
        "ExAws",
        "Sentry",
        "Telemetry",
    ] {
        assert!(
            !is_external_elixir_module(name),
            "Hex-package root `{name}` survived the purge"
        );
    }
}
