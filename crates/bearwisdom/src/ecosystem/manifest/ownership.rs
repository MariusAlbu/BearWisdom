//! Ecosystem-owned manifest-kind metadata used by activation.
//!
//! Project context asks an ecosystem for the kinds it owns; it never selects
//! an ecosystem id or interprets a manifest family itself.

use super::ManifestKind;
use crate::ecosystem::EcosystemId;

pub(crate) fn kinds_for(id: EcosystemId) -> &'static [ManifestKind] {
    match id.as_str() {
        "maven" => &[
            ManifestKind::Maven,
            ManifestKind::Gradle,
            ManifestKind::Sbt,
            ManifestKind::Clojure,
        ],
        "npm" => &[ManifestKind::Npm],
        "pypi" => &[ManifestKind::PyProject],
        "cargo" => &[ManifestKind::Cargo],
        "hex" => &[ManifestKind::Mix, ManifestKind::Gleam],
        "nuget" => &[ManifestKind::NuGet],
        "spm" => &[ManifestKind::SwiftPM],
        "go-mod" => &[ManifestKind::GoMod],
        "rubygems" => &[ManifestKind::Gemfile],
        "composer" => &[ManifestKind::Composer],
        "cran" => &[ManifestKind::Description],
        "pub" => &[ManifestKind::Pubspec],
        "opam" => &[ManifestKind::Opam],
        "luarocks" => &[ManifestKind::Rockspec],
        "zig-pkg" => &[ManifestKind::ZigZon],
        "puppet-forge" => &[ManifestKind::Puppet],
        "cabal" => &[ManifestKind::Cabal],
        "cpan" => &[ManifestKind::Cpan],
        "nimble" => &[ManifestKind::Nimble],
        "alire" => &[ManifestKind::Alire],
        "psgallery" => &[ManifestKind::Psd1],
        "bazel-central-registry" => &[ManifestKind::ModuleBazel],
        "tf-registry" => &[ManifestKind::Terraform],
        _ => &[],
    }
}
