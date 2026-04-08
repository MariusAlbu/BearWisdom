use std::collections::HashSet;

/// Runtime globals always external for Scala.
pub(crate) const EXTERNALS: &[&str] = &[
    "Logger", "LoggerFactory",
];

/// Dependency-gated framework globals for Scala.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    if deps.contains("org.scalatest") {
        globals.extend(SCALATEST_GLOBALS);
    }

    globals
}

const SCALATEST_GLOBALS: &[&str] = &[
    "should", "must", "can", "in", "ignore",
    "FlatSpec", "WordSpec", "FunSuite", "FunSpec",
    "AnyFlatSpec", "AnyWordSpec", "AnyFunSuite", "AnyFunSpec",
    "Matchers", "BeforeAndAfter", "BeforeAndAfterAll",
];
