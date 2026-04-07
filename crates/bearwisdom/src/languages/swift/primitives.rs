// =============================================================================
// swift/primitives.rs — Swift primitive types
// =============================================================================

/// Primitive and built-in type names for Swift.
pub(crate) const PRIMITIVES: &[&str] = &[
    // Numeric
    "Int", "Int8", "Int16", "Int32", "Int64",
    "UInt", "UInt8", "UInt16", "UInt32", "UInt64",
    "Float", "Double", "Float16", "Float80", "CGFloat",
    "Decimal", "NSDecimalNumber", "NSNumber",
    "Bool", "String", "Character", "Void", "Never",
    "Any", "AnyObject", "AnyHashable", "Self",
    // Collections
    "Optional", "Array", "Dictionary", "Set", "Sequence",
    "Collection", "MutableCollection", "RandomAccessCollection",
    "Range", "ClosedRange", "Stride",
    "ArraySlice", "ContiguousArray",
    // Data / buffers
    "Data", "URL", "URLRequest", "URLResponse", "URLSession",
    "HTTPURLResponse", "URLSessionDataTask", "URLSessionTask",
    // Errors
    "Error", "NSError", "LocalizedError", "DecodingError", "EncodingError",
    // Foundation
    "Date", "DateFormatter", "Calendar", "TimeZone", "Locale",
    "UUID", "Notification", "NotificationCenter", "UserDefaults",
    "DispatchQueue", "DispatchGroup", "DispatchSemaphore",
    "OperationQueue", "Operation",
    "NSObject", "NSString", "NSArray", "NSDictionary", "NSSet",
    "NSPredicate", "NSAttributedString",
    // Codable
    "Codable", "Encodable", "Decodable", "Encoder", "Decoder",
    "JSONEncoder", "JSONDecoder", "PropertyListEncoder", "PropertyListDecoder",
    "CodingKey", "KeyedDecodingContainer", "KeyedEncodingContainer",
    "SingleValueDecodingContainer", "SingleValueEncodingContainer",
    "UnkeyedDecodingContainer", "UnkeyedEncodingContainer",
    // Combine / async
    "Task", "AsyncSequence", "AsyncStream", "CheckedContinuation",
    "Publisher", "AnyPublisher", "PassthroughSubject", "CurrentValueSubject",
    "Cancellable", "AnyCancellable",
    // SwiftUI
    "View", "State", "Binding", "ObservedObject", "EnvironmentObject",
    "Published", "ObservableObject", "StateObject", "Environment",
    "Text", "Image", "Button", "NavigationView", "NavigationLink",
    "List", "ForEach", "VStack", "HStack", "ZStack", "Group",
    "GeometryReader", "ScrollView", "LazyVStack", "LazyHStack",
    "Color", "Font", "EdgeInsets", "CGSize", "CGPoint", "CGRect",
    // Generic type parameters
    "T", "U", "K", "V", "E", "R", "S", "Element", "Key", "Value",
    // Common protocol names
    "Hashable", "Equatable", "Comparable", "Identifiable", "CustomStringConvertible",
    "Sendable", "Actor",
];
