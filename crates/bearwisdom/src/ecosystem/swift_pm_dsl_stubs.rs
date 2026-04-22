// =============================================================================
// ecosystem/swift_pm_dsl_stubs.rs — Swift Package Manager DSL synthetic stubs
//
// Package.swift is a Swift DSL built on the PackageDescription library:
//
//     import PackageDescription
//     let package = Package(
//         name: "MyPackage",
//         products: [.library(name: "Lib", targets: ["Lib"])],
//         dependencies: [.package(url: "...", from: "1.0.0")],
//         targets: [.target(name: "Lib"), .testTarget(name: "LibTests")]
//     )
//
// The Swift extractor emits the dot-shorthand factory calls (`.library`,
// `.target`, `.testTarget`, `.package`, `.product`) and every argument label
// (`dependencies:`, `targets:`, `products:`, …) as bare TypeRefs at
// declaration level. Those refs have no matching symbol in the Swift index
// because PackageDescription is an SDK-vended module that doesn't live in
// the project tree.
//
// This synthesizes PackageDescription's top-level types + factory methods
// so the Swift resolver's by_name lookup step resolves those bare refs.
// Pattern matches laravel_stubs.rs / phoenix_stubs.rs.
//
// Activation: Swift language present.
// =============================================================================

use std::path::Path;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("swift-pm-dsl-stubs");
const LEGACY_ECOSYSTEM_TAG: &str = "swift-pm-dsl-stubs";
const LANGUAGES: &[&str] = &["swift"];

// =============================================================================
// Symbol inventories
// =============================================================================

/// Top-level PackageDescription types. Referenced as `Package(...)`,
/// `Product.library(...)`, `Target.target(...)`, etc. Synthesized as
/// Struct so the kind check in the resolver permits type_ref resolution.
const TYPES: &[&str] = &[
    "Package",
    "Product",
    "Target",
    "SwiftSetting",
    "CSetting",
    "CXXSetting",
    "LinkerSetting",
    "SwiftVersion",
    "LanguageMode",
    "SupportedPlatform",
    "PackagePlugin",
    "BuildConfiguration",
    "TargetDependencyCondition",
];

/// Dot-shorthand factory methods. In Swift, `.library(...)` inside a
/// `[Product]` literal is sugar for `Product.library(...)`. The extractor
/// emits these as bare TypeRefs; synthesizing them as free-standing
/// Functions makes by_name resolution succeed.
const FACTORIES: &[&str] = &[
    // Product variants
    "library", "executable", "plugin",
    // Target variants
    "target", "testTarget", "binaryTarget", "systemLibrary", "macro",
    // Dependency variants (both `.package(url:)` and `.product(name:package:)`)
    "package", "product", "byName",
    // Platform variants — `.iOS(.v18)`, `.macOS(.v15)`, etc. — these are
    // static methods on SupportedPlatform. The `iOS`/`macOS`/… method names
    // are Swift identifiers starting with lowercase-then-upper; included
    // because they appear as unresolved refs from Package.swift files.
    "iOS", "macOS", "tvOS", "watchOS", "visionOS", "macCatalyst", "driverKit",
    // Capability variants for plugin targets
    "buildTool", "command",
    // Common SwiftSetting / CSetting helpers
    "define", "enableExperimentalFeature", "enableUpcomingFeature",
    "unsafeFlags", "swiftLanguageMode", "interoperabilityMode",
    "headerSearchPath", "linkedLibrary", "linkedFramework",
    "defaultIsolation",
];

/// Argument labels the Swift extractor leaks as bare TypeRefs from
/// Package.swift files. These aren't real refs — they're parameter names
/// in Swift call syntax (`Package(dependencies: [...])`). Still, making
/// them resolve eliminates the noise. Scoped to labels that appear ONLY
/// in Package.swift manifest files; generic labels like `name`, `path`,
/// `url`, `type` are deliberately excluded to avoid false-positive
/// resolutions in regular Swift code.
const ARGUMENT_LABELS: &[&str] = &[
    "dependencies", "targets", "products", "platforms",
    "swiftLanguageModes", "cLanguageStandard", "cxxLanguageStandard",
    "swiftSettings", "cSettings", "cxxSettings", "linkerSettings",
    "packageAccess", "publicHeadersPath", "pkgConfig", "providers",
    "capability", "checksum", "plugins", "resources",
    "defaultLocalization", "traits",
];

// =============================================================================
// Synthesis
// =============================================================================

fn sym(name: &str, qualified_name: &str, kind: SymbolKind, signature: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qualified_name.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(signature.to_string()),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

fn synthesize_file() -> ParsedFile {
    let mut symbols = Vec::new();

    // The namespace itself.
    symbols.push(sym(
        "PackageDescription",
        "PackageDescription",
        SymbolKind::Namespace,
        "import PackageDescription",
    ));

    // Note: all synthetic DSL symbols are emitted as `Struct` rather than
    // Function/Property because the Swift extractor categorises every
    // Package.swift-level identifier as `EdgeKind::TypeRef`, and the Swift
    // resolver's kind_compatible check only permits class/struct/interface/
    // enum/type_alias/namespace for TypeRef. The kind is a convenient lie —
    // these symbols exist solely to satisfy by_name lookup on unresolved
    // type_ref entries.
    for t in TYPES {
        symbols.push(sym(
            t,
            &format!("PackageDescription.{t}"),
            SymbolKind::Struct,
            &format!("public struct {t}"),
        ));
    }

    for f in FACTORIES {
        symbols.push(sym(
            f,
            &format!("PackageDescription.{f}"),
            SymbolKind::Struct,
            &format!("public static func {f}"),
        ));
    }

    for label in ARGUMENT_LABELS {
        symbols.push(sym(
            label,
            &format!("PackageDescription.{label}"),
            SymbolKind::Struct,
            &format!("public var {label}"),
        ));
    }

    let n_syms = symbols.len();
    ParsedFile {
        path: "ext:swift-pm-dsl-stubs:PackageDescription.swift".to_string(),
        language: "swift".to_string(),
        content_hash: format!("swift-pm-dsl-stubs-{n_syms}"),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: vec![None; n_syms],
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: vec![false; n_syms],
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    }
}

// =============================================================================
// Synthetic dep root + Ecosystem impl
// =============================================================================

fn synthetic_dep_root() -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "swift-pm-dsl-stubs".to_string(),
        version: String::new(),
        root: std::path::PathBuf::from("ext:swift-pm-dsl-stubs"),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

pub struct SwiftPmDslStubsEcosystem;

impl Ecosystem for SwiftPmDslStubsEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("swift")
    }

    fn locate_roots(&self, _ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        vec![synthetic_dep_root()]
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn parse_metadata_only(&self, _dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        Some(vec![synthesize_file()])
    }
}

impl ExternalSourceLocator for SwiftPmDslStubsEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        vec![synthetic_dep_root()]
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        Some(vec![synthesize_file()])
    }
}

#[cfg(test)]
#[path = "swift_pm_dsl_stubs_tests.rs"]
mod tests;
