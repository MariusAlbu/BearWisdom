// =============================================================================
// dart/primitives.rs — Dart primitive types
// =============================================================================

/// Primitive and built-in type names for Dart.
pub(crate) const PRIMITIVES: &[&str] = &[
    // Core types
    "int", "double", "num", "bool", "String", "List", "Map", "Set", "dynamic",
    "void", "Null", "Object", "Future", "Stream", "Iterable", "Type", "Function",
    "Never", "Record", "Enum", "Duration", "DateTime", "Uri", "BigInt",
    "Symbol", "Runes", "Pattern", "Match", "RegExp", "Comparable",
    // Typed data
    "Uint8List", "Uint16List", "Uint32List", "Int8List", "Int16List", "Int32List",
    "Float32List", "Float64List", "ByteData", "ByteBuffer", "Endian",
    // Async
    "Completer", "StreamController", "StreamSubscription", "Timer",
    "FutureOr", "Zone",
    // Collections
    "LinkedHashMap", "LinkedHashSet", "HashMap", "HashSet", "Queue",
    "ListQueue", "SplayTreeMap", "SplayTreeSet", "UnmodifiableListView",
    // IO / Convert
    "File", "Directory", "Platform", "Stdin", "Stdout", "Stderr",
    "Encoding", "Utf8Codec", "JsonCodec", "Base64Codec",
    // Flutter SDK core
    "Widget", "StatelessWidget", "StatefulWidget", "State",
    "BuildContext", "Key", "ValueKey", "ObjectKey", "GlobalKey", "UniqueKey",
    "EdgeInsets", "EdgeInsetsGeometry", "Alignment", "AlignmentGeometry",
    "Color", "Colors", "Icons", "TextStyle", "ThemeData",
    "Size", "Offset", "Rect", "BorderRadius", "BoxDecoration",
    "Container", "Column", "Row", "Stack", "Expanded", "Flexible",
    "Padding", "Center", "SizedBox", "Scaffold", "AppBar",
    "Text", "Icon", "Image", "ListView", "GridView",
    "Navigator", "MaterialApp", "MaterialPageRoute",
    "GestureDetector", "InkWell", "TextButton", "ElevatedButton",
    "IconButton", "FloatingActionButton",
    "TextField", "TextFormField", "TextEditingController",
    "ScrollController", "AnimationController", "Animation",
    "ValueNotifier", "ChangeNotifier",
    // Drift / Moor ORM (common in generated code)
    "GeneratedColumn", "GeneratedDatabase", "Table", "TableInfo",
    "DataClass", "Insertable", "UpdateCompanion", "RawValuesInsertable",
    "Expression", "CustomExpression", "Variable", "Constant",
    "QueryBuilder", "JoinedSelectStatement", "SimpleSelectStatement",
    "ValueSerializer",
    "VerificationMeta", "VerificationResult",
    // Isar (common in generated code)
    "QAfterFilterCondition", "QAfterSortBy", "QAfterWhereClause",
    "QFilterCondition", "QSortBy", "QWhereClause",
    "IsarCollection", "IsarLink", "IsarLinks",
    // Generic type parameters
    "T", "E", "K", "V", "S", "R",
    // Synthetic
    "absent",
];
