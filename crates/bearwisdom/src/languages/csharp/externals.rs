use std::collections::HashSet;

/// Runtime globals always external for C#.
pub(crate) const EXTERNALS: &[&str] = &[];

/// Dependency-gated framework globals for C#.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    // xUnit / NUnit / MSTest
    for dep in ["xunit", "nunit", "MSTest"] {
        if deps.contains(dep) {
            globals.extend(&["Assert", "Fact", "Theory", "TestMethod", "SetUp", "TearDown"]);
            break;
        }
    }

    globals
}
