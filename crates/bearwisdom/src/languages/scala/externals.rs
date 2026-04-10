use std::collections::HashSet;

/// Runtime globals always external for Scala.
pub(crate) const EXTERNALS: &[&str] = &[
    "Logger", "LoggerFactory",
];

/// Dependency-gated framework globals for Scala.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    // Cats operators / FP methods are universal across any Cats-using project.
    // They arrive as method dispatches so the import walk never sees them.
    let has_cats = deps.iter().any(|d| d.starts_with("org.typelevel") || d.starts_with("cats"));
    let has_fs2  = deps.iter().any(|d| d.starts_with("co.fs2") || d.starts_with("fs2"));
    if has_cats || has_fs2 {
        globals.extend(CATS_GLOBALS);
    }

    if deps.contains("org.scalatest") {
        globals.extend(SCALATEST_GLOBALS);
    }

    globals
}

/// Cats / FP operator and method names surfaced as unresolved call edges.
/// These are typeclass method dispatches — they can never be resolved via the
/// import graph, so we mark them as known externals to suppress false positives.
const CATS_GLOBALS: &[&str] = &[
    // Symbolic operators
    "*>", "<*", "===", "=!=", ">>", ">>=", "<*>", "<$>", "|+|", ">>>", "<<<", "&>", "<&",
    // Universal FP methods
    "flatMap", "map", "fold", "foldLeft", "foldRight",
    "traverse", "sequence", "pure", "flatten",
    "filter", "collect", "exists", "forall", "foreach", "groupBy",
    "toList", "toVector", "toSet", "toMap", "toOption",
    "getOrElse", "orElse", "contains", "mkString",
    "zip", "zipWithIndex", "take", "drop",
    "head", "tail", "last", "headOption", "lastOption",
    "isEmpty", "nonEmpty", "size", "length",
    // Effect / stream combinators
    "unsafeRunSync", "use", "evalMap", "compile", "drain",
    "through", "attempt", "handleErrorWith", "recoverWith",
    "void", "as", "tupleLeft", "tupleRight", "product", "productL", "productR",
];

const SCALATEST_GLOBALS: &[&str] = &[
    "should", "must", "can", "in", "ignore",
    "FlatSpec", "WordSpec", "FunSuite", "FunSpec",
    "AnyFlatSpec", "AnyWordSpec", "AnyFunSuite", "AnyFunSpec",
    "Matchers", "BeforeAndAfter", "BeforeAndAfterAll",
];
