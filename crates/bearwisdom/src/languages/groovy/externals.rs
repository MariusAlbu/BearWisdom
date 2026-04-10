/// Groovy built-in methods, GDK collection extensions, Spock testing
/// constructs, and Gradle DSL keywords that are never defined inside a project.
pub(crate) const EXTERNALS: &[&str] = &[
    // -------------------------------------------------------------------------
    // Groovy / GDK object and collection methods
    // -------------------------------------------------------------------------
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
    // GDK string extensions
    "stripMargin", "stripIndent", "tokenize",
    "padLeft", "padRight", "center",
    "capitalize", "uncapitalize",
    "denormalize", "normalize",
    "readLines", "eachLine",
    "execute", "toList",
    // GDK file/process
    "withWriter", "withReader", "withInputStream",
    "text", "bytes", "eachFile", "eachDir", "traverse",
    // -------------------------------------------------------------------------
    // Spock framework
    // -------------------------------------------------------------------------
    "setup", "given", "when", "then", "expect", "where", "cleanup",
    "Mock", "Stub", "Spy", "thrown", "notThrown",
    // -------------------------------------------------------------------------
    // Gradle DSL
    // -------------------------------------------------------------------------
    "apply", "plugins", "dependencies", "repositories",
    "configurations", "task", "sourceSets",
    "buildscript", "allprojects", "subprojects", "ext",
    // -------------------------------------------------------------------------
    // CodeNarc test assertions (inherited via AbstractRuleTestCase)
    // -------------------------------------------------------------------------
    "assertSingleViolation", "assertTwoViolations", "assertViolations",
    "assertNoViolations", "assertNoViolation",
    "shouldFailWithMessageContaining", "shouldFail",
    "applyRuleTo", "sourceCodeFor",
    "manuallyApplyRule", "assertInlineViolations",
    // -------------------------------------------------------------------------
    // Groovy AST types (org.codehaus.groovy.ast)
    // -------------------------------------------------------------------------
    "Expression", "ASTNode", "ClassNode", "MethodNode",
    "FieldNode", "PropertyNode", "Parameter", "AnnotationNode",
    "BinaryExpression", "MethodCallExpression", "ConstantExpression",
    "VariableExpression", "PropertyExpression", "ClosureExpression",
    "DeclarationExpression", "ConstructorCallExpression",
    "TupleExpression", "GStringExpression", "ListExpression",
    "MapExpression", "MapEntryExpression", "CastExpression",
    "ClassHelper", "GenericsUtils",
    // -------------------------------------------------------------------------
    // Groovy control flow (sometimes extracted as calls)
    // -------------------------------------------------------------------------
    "if", "else", "while", "for", "switch", "case",
    "do", "try", "catch", "finally", "throw",
    "return", "break", "continue",
];
