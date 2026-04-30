// =============================================================================
// ecosystem/robot_builtin_synthetics.rs — Robot Framework stdlib keyword stubs
//
// Robot Framework ships a set of standard libraries that are always available
// to any project that has Robot Framework installed. These libraries do not
// live on disk as parseable `.robot` source — they are Python modules bundled
// with the Robot Framework package. The primary one, `BuiltIn`, is imported
// implicitly by every robot file without a `Library    BuiltIn` import; the
// rest (Collections, String, OperatingSystem, Process, DateTime, XML,
// Screenshot, Dialogs, Telnet) are opt-in but ship with the core distribution
// and are indexed here so that `Library    Collections` etc. resolve cleanly.
//
// Virtual file paths: `ext:robot-builtin:<library>/<keyword>.robot`
//
// Activation: any `.robot` or `.resource` file in the project
//   (`LanguagePresent("robot")`). No on-disk walk needed — everything is
//   synthesised in `parse_metadata_only`. The BuiltIn keywords resolve
//   regardless of imports because Robot auto-imports BuiltIn for every file.
//
// Resolution flow change: `RobotResolver::resolve` no longer calls
// `is_robot_builtin()`; the global lookup at step 5 now finds these synthetic
// symbols directly, which is the correct architectural path.
// =============================================================================

use std::path::Path;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("robot-builtin");
const TAG: &str = "robot-builtin";
const LANGUAGES: &[&str] = &["robot"];

// =============================================================================
// Keyword catalogs — grouped by standard library
// =============================================================================

/// Robot Framework BuiltIn library keywords.
/// Auto-imported by every robot file; no `Library    BuiltIn` needed.
const BUILTIN_KEYWORDS: &[&str] = &[
    // Logging
    "Log",
    "Log Many",
    "Log To Console",
    "Log Variables",
    // Variable handling
    "Set Variable",
    "Set Suite Variable",
    "Set Global Variable",
    "Set Test Variable",
    "Set Local Variable",
    "Set Task Variable",
    "Get Variable Value",
    "Variable Should Exist",
    "Variable Should Not Exist",
    // Assertions — equality / truth
    "Should Be Equal",
    "Should Not Be Equal",
    "Should Be True",
    "Should Not Be True",
    "Should Be Empty",
    "Should Not Be Empty",
    "Should Contain",
    "Should Not Contain",
    "Should Start With",
    "Should End With",
    "Should Match",
    "Should Not Match",
    "Should Match Regexp",
    "Should Not Match Regexp",
    "Should Be Equal As Integers",
    "Should Be Equal As Numbers",
    "Should Be Equal As Strings",
    "Should Be Equal As Bytes",
    "Should Not Be Equal As Integers",
    "Should Not Be Equal As Numbers",
    "Should Not Be Equal As Strings",
    "Length Should Be",
    // Control flow — run keyword variants
    "Run Keyword",
    "Run Keyword If",
    "Run Keyword Unless",
    "Run Keyword And Return",
    "Run Keyword And Return If",
    "Run Keyword And Return Status",
    "Run Keyword And Ignore Error",
    "Run Keyword And Expect Error",
    "Run Keyword And Continue On Failure",
    "Run Keyword And Warn On Failure",
    "Run Keywords",
    "Repeat Keyword",
    "Wait Until Keyword Succeeds",
    "Run Keyword If Any Tests Failed",
    "Run Keyword If All Tests Passed",
    "Run Keyword If Any Critical Tests Failed",
    "Run Keyword If All Critical Tests Passed",
    "Run Keyword If Test Failed",
    "Run Keyword If Test Passed",
    "Run Keyword If Timeout Occurred",
    "Run Keyword If Setup Failed",
    "Run Keyword If Teardown Failed",
    "Return From Keyword",
    "Return From Keyword If",
    // Execution control
    "Pass Execution",
    "Pass Execution If",
    "Fatal Error",
    "Fail",
    "Skip",
    "Skip If",
    "Comment",
    "No Operation",
    "Sleep",
    // Type conversion
    "Convert To Integer",
    "Convert To String",
    "Convert To Number",
    "Convert To Boolean",
    "Convert To Bytes",
    "Convert To Hex",
    "Convert To Octal",
    "Convert To Binary",
    // Collections
    "Create List",
    "Create Dictionary",
    "Append To List",
    "Remove From List",
    "Get From List",
    "Get From Dictionary",
    "Set To Dictionary",
    // String / evaluation
    "Evaluate",
    "Call Method",
    "Catenate",
    "Get Time",
    "Get Count",
    "Get Length",
    // Library / resource management
    "Import Library",
    "Import Resource",
    "Import Variables",
    "Set Library Search Order",
    "Keyword Should Exist",
    "Set Log Level",
    // Other BuiltIn
    "Get Library Instance",
    "Reload Library",
    "Add Tags",
    "Remove Tags",
    "Get Tags",
    "Set Tags",
    "Fail If",
    "Run Setup Only Once",
    "Set Test Documentation",
    "Set Suite Documentation",
    "Set Test Message",
    "Set Suite Metadata",
    "Log Message",
];

/// `Collections` standard library keywords.
/// Imported via `Library    Collections`.
const COLLECTIONS_KEYWORDS: &[&str] = &[
    "Append To List",
    "Combine Lists",
    "Convert To List",
    "Copy Dictionary",
    "Copy List",
    "Count Values In List",
    "Dictionaries Should Be Equal",
    "Dictionary Should Contain Item",
    "Dictionary Should Contain Key",
    "Dictionary Should Contain Sub Dictionary",
    "Dictionary Should Contain Value",
    "Dictionary Should Not Contain Key",
    "Dictionary Should Not Contain Value",
    "Get Dictionary Items",
    "Get Dictionary Keys",
    "Get Dictionary Values",
    "Get From Dictionary",
    "Get From List",
    "Get Index From List",
    "Get Match Count",
    "Get Matches",
    "Get Slice From List",
    "Insert Into List",
    "Keep In Dictionary",
    "List Should Contain Sub List",
    "List Should Contain Value",
    "List Should Not Contain Duplicates",
    "List Should Not Contain Value",
    "Lists Should Be Equal",
    "Log Dictionary",
    "Log List",
    "Merge Lists",
    "Pop From Dictionary",
    "Remove Duplicates",
    "Remove From Dictionary",
    "Remove From List",
    "Remove Values From List",
    "Reverse List",
    "Set List Value",
    "Set To Dictionary",
    "Should Contain Match",
    "Should Not Contain Match",
    "Sort List",
];

/// `String` standard library keywords.
/// Imported via `Library    String`.
const STRING_KEYWORDS: &[&str] = &[
    "Convert To Lower Case",
    "Convert To Title Case",
    "Convert To Upper Case",
    "Decode Bytes To String",
    "Encode String To Bytes",
    "Fetch From Left",
    "Fetch From Right",
    "Format String",
    "Generate Random String",
    "Get Line",
    "Get Line Count",
    "Get Lines Containing String",
    "Get Lines Matching Pattern",
    "Get Lines Matching Regexp",
    "Get Regexp Matches",
    "Get Substring",
    "Remove String",
    "Remove String Using Regexp",
    "Replace String",
    "Replace String Using Regexp",
    "Should Be String",
    "Should Be Unicode String",
    "Should Be Byte String",
    "Should Not Be String",
    "Split String",
    "Split String From Right",
    "Split String To Characters",
    "Split To Lines",
    "String Should Match Regexp",
    "Strip String",
];

/// `OperatingSystem` standard library keywords.
/// Imported via `Library    OperatingSystem`.
const OPERATING_SYSTEM_KEYWORDS: &[&str] = &[
    "Append To File",
    "Copy Directory",
    "Copy File",
    "Copy Files",
    "Count Directories In Directory",
    "Count Files In Directory",
    "Count Items In Directory",
    "Create Binary File",
    "Create Directory",
    "Create File",
    "Directory Should Be Empty",
    "Directory Should Exist",
    "Directory Should Not Be Empty",
    "Directory Should Not Exist",
    "Empty Directory",
    "Environment Variable Should Be Set",
    "Environment Variable Should Not Be Set",
    "File Should Be Empty",
    "File Should Exist",
    "File Should Not Be Empty",
    "File Should Not Exist",
    "Get Binary File",
    "Get Environment Variable",
    "Get Environment Variables",
    "Get File",
    "Get File Size",
    "Get Modified Time",
    "Grep File",
    "Join Path",
    "Join Paths",
    "List Directories In Directory",
    "List Directory",
    "List Files In Directory",
    "Log Environment Variables",
    "Log File",
    "Move Directory",
    "Move File",
    "Move Files",
    "Normalize Path",
    "Remove Directory",
    "Remove Environment Variable",
    "Remove File",
    "Remove Files",
    "Run",
    "Run And Return Rc",
    "Run And Return Rc And Output",
    "Set Environment Variable",
    "Set Modified Time",
    "Should Exist",
    "Should Not Exist",
    "Split Extension",
    "Split Path",
    "Touch",
    "Wait Until Created",
    "Wait Until Removed",
];

/// `Process` standard library keywords.
/// Imported via `Library    Process`.
const PROCESS_KEYWORDS: &[&str] = &[
    "Get Process Id",
    "Get Process Object",
    "Get Process Result",
    "Is Process Running",
    "Process Should Be Running",
    "Process Should Be Stopped",
    "Run Process",
    "Send Signal To Process",
    "Split Command Line",
    "Start Process",
    "Switch Process",
    "Terminate All Processes",
    "Terminate Process",
    "Wait For Process",
];

/// `DateTime` standard library keywords.
/// Imported via `Library    DateTime`.
const DATETIME_KEYWORDS: &[&str] = &[
    "Add Time To Date",
    "Add Time To Time",
    "Convert Date",
    "Convert Time",
    "Get Current Date",
    "Subtract Date From Date",
    "Subtract Time From Date",
    "Subtract Time From Time",
];

/// `XML` standard library keywords.
/// Imported via `Library    XML`.
const XML_KEYWORDS: &[&str] = &[
    "Add Element",
    "Clear Element",
    "Copy Element",
    "Element Attribute Should Be",
    "Element Attribute Should Match",
    "Element Should Exist",
    "Element Should Not Exist",
    "Element Text Should Be",
    "Element Text Should Match",
    "Elements Should Be Equal",
    "Elements Should Match",
    "Get Child Elements",
    "Get Element",
    "Get Element Attribute",
    "Get Element Attributes",
    "Get Element Count",
    "Get Element Text",
    "Get Elements",
    "Get Elements Texts",
    "Log Element",
    "Parse Xml",
    "Remove Element",
    "Remove Element Attribute",
    "Remove Element Attributes",
    "Remove Elements",
    "Remove Elements Attribute",
    "Remove Elements Attributes",
    "Save Xml",
    "Set Element Attribute",
    "Set Element Tag",
    "Set Element Text",
    "Set Elements Attribute",
    "Set Elements Tag",
    "Set Elements Text",
];

// =============================================================================
// Combined catalog
// =============================================================================

/// All stdlib keyword lists paired with the virtual library name used in the
/// synthetic path. Order matters only for uniqueness — BuiltIn first so it
/// wins when duplicate names appear across libraries.
const ALL_LIBRARIES: &[(&str, &[&str])] = &[
    ("BuiltIn", BUILTIN_KEYWORDS),
    ("Collections", COLLECTIONS_KEYWORDS),
    ("String", STRING_KEYWORDS),
    ("OperatingSystem", OPERATING_SYSTEM_KEYWORDS),
    ("Process", PROCESS_KEYWORDS),
    ("DateTime", DATETIME_KEYWORDS),
    ("XML", XML_KEYWORDS),
];

// =============================================================================
// Symbol / ParsedFile construction
// =============================================================================

fn keyword_sym(name: &str, library: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("# {library}.{name}")),
        doc_comment: None,
        scope_path: Some(format!("robot-builtin::{library}")),
        parent_index: None,
    }
}

fn build_parsed_file(virtual_path: String, symbols: Vec<ExtractedSymbol>) -> ParsedFile {
    let n = symbols.len();
    ParsedFile {
        path: virtual_path,
        language: "robot".to_string(),
        content_hash: format!("robot-builtin-{n}"),
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

fn synthesize_stdlib() -> Vec<ParsedFile> {
    ALL_LIBRARIES
        .iter()
        .flat_map(|(lib, keywords)| {
            keywords.iter().map(move |kw| {
                let path = format!("ext:robot-builtin:{lib}/{kw}.robot");
                let sym = keyword_sym(kw, lib);
                build_parsed_file(path, vec![sym])
            })
        })
        .collect()
}

fn synthetic_dep_root() -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "robot-builtin".to_string(),
        version: String::new(),
        root: std::path::PathBuf::from("ext:robot-builtin"),
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

// =============================================================================
// Ecosystem impl
// =============================================================================

pub struct RobotBuiltinEcosystem;

impl Ecosystem for RobotBuiltinEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("robot")
    }

    fn locate_roots(&self, _ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        vec![synthetic_dep_root()]
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn parse_metadata_only(&self, _dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        Some(synthesize_stdlib())
    }
}

impl ExternalSourceLocator for RobotBuiltinEcosystem {
    fn ecosystem(&self) -> &'static str { TAG }

    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        vec![synthetic_dep_root()]
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        Some(synthesize_stdlib())
    }
}

pub fn shared_locator() -> std::sync::Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<std::sync::Arc<RobotBuiltinEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| std::sync::Arc::new(RobotBuiltinEcosystem)).clone()
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
#[path = "robot_builtin_synthetics_tests.rs"]
mod tests;
