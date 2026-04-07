// =============================================================================
// robot/primitives.rs — Robot Framework primitive and built-in keywords
// =============================================================================

/// Primitive and built-in type/function names for Robot Framework.
pub(crate) const PRIMITIVES: &[&str] = &[
    // BuiltIn library
    "Should Be Equal", "Should Not Be Equal",
    "Should Be True", "Should Be False",
    "Should Contain", "Should Not Contain",
    "Should Be Empty", "Should Not Be Empty",
    "Should Match", "Should Match Regexp",
    "Should Start With", "Should End With",
    "Length Should Be",
    "Log", "Log Many", "Log To Console", "Comment",
    "Set Variable", "Set Test Variable", "Set Suite Variable", "Set Global Variable",
    "Get Variable Value",
    "Variable Should Exist", "Variable Should Not Exist",
    "Create List", "Create Dictionary",
    "Append To List", "Remove From List",
    "Get From List", "Get From Dictionary",
    "Set To Dictionary", "Pop From Dictionary",
    "Dictionary Should Contain Key", "Dictionary Should Not Contain Key",
    "Lists Should Be Equal",
    "List Should Contain Value", "List Should Not Contain Value",
    "Convert To String", "Convert To Integer",
    "Convert To Number", "Convert To Boolean", "Convert To Bytes",
    "Set Tags", "Remove Tags",
    "Fail", "Fatal Error",
    "Pass Execution", "Pass Execution If",
    "Skip", "Skip If",
    "Return From Keyword",
    "Run Keyword", "Run Keyword If", "Run Keyword Unless",
    "Run Keyword And Ignore Error", "Run Keyword And Return Status",
    "Run Keyword And Expect Error", "Run Keyword And Return",
    "Run Keyword If All Tests Passed", "Run Keywords",
    "Wait Until Keyword Succeeds", "Repeat Keyword",
    "Sleep", "No Operation",
    "Catenate", "Get Time", "Get Count",
    "Should Be Equal As Strings", "Should Be Equal As Integers",
    "Should Be Equal As Numbers",
    "Should Be Greater Than", "Should Be Less Than",
    "Import Library", "Import Resource",
    "Evaluate", "Call Method", "Set Library Search Order",
    "Keyword Should Exist", "Get Library Instance",
    "Check Test Case", "Check Log Message",
    // flow control syntax
    "FOR", "END", "IF", "ELSE", "ELSE IF", "WHILE",
    "TRY", "EXCEPT", "FINALLY", "CONTINUE", "BREAK", "RETURN", "VAR",
    // setting markers
    "...", "[Arguments]", "[Return]", "[Documentation]",
    "[Tags]", "[Setup]", "[Teardown]", "[Timeout]", "[Template]",
    // type markers
    "Integer", "Boolean", "String",
    // OperatingSystem library
    "Create File", "File Should Exist", "File Should Not Exist",
    "Directory Should Exist", "Append To File", "Get File",
    "Get Binary File", "List Directory",
    // XML library
    "Element Should Exist", "Element Should Not Exist",
    "Element Attribute Should Be", "Element Text Should Be",
    "Get Element", "Get Elements", "Get Element Count",
    "Get Element Text", "Get Element Attribute",
    // SeleniumLibrary
    "Open Browser", "Close Browser", "Go To",
    "Click Element", "Input Text", "Select From List",
    "Get Text", "Get Value",
    "Page Should Contain", "Page Should Not Contain",
    "Element Should Be Visible", "Element Should Not Be Visible",
    "Wait Until Element Is Visible", "Wait Until Element Is Not Visible",
    "Wait Until Page Contains", "Wait Until Page Contains Element",
    "Capture Page Screenshot", "Execute Javascript",
    "Get Title", "Get Location",
];
