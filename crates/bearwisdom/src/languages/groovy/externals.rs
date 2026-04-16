/// Groovy GDK runtime methods and well-known external types — built into the
/// Groovy language runtime or the JDK/AST API, not defined inside the project.
/// Framework-specific entries (Spock, Gradle DSL, CodeNarc) are handled by the
/// Java externals locator.
pub(crate) const EXTERNALS: &[&str] = &[
    // ---------------------------------------------------------------------------
    // GDK collection / object methods
    // ---------------------------------------------------------------------------
    "println", "print",
    "with", "tap",
    "collect", "find", "findAll",
    "each", "eachWithIndex",
    "inject",
    "any", "every",
    "sort", "unique",
    "groupBy", "collectEntries",
    "flatten",
    "sum", "min", "max", "count", "size",
    "first", "last", "head", "tail",
    "take", "drop",
    "toList", "toSet",
    "asType",
    "getClass", "toString", "hashCode", "equals",
    "metaClass", "getMetaClass",
    "invokeMethod", "getProperty", "setProperty",
    // GDK additional collection methods
    "reverse", "asBoolean", "times", "upto", "downto",
    "findResult", "findIndexOf", "findLastIndexOf",
    "withDefault", "getAt", "putAt",
    "intersect", "disjoint", "containsAll",
    "combinations", "subsequences", "permutations",
    "leftShift", "rightShift",
    "addAll", "removeAll", "retainAll",
    "join", "split", "splitEachLine",
    "push", "pop",
    // GDK object inspection / meta
    "dump", "inspect", "use", "mixin",
    "respondsTo", "hasProperty",
    "isCase", "isEmpty",
    "newInstance",
    // GDK string extensions
    "stripMargin", "stripIndent", "tokenize",
    "padLeft", "padRight", "center",
    "capitalize", "uncapitalize",
    "readLines", "eachLine",
    "tr", "replaceAll", "replaceFirst",
    "format",   // String.format (static GDK sugar)
    // GDK file/process/IO
    "withWriter", "withReader", "withInputStream", "withCloseable",
    "text", "bytes", "eachFile", "eachDir", "traverse",
    "execute",  // GDK String#execute() → Process
    "toURI", "toURL", "toFile",
    // Groovy MarkupBuilder / HTML builder DSL — dynamic node methods
    "html", "head", "body", "table", "tr", "th", "td",
    "div", "span", "p", "a", "ul", "ol", "li", "item",
    "br", "hr", "img", "form", "input", "select", "option",
    "h1", "h2", "h3", "h4", "h5", "h6",
    "script", "style", "link", "meta",
    // ---------------------------------------------------------------------------
    // org.codehaus.groovy.ast.* — Groovy compiler / CodeNarc AST type refs
    // ---------------------------------------------------------------------------
    // Base nodes
    "ASTNode", "AnnotatedNode",
    // Class / module nodes
    "ClassNode", "ModuleNode", "PackageNode", "ImportNode",
    "InnerClassNode", "EnumConstantClassNode",
    // Member nodes
    "MethodNode", "ConstructorNode", "FieldNode", "PropertyNode",
    "MixinNode", "GenericsType",
    "Parameter", "VariableScope",
    // Statement nodes
    "Statement", "BlockStatement",
    "ExpressionStatement", "ReturnStatement", "ThrowStatement",
    "IfStatement", "WhileStatement", "ForStatement", "DoWhileStatement",
    "SwitchStatement", "CaseStatement", "BreakStatement", "ContinueStatement",
    "TryCatchStatement", "CatchStatement", "SynchronizedStatement",
    "AssertStatement", "EmptyStatement",
    // Expression base & generic
    "Expression",
    "BinaryExpression", "BooleanExpression",
    "UnaryMinusExpression", "UnaryPlusExpression",
    "BitwiseNegationExpression", "NotExpression",
    "PrefixExpression", "PostfixExpression",
    "TernaryExpression", "ElvisOperatorExpression",
    "CastExpression",
    "ClosureExpression", "ClosureListExpression",
    "ConstantExpression",
    "VariableExpression",
    "PropertyExpression", "AttributeExpression", "FieldExpression",
    "ClassExpression",
    "MethodCallExpression", "StaticMethodCallExpression",
    "MethodPointerExpression",
    "ConstructorCallExpression",
    "DeclarationExpression",
    "ListExpression", "MapExpression", "MapEntryExpression",
    "ArrayExpression", "TupleExpression",
    "RangeExpression",
    "SpreadExpression", "SpreadMapExpression",
    "ArgumentListExpression",
    "GStringExpression",
    // Visitors (org.codehaus.groovy.ast.visitor / CodeNarc helpers)
    "AstVisitor", "ClassCodeVisitorSupport",
    // ---------------------------------------------------------------------------
    // java.util / java.util.concurrent types commonly imported in Groovy projects
    // ---------------------------------------------------------------------------
    "BitSet",
    "Comparator",
    "ExecutorService", "Executors",
    "ConcurrentMap", "ConcurrentHashMap",
    "InterruptedException",
    // ---------------------------------------------------------------------------
    // Spock Framework lifecycle / assertion keywords (beyond those in builtins)
    // ---------------------------------------------------------------------------
    "setupSpec", "cleanupSpec",
    "verifyAll", "verifyEach",
    "thrown", "notThrown",
    "Mock", "Stub", "Spy", "Interaction",
    // ---------------------------------------------------------------------------
    // Gradle DSL top-level blocks / methods (beyond those in builtins)
    // ---------------------------------------------------------------------------
    "implementation", "testImplementation", "api",
    "runtimeOnly", "testRuntimeOnly",
    "compileOnly", "testCompileOnly",
    "annotationProcessor",
    "testFixturesImplementation",
    "classpath",
    "mavenCentral", "mavenLocal", "gradlePluginPortal", "jcenter",
    "google",
];
