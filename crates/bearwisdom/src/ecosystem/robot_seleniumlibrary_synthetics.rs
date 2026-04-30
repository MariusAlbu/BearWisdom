// =============================================================================
// ecosystem/robot_seleniumlibrary_synthetics.rs — SeleniumLibrary keyword stubs
//
// SeleniumLibrary is the most widely-used Robot Framework library for
// browser-based testing. It is a third-party package (pip install
// robotframework-seleniumlibrary) and is NOT part of the Robot Framework
// standard distribution. Projects that use it declare:
//
//   *** Settings ***
//   Library    SeleniumLibrary
//
// The library's Python source is under site-packages and is walked by the
// Python externals pipeline, but the keyword names are methods on a Python
// class — the current tree-sitter Python extractor does not emit them in a
// form the Robot resolver can look up by bare keyword name. This module
// synthesises the public keyword API surface so those refs resolve cleanly.
//
// Activation: scan project `.robot` and `.resource` files for any line
// containing `Library    SeleniumLibrary` (or `Library    Selenium2Library`).
// Projects that don't use SeleniumLibrary don't pay any cost.
//
// Virtual file paths: `ext:robot-seleniumlibrary:<keyword>.robot`
// =============================================================================

use std::path::Path;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("robot-seleniumlibrary");
const TAG: &str = "robot-seleniumlibrary";
const LANGUAGES: &[&str] = &["robot"];

// =============================================================================
// SeleniumLibrary keyword catalog
// =============================================================================

const SELENIUM_KEYWORDS: &[&str] = &[
    // Browser lifecycle
    "Open Browser",
    "Close Browser",
    "Close All Browsers",
    "Create Webdriver",
    "Switch Browser",
    "Get Browser Ids",
    "Get Session Id",
    // Navigation
    "Go To",
    "Go Back",
    "Reload Page",
    "Get Location",
    "Get Title",
    "Get Source",
    // Window / frame management
    "Maximize Browser Window",
    "Set Browser Implicit Wait",
    "Set Selenium Speed",
    "Set Selenium Timeout",
    "Set Selenium Implicit Wait",
    "Select Frame",
    "Unselect Frame",
    "Switch Window",
    "Get Window Handles",
    "Get Window Titles",
    "Get Window Position",
    "Get Window Size",
    "Set Window Position",
    "Set Window Size",
    // Waiting
    "Wait Until Element Is Visible",
    "Wait Until Element Is Not Visible",
    "Wait Until Element Is Enabled",
    "Wait Until Element Is Disabled",
    "Wait Until Page Contains",
    "Wait Until Page Does Not Contain",
    "Wait Until Page Contains Element",
    "Wait Until Page Does Not Contain Element",
    // Element assertions
    "Page Should Contain",
    "Page Should Not Contain",
    "Page Should Contain Element",
    "Page Should Not Contain Element",
    "Page Should Contain Link",
    "Page Should Not Contain Link",
    "Page Should Contain Button",
    "Page Should Not Contain Button",
    "Page Should Contain Checkbox",
    "Page Should Not Contain Checkbox",
    "Page Should Contain Radio Button",
    "Page Should Not Contain Radio Button",
    "Page Should Contain Image",
    "Page Should Not Contain Image",
    "Page Should Contain Textfield",
    "Page Should Not Contain Textfield",
    "Element Should Be Visible",
    "Element Should Not Be Visible",
    "Element Should Be Enabled",
    "Element Should Be Disabled",
    "Element Should Be Selected",
    "Element Should Not Be Selected",
    "Element Should Contain",
    "Element Should Not Contain",
    "Element Text Should Be",
    "Element Text Should Not Be",
    "Element Attribute Value Should Be",
    // Element interaction — click
    "Click Element",
    "Click Button",
    "Click Link",
    "Click Image",
    "Double Click Element",
    "Click Element At Coordinates",
    // Element interaction — input
    "Input Text",
    "Input Password",
    "Clear Element Text",
    "Press Key",
    "Press Keys",
    // Checkbox / radio / dropdown
    "Select Checkbox",
    "Unselect Checkbox",
    "Checkbox Should Be Selected",
    "Checkbox Should Not Be Selected",
    "Select Radio Button",
    "Radio Button Should Be Set To",
    "Radio Button Should Not Be Selected",
    "Select From List By Value",
    "Select From List By Label",
    "Select From List By Index",
    "Unselect From List By Value",
    "Unselect From List By Label",
    "Unselect From List By Index",
    "Unselect All From List",
    "List Selection Should Be",
    "List Should Have No Selections",
    "Get Selected List Value",
    "Get Selected List Values",
    "Get Selected List Label",
    "Get Selected List Labels",
    // Element queries
    "Get Text",
    "Get Value",
    "Get Element Attribute",
    "Get Element Count",
    "Get Element Size",
    "Get Element Location",
    "Get WebElement",
    "Get WebElements",
    "Get All Links",
    "Get List Items",
    "Get Horizontal Position",
    "Get Vertical Position",
    // JavaScript / cookies / alerts
    "Execute Javascript",
    "Execute Async Javascript",
    "Add Cookie",
    "Get Cookie",
    "Get Cookies",
    "Delete Cookie",
    "Delete All Cookies",
    "Alert Should Be Present",
    "Alert Should Not Be Present",
    "Handle Alert",
    "Input Text Into Alert",
    // Screenshots / logging
    "Capture Page Screenshot",
    "Capture Element Screenshot",
    "Log Title",
    "Log Location",
    "Log Source",
    // Mouse actions
    "Mouse Down",
    "Mouse Up",
    "Mouse Over",
    "Mouse Out",
    "Mouse Down On Link",
    "Mouse Down On Image",
    "Open Context Menu",
    "Drag And Drop",
    "Drag And Drop By Offset",
    "Scroll Element Into View",
    // Misc
    "Set Focus To Element",
    "Simulate Event",
    "Register Keyword To Run On Failure",
    "Assign Id To Element",
    "Element Should Be Focused",
    "Wait Until Element Contains",
    "Wait Until Element Does Not Contain",
    "Wait For Condition",
];

// =============================================================================
// Detection: does this project import SeleniumLibrary?
// =============================================================================

/// Scan project files for `Library    SeleniumLibrary` import lines.
pub(crate) fn project_uses_seleniumlibrary(project_root: &Path) -> bool {
    scan_for_library_import(project_root, "SeleniumLibrary", 0)
        || scan_for_library_import(project_root, "Selenium2Library", 0)
}

fn scan_for_library_import(dir: &Path, library_name: &str, depth: u32) -> bool {
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
            if scan_for_library_import(&path, library_name, depth + 1) {
                return true;
            }
        } else if ft.is_file() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let ext = name.rsplit('.').next().unwrap_or("");
            if !matches!(ext, "robot" | "resource") {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                if content.contains(library_name) {
                    return true;
                }
            }
        }
    }
    false
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
        signature: Some(format!("# SeleniumLibrary.{name}")),
        doc_comment: None,
        scope_path: Some("robot-seleniumlibrary::SeleniumLibrary".to_string()),
        parent_index: None,
    }
}

fn build_parsed_file(virtual_path: String, symbols: Vec<ExtractedSymbol>) -> ParsedFile {
    let n = symbols.len();
    ParsedFile {
        path: virtual_path,
        language: "robot".to_string(),
        content_hash: format!("robot-seleniumlibrary-{n}"),
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
    SELENIUM_KEYWORDS
        .iter()
        .map(|kw| {
            let path = format!("ext:robot-seleniumlibrary:{kw}.robot");
            let sym = keyword_sym(kw);
            build_parsed_file(path, vec![sym])
        })
        .collect()
}

fn synthetic_dep_root() -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "robot-seleniumlibrary".to_string(),
        version: String::new(),
        root: std::path::PathBuf::from("ext:robot-seleniumlibrary"),
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

// =============================================================================
// Ecosystem impl
// =============================================================================

pub struct RobotSeleniumLibraryEcosystem;

impl Ecosystem for RobotSeleniumLibraryEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("robot")
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        if !project_uses_seleniumlibrary(ctx.project_root) {
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

impl ExternalSourceLocator for RobotSeleniumLibraryEcosystem {
    fn ecosystem(&self) -> &'static str { TAG }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        if !project_uses_seleniumlibrary(project_root) {
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
#[path = "robot_seleniumlibrary_synthetics_tests.rs"]
mod tests;
