// =============================================================================
// ecosystem/robot_browser_synthetics.rs — Browser library keyword stubs
//
// `robotframework-browser` (https://robotframework-browser.org) is a
// Playwright-based Robot Framework library for modern browser testing. It is
// activated by:
//
//   *** Settings ***
//   Library    Browser
//
// The library is a Python package wrapping a gRPC Node.js bridge to
// Playwright. Its keywords are Python methods on a `Browser` class; the
// tree-sitter Python extractor does not emit them in a form Robot's resolver
// can look up by bare keyword name. This module synthesises the public
// keyword API surface from the top unresolved refs observed in the
// `robot-browser` test project index.
//
// Activation: scan project `.robot` and `.resource` files for any line
// that contains `Library    Browser` (as a standalone token, not a path).
// Projects without Browser don't pay any cost.
//
// Virtual file paths: `ext:robot-browser:<keyword>.robot`
// =============================================================================

use std::path::Path;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("robot-browser");
const TAG: &str = "robot-browser";
const LANGUAGES: &[&str] = &["robot"];

// =============================================================================
// Browser library keyword catalog
// Derived from: SELECT target_name, COUNT(*) FROM unresolved_refs ... robot-browser
// =============================================================================

const BROWSER_KEYWORDS: &[&str] = &[
    // Browser / context / page lifecycle
    "New Browser",
    "Close Browser",
    "Connect To Browser",
    "Get Browser Catalog",
    "Get Page Ids",
    "New Context",
    "New Page",
    "New Persistent Context",
    "Close Context",
    "Close Page",
    "Switch Page",
    "Switch Context",
    "Switch Browser",
    // Navigation
    "Go To",
    "Get Url",
    "Reload",
    "Go Back",
    "Go Forward",
    "Wait For Load State",
    // Timing / assertions
    "Set Browser Timeout",
    "Set Retry Assertions For",
    "Set Assertion Formatters",
    "Set Default Assertion Development Mode",
    "Wait For",
    "Wait For Elements State",
    "Wait For Request",
    "Wait For Response",
    "Wait For Navigation",
    // Element interaction
    "Click",
    "Click With Options",
    "Type Text",
    "Fill Text",
    "Clear Text",
    "Type Secret",
    "Fill Secret",
    "Press Keys",
    "Press Key",
    "Focus",
    "Hover",
    "Scroll To",
    "Scroll To Element",
    "Scroll Down",
    "Scroll Up",
    "Drag And Drop",
    "Drag And Drop By Coordinates",
    "Mouse Button",
    "Mouse Move",
    "Tap",
    "Upload File",
    "Upload File By Selector",
    // Element queries / assertions
    "Get Element",
    "Get Elements",
    "Get Element By",
    "Get Element Count",
    "Get Text",
    "Get Property",
    "Get Style",
    "Get Attribute",
    "Get Attribute Names",
    "Get BoundingBox",
    "Get Boundingbox",
    "Get Table Row Index",
    "Get Checkbox State",
    "Get Select Options",
    "Get Selected Options",
    "Get Page Source",
    "Get Page",
    "Get Viewport Size",
    "Get Scroll Size",
    "Get Scroll Position",
    "Get Client Size",
    "Get Element States",
    "Get Title",
    "Element Should Be Visible",
    "Element Should Not Be Visible",
    "Element Should Be Enabled",
    "Element Should Be Disabled",
    "Element Should Be Focused",
    // Forms / select / checkbox
    "Select Options By",
    "Uncheck Checkbox",
    "Check Checkbox",
    "Select Checkbox",
    // JavaScript evaluation
    "Evaluate JavaScript",
    "Evaluate Javascript",
    // Screenshots / video
    "Take Screenshot",
    "Record Video",
    "Start Video Recording",
    "Stop Video Recording",
    // HTTP / network
    "HTTP",
    "Promise To",
    "Wait For Promise",
    // Cookies / storage
    "Add Cookie",
    "Get Cookie",
    "Get Cookies",
    "Delete Cookie",
    "Delete All Cookies",
    "Add Web Message Listener",
    "LocalStorage Get Item",
    "LocalStorage Set Item",
    "LocalStorage Remove Item",
    "LocalStorage Clear",
    "SessionStorage Get Item",
    "SessionStorage Set Item",
    "SessionStorage Remove Item",
    "SessionStorage Clear",
    // Dev tools / coverage
    "Start Coverage",
    "Stop Coverage",
    "Merge Coverage Reports",
    "Export Coverage",
    // Misc
    "Set Presenter Mode",
    "Highlight Elements",
    "Log All Scopes",
    "Strict Mode Should Be",
    "Reload Library",
    "Set Geolocation",
    "Set Offline",
    "Grant Permissions",
    "Clear Permissions",
    "Download",
    "Handle Future Dialogs",
    "Dismiss Alert",
    "Accept Alert",
    "Get Alert Message",
    "Set Viewport Size",
    "Emulate Media",
    "Mouse Wheel",
    "Scroll By",
    "Select File",
    "Deselect Option",
];

// =============================================================================
// Detection: does this project import Browser?
// =============================================================================

/// Scan project `.robot` and `.resource` files for `Library    Browser`.
/// Avoids false positives on library paths (`Library    path/to/browser.py`)
/// by requiring the target word to be `Browser` without slashes.
pub(crate) fn project_uses_browser(project_root: &Path) -> bool {
    scan_for_browser_import(project_root, 0)
}

fn scan_for_browser_import(dir: &Path, depth: u32) -> bool {
    if depth > 5 { return false; }
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || matches!(name, "node_modules" | "target" | "__pycache__") {
                continue;
            }
            if scan_for_browser_import(&path, depth + 1) {
                return true;
            }
        } else if ft.is_file() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let ext = name.rsplit('.').next().unwrap_or("");
            if !matches!(ext, "robot" | "resource") {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                // Match `Library    Browser` where `Browser` is followed by
                // whitespace, a line ending, or end of input — not a path separator.
                if content.lines().any(|line| is_browser_library_line(line)) {
                    return true;
                }
            }
        }
    }
    false
}

/// Detect `Library    Browser` (with optional arguments like `retry_assertions_for=2 sec`).
/// Requires the library token to be exactly `Browser` — excludes paths like
/// `Library    path/browser.py` or `Library    BrowserStack`.
fn is_browser_library_line(line: &str) -> bool {
    let trimmed = line.trim();
    // Must start with `Library` keyword.
    let Some(rest) = trimmed.strip_prefix("Library") else { return false };
    // Must be followed by whitespace (the separator).
    if rest.is_empty() || !rest.starts_with(|c: char| c.is_whitespace()) {
        return false;
    }
    let token = rest.trim_start();
    // Token must start with exactly `Browser` followed by whitespace, newline, or end.
    if let Some(after) = token.strip_prefix("Browser") {
        // `Browser` with no suffix, or followed by whitespace/args — valid.
        // `BrowserStack`, `BrowserLibrary`, etc. — invalid.
        after.is_empty() || after.starts_with(|c: char| c.is_whitespace())
    } else {
        false
    }
}

// =============================================================================
// Symbol / ParsedFile construction
// =============================================================================

fn keyword_sym(name: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("# Browser.{name}")),
        doc_comment: None,
        scope_path: Some("robot-browser::Browser".to_string()),
        parent_index: None,
    }
}

fn build_parsed_file(virtual_path: String, symbols: Vec<ExtractedSymbol>) -> ParsedFile {
    let n = symbols.len();
    ParsedFile {
        path: virtual_path,
        language: "robot".to_string(),
        content_hash: format!("robot-browser-{n}"),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: vec![None; n],
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: vec![false; n],
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
    }
}

fn synthesize_library() -> Vec<ParsedFile> {
    BROWSER_KEYWORDS
        .iter()
        .map(|kw| {
            let path = format!("ext:robot-browser:{kw}.robot");
            let sym = keyword_sym(kw);
            build_parsed_file(path, vec![sym])
        })
        .collect()
}

fn synthetic_dep_root() -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "robot-browser".to_string(),
        version: String::new(),
        root: std::path::PathBuf::from("ext:robot-browser"),
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

// =============================================================================
// Ecosystem impl
// =============================================================================

pub struct RobotBrowserEcosystem;

impl Ecosystem for RobotBrowserEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("robot")
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        if !project_uses_browser(ctx.project_root) {
            return Vec::new();
        }
        vec![synthetic_dep_root()]
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn parse_metadata_only(&self, _dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        Some(synthesize_library())
    }
}

impl ExternalSourceLocator for RobotBrowserEcosystem {
    fn ecosystem(&self) -> &'static str { TAG }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        if !project_uses_browser(project_root) {
            return Vec::new();
        }
        vec![synthetic_dep_root()]
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        Some(synthesize_library())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
#[path = "robot_browser_synthetics_tests.rs"]
mod tests;
