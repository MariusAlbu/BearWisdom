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

