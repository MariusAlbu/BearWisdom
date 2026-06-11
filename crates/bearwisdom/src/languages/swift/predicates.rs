// =============================================================================
// swift/predicates.rs — Swift builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class" | "struct"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "struct" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "struct" | "interface" | "enum" | "type_alias" | "namespace"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "struct"),
        _ => true,
    }
}

/// Apple platform-SDK framework modules — the frameworks bundled with Xcode /
/// the Swift toolchain (`Swift`, `Dispatch`, `Darwin`, `XCTest`) that every
/// project links without declaring a dependency. This is the closed platform
/// set, not a dependency list. SwiftPM packages (Vapor, Alamofire, RxSwift, …)
/// are classified from `Package.swift` at the resolver hooks, never here.
const PLATFORM_MODULES: &[&str] = &[
    "Foundation",
    "UIKit",
    "SwiftUI",
    "Combine",
    "CoreData",
    "CoreGraphics",
    "CoreLocation",
    "CoreMotion",
    "CoreBluetooth",
    "CoreNFC",
    "CoreImage",
    "ARKit",
    "SceneKit",
    "SpriteKit",
    "GameKit",
    "MapKit",
    "AVFoundation",
    "AVKit",
    "AppKit",
    "XCTest",
    "Swift",
    "Dispatch",
    "Darwin",
];

/// Check whether a Swift `import` module name is an Apple platform-SDK
/// framework. SwiftPM package modules are NOT recognized here — they are
/// classified from `Package.swift` at the resolver hooks. A package module
/// without a manifest declaration is honestly unresolved.
pub(super) fn is_external_swift_module(module: &str) -> bool {
    // The root module name (before the first `.`).
    let root = module.split('.').next().unwrap_or(module);
    for &ext in PLATFORM_MODULES {
        if root == ext {
            return true;
        }
    }
    false
}

/// Swift primitive type names + universal language tokens that the
/// extractor emits as type_identifier nodes. Filtered at extract time.
/// Stdlib types (Array, Dictionary, Optional, Result) flow through and
/// resolve via the swift_foundation walker.
pub(super) fn is_swift_primitive_type(name: &str) -> bool {
    matches!(
        name,
        // Numeric / boolean primitives
        "Bool" | "Int" | "Int8" | "Int16" | "Int32" | "Int64"
        | "UInt" | "UInt8" | "UInt16" | "UInt32" | "UInt64"
        | "Float" | "Float32" | "Float64" | "Float80" | "Double"
        // Empty / never types
        | "Void" | "Never" | "Any" | "AnyObject"
        // Universal literals
        | "true" | "false" | "nil"
    )
}
