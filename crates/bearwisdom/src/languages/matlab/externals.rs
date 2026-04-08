use std::collections::HashSet;

/// Runtime globals always external for MATLAB.
///
/// MATLAB toolbox functions and OOP infrastructure that appear in project code
/// but are never defined there. These supplement primitives.rs (core builtins).
pub(crate) const EXTERNALS: &[&str] = &[
    // Object-oriented infrastructure
    "properties", "methods", "events", "enumeration",
    "addlistener", "notify", "isvalid", "delete",
    "findobj", "findprop",
    // Handle class methods
    "copy", "listener", "addprop",
    // Parallel computing (parfor, spmd are keywords, but workers/pools are objects)
    "gcp", "parpool", "delete",
    // Timer
    "timer", "start", "stop",
    // Figure / graphics object methods
    "drawnow", "refresh", "clf", "cla", "clc",
    "colorbar", "colormap", "shading", "lighting",
    "camlight", "material", "view",
    "getframe", "frame2im",
    // Animation / video
    "VideoWriter", "open", "writeVideo", "close",
    // inputParser
    "inputParser", "addRequired", "addOptional", "addParameter", "parse",
    // containers.Map methods
    "isKey", "keys", "values", "remove",
    // Java interop (MATLAB can call Java)
    "javaObject", "javaMethod", "javaArray",
    // MEX
    "mexFunction", "mxGetPr", "mxGetM", "mxGetN",
    "mxCreateDoubleMatrix", "mxCreateDoubleScalar",
    "mexErrMsgTxt", "mexPrintf",
];

/// Dependency-gated framework globals for MATLAB.
///
/// MATLAB toolboxes are identified by their package names or by
/// characteristic functions present in deps/requirements.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    // MATLAB Unit Testing Framework (built-in since R2013a, but may be
    // declared as a dependency in CI config or project manifests)
    if deps.contains("matlab.unittest") || deps.contains("matlab-test") {
        globals.extend(MATLAB_TEST_GLOBALS);
    }
    // Simulink
    if deps.contains("simulink") || deps.contains("Simulink") {
        globals.extend(&[
            "sim", "simout", "set_param", "get_param",
            "add_block", "add_line", "delete_block", "delete_line",
            "new_system", "open_system", "close_system", "save_system",
            "bdroot", "gcb", "gcs",
        ]);
    }

    globals
}

const MATLAB_TEST_GLOBALS: &[&str] = &[
    "matlab.unittest.TestCase",
    "matlab.unittest.TestRunner",
    "matlab.unittest.TestSuite",
    "verifyEqual", "verifyTrue", "verifyFalse",
    "verifyEmpty", "verifyNotEmpty", "verifySize",
    "verifyError", "verifyWarning", "verifyReturnsTrue",
    "verifyClass", "verifyInstanceOf",
    "verifyGreaterThan", "verifyLessThan",
    "verifyGreaterThanOrEqual", "verifyLessThanOrEqual",
    "assertError", "assertWarning", "assertEqual",
    "assumeTrue", "assumeEqual",
    "fatalAssertEqual", "fatalAssertTrue",
    "TestCase", "TestRunner", "TestSuite",
    "run", "runFile", "runDirectory",
];
