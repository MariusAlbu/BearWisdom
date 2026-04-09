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
    "Set Test Variable", "Get Variable Value", "Variable Should Exist",
    // -------------------------------------------------------------------------
    // Assertions
    // -------------------------------------------------------------------------
    "Should Be Equal", "Should Not Be Equal",
    "Should Be True", "Should Be Empty", "Should Not Be Empty",
    "Should Contain", "Should Not Contain",
    "Should Match", "Should Match Regexp",
    "Length Should Be",
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
    // Evaluation / misc
    // -------------------------------------------------------------------------
    "Evaluate", "Call Method", "Create List", "Create Dictionary",
    "Append To List", "Remove From List", "Get From List",
    "Get From Dictionary", "Set To Dictionary",
    "Sleep", "Catenate", "Get Time", "Get Count",
    "Import Library", "Import Resource", "Set Library Search Order",
    "No Operation", "Comment",
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
