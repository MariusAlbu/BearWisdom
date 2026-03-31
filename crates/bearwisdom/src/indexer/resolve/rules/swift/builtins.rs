// =============================================================================
// swift/builtins.rs — Swift builtin and helper predicates
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

/// Always-external Swift framework/module names.
const ALWAYS_EXTERNAL_MODULES: &[&str] = &[
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
    "Vapor",
    "Fluent",
    "Leaf",
    "Queues",
    "JWT",
    "RxSwift",
    "RxCocoa",
    "Combine",
    "Alamofire",
    "Moya",
    "SnapKit",
    "Kingfisher",
    "SDWebImage",
    "RealmSwift",
    "Realm",
    "Firebase",
    "FirebaseFirestore",
    "FirebaseAuth",
    "FirebaseStorage",
    "Quick",
    "Nimble",
];

/// Check whether a Swift `import` module name is external.
pub(super) fn is_external_swift_module(module: &str) -> bool {
    // The root module name (before the first `.`).
    let root = module.split('.').next().unwrap_or(module);
    for &ext in ALWAYS_EXTERNAL_MODULES {
        if root == ext {
            return true;
        }
    }
    false
}

/// Swift stdlib builtins always in scope (no import needed).
pub(super) fn is_swift_builtin(name: &str) -> bool {
    let root = name.split('.').next().unwrap_or(name);
    matches!(
        root,
        // Global functions
        "print"
            | "debugPrint"
            | "dump"
            | "fatalError"
            | "precondition"
            | "preconditionFailure"
            | "assert"
            | "assertionFailure"
            | "min"
            | "max"
            | "abs"
            | "zip"
            | "stride"
            | "sequence"
            | "repeatElement"
            | "swap"
            | "withUnsafePointer"
            | "withUnsafeMutablePointer"
            | "withUnsafeBytes"
            | "withUnsafeMutableBytes"
            | "withExtendedLifetime"
            | "unsafeBitCast"
            | "unsafeDowncast"
            | "type"
            | "MemoryLayout"
            | "numericCast"
            | "readLine"
            // Swift stdlib types (always in scope)
            | "String"
            | "Substring"
            | "Character"
            | "Unicode"
            | "Int"
            | "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "UInt"
            | "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "Float"
            | "Double"
            | "Float80"
            | "Bool"
            | "Array"
            | "ContiguousArray"
            | "ArraySlice"
            | "Dictionary"
            | "Set"
            | "Optional"
            | "Result"
            | "Never"
            | "Void"
            | "AnyObject"
            | "AnyClass"
            | "Any"
            // Protocols (Swift stdlib)
            | "Error"
            | "Codable"
            | "Encodable"
            | "Decodable"
            | "Hashable"
            | "Equatable"
            | "Comparable"
            | "CustomStringConvertible"
            | "CustomDebugStringConvertible"
            | "Identifiable"
            | "Sendable"
            | "CaseIterable"
            | "RawRepresentable"
            | "Sequence"
            | "Collection"
            | "BidirectionalCollection"
            | "RandomAccessCollection"
            | "MutableCollection"
            | "IteratorProtocol"
            | "StringProtocol"
            | "Numeric"
            | "SignedNumeric"
            | "BinaryInteger"
            | "FixedWidthInteger"
            | "FloatingPoint"
            // SwiftUI state / view protocols (extremely common)
            | "ObservableObject"
            | "Published"
            | "State"
            | "Binding"
            | "StateObject"
            | "ObservedObject"
            | "EnvironmentObject"
            | "Environment"
            | "View"
            | "some"
            // pseudo-keywords used as refs
            | "self"
            | "Self"
            | "super"
    )
}
