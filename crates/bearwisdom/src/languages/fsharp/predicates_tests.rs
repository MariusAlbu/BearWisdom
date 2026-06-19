// Tests for F# external-namespace classification after the third-party-name
// purge: NuGet-declared packages classify external, platform roots stay, and
// the purged framework namespaces are gone from the fallback.

use super::predicates::{is_external_namespace_fallback, is_manifest_external_namespace};
use crate::ecosystem::manifest::{ManifestData, ManifestKind};
use crate::indexer::project_context::ProjectContext;

fn ctx_with_nuget(deps: &[&str]) -> ProjectContext {
    let mut ctx = ProjectContext::default();
    let mut nuget = ManifestData::default();
    for d in deps {
        nuget.dependencies.insert((*d).to_string());
    }
    ctx.manifests.insert(ManifestKind::NuGet, nuget);
    ctx
}

#[test]
fn nuget_declared_package_classifies_external() {
    let ctx = ctx_with_nuget(&["Serilog", "Giraffe"]);
    assert!(is_manifest_external_namespace(&ctx, "Serilog"));
    assert!(is_manifest_external_namespace(&ctx, "Giraffe.Core"));
}

#[test]
fn package_without_manifest_is_not_external() {
    // No NuGet dependency declares Serilog/Giraffe — purged from the fallback,
    // so they must NOT classify external (honest-unresolved).
    assert!(!is_external_namespace_fallback("Serilog"));
    assert!(!is_external_namespace_fallback("Giraffe"));
    assert!(!is_external_namespace_fallback("Expecto"));
    assert!(!is_external_namespace_fallback("Moq.Mock"));
}

#[test]
fn platform_roots_still_classify() {
    // BCL + Microsoft + FSharp.Core root stay closed-set, no manifest needed.
    assert!(is_external_namespace_fallback("System"));
    assert!(is_external_namespace_fallback("System.Collections.Generic"));
    assert!(is_external_namespace_fallback("Microsoft.Extensions.Logging"));
    assert!(is_external_namespace_fallback("FSharp.Core"));
}

#[test]
fn purged_thirdparty_namespaces_absent_from_fallback() {
    for name in [
        "Newtonsoft",
        "Serilog",
        "NLog",
        "AutoMapper",
        "FluentValidation",
        "MediatR",
        "Polly",
        "NSubstitute",
        "Moq",
        "FakeItEasy",
        "Xunit",
        "NUnit",
        "Giraffe",
        "Saturn",
        "Suave",
        "Fable",
        "Elmish",
        "Expecto",
        "FsCheck",
        "FParsec",
        "Argu",
    ] {
        assert!(
            !is_external_namespace_fallback(name),
            "third-party namespace `{name}` survived the purge"
        );
    }
}
