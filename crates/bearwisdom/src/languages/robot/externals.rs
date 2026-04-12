/// Robot Framework BuiltIn library keywords and common library keywords —
/// always external (never defined inside a project).
pub(crate) const EXTERNALS: &[&str] = &[
    // -------------------------------------------------------------------------
    // Robot BuiltIn library — logging
    // -------------------------------------------------------------------------
    "Log", "Log Many", "Log To Console", "Log Variables",
    // -------------------------------------------------------------------------
    // Variable handling
    // -------------------------------------------------------------------------
    "Set Variable", "Set Suite Variable", "Set Global Variable",
    "Set Test Variable", "Get Variable Value",
    "Variable Should Exist", "Variable Should Not Exist",
    // -------------------------------------------------------------------------
    // Assertions — equality / truth
    // -------------------------------------------------------------------------
    "Should Be Equal", "Should Not Be Equal",
    "Should Be Equal As Strings", "Should Be Equal As Integers",
    "Should Be Equal As Numbers",
    "Should Be True", "Should Not Be True",
    "Should Be Empty", "Should Not Be Empty",
    // -------------------------------------------------------------------------
    // Assertions — containment / matching
    // -------------------------------------------------------------------------
    "Should Contain", "Should Not Contain",
    "Should Contain X Times",
    "Should Match", "Should Match Regexp", "Should Not Match Regexp",
    "Should Start With", "Should End With",
    "Length Should Be",
    // -------------------------------------------------------------------------
    // Assertions — type checks
    // -------------------------------------------------------------------------
    "Should Be String", "Should Be Integer", "Should Be Boolean",
    "Should Be Decimal", "Should Be Number",
    // -------------------------------------------------------------------------
    // Control flow
    // -------------------------------------------------------------------------
    "Run Keyword", "Run Keyword If", "Run Keyword And Return",
    "Run Keyword And Expect Error", "Run Keyword And Ignore Error",
    "Wait Until Keyword Succeeds", "Repeat Keyword", "Run Keywords",
    "Return From Keyword", "Pass Execution", "Fatal Error", "Fail",
    "Skip", "Skip If",
    // -------------------------------------------------------------------------
    // Type conversion
    // -------------------------------------------------------------------------
    "Convert To Integer", "Convert To String", "Convert To Number",
    "Convert To Boolean",
    // -------------------------------------------------------------------------
    // String operations
    // -------------------------------------------------------------------------
    "Get Length", "Get Substring",
    "Get Line Count", "Get Lines Matching Pattern",
    "Get Lines Matching Regexp", "Get Lines Containing String",
    "Remove String", "Replace String", "Replace String Using Regexp",
    "Split String", "Split String From Right", "Split String To Characters",
    "Fetch From Left", "Fetch From Right",
    "Generate Random String",
    "Get Regexp Matches", "Get Match Count",
    // -------------------------------------------------------------------------
    // Test metadata
    // -------------------------------------------------------------------------
    "Check Test Case", "Check Log Message", "Check Test Doc", "Check Test Tags",
    // -------------------------------------------------------------------------
    // Environment variables
    // -------------------------------------------------------------------------
    "Environment Variable Should Be Set",
    "Set Environment Variable", "Remove Environment Variable",
    "Get Environment Variable",
    // -------------------------------------------------------------------------
    // Evaluation / misc
    // -------------------------------------------------------------------------
    "Evaluate", "Call Method", "Create List", "Create Dictionary",
    "Sleep", "Catenate", "Get Time", "Get Count",
    "Import Library", "Import Resource", "Set Library Search Order",
    "No Operation", "Comment",
    // -------------------------------------------------------------------------
    // OperatingSystem library
    // -------------------------------------------------------------------------
    "Run", "Run And Return Rc", "Run And Return Rc And Output",
    "Start Process", "Wait For Process",
    "Process Should Be Running", "Process Should Be Stopped",
    "Terminate Process",
    "Get File", "Create File", "Create Binary File", "Append To File", "Touch",
    "Copy File", "Move File", "Remove File", "Remove Files",
    "Copy Directory", "Move Directory", "Remove Directory", "Empty Directory",
    "File Should Exist", "File Should Not Exist",
    "Directory Should Exist", "Directory Should Not Exist",
    "File Should Be Empty", "File Should Not Be Empty",
    "Count Files In Directory", "Count Directories In Directory",
    "List Files In Directory", "List Directories In Directory",
    // -------------------------------------------------------------------------
    // Collections library — lists
    // -------------------------------------------------------------------------
    "Append To List", "Insert Into List", "Combine Lists",
    "Remove From List", "Remove Duplicates",
    "Get From List", "Set List Value", "Get Slice From List",
    "Sort List", "Reverse List",
    "Lists Should Be Equal",
    "List Should Contain Value", "List Should Not Contain Value",
    // -------------------------------------------------------------------------
    // Collections library — dictionaries
    // -------------------------------------------------------------------------
    "Get From Dictionary", "Set To Dictionary",
    "Dictionary Should Contain Key", "Dictionary Should Not Contain Key",
    "Dictionary Should Contain Value",
    "Dictionaries Should Be Equal",
    "Get Dictionary Keys", "Get Dictionary Values", "Get Dictionary Items",
    "Pop From Dictionary", "Keep In Dictionary",
    "Remove From Dictionary", "Copy Dictionary",
    // -------------------------------------------------------------------------
    // SeleniumLibrary
    // -------------------------------------------------------------------------
    "Open Browser", "Close Browser", "Go To",
    "Click Element", "Click Button", "Click Link",
    "Input Text", "Select From List By Value",
    "Wait Until Element Is Visible", "Page Should Contain",
    "Element Should Be Visible", "Get Text", "Get Title",
    "Execute JavaScript", "Capture Page Screenshot",
];

