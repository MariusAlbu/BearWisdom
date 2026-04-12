/// Runtime globals always external for JS/TS (browser + Node.js).
pub(crate) const EXTERNALS: &[&str] = &[
    // Browser globals
    "console", "setTimeout", "setInterval", "clearTimeout", "clearInterval",
    "requestAnimationFrame", "cancelAnimationFrame",
    "document", "window", "navigator", "location", "history",
    "localStorage", "sessionStorage",
    "fetch", "XMLHttpRequest",
    // Node.js globals
    "process", "require", "module", "exports", "__dirname", "__filename",
    "global", "globalThis",
    // Common utility libraries (always global when present)
    "toastr", "bootbox", "moment", "dayjs", "lodash", "_",
];

