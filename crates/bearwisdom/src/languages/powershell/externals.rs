/// Runtime globals always external for PowerShell.
///
/// These are .NET instance/static method names commonly called on objects in
/// PowerShell scripts (e.g., `$list.Add(...)`, `[string]::IsNullOrEmpty(...)`).
/// They are never defined in project code and are not cmdlets (those are in
/// primitives.rs) — they come from the .NET BCL at runtime.
pub(crate) const EXTERNALS: &[&str] = &[
    // System.Object / common .NET instance methods
    "ToString", "GetType", "GetHashCode", "Equals", "Dispose",
    // System.Collections (IList, IDictionary, ISet, …)
    "Add", "Remove", "Contains", "ContainsKey", "ContainsValue",
    "Clear", "Count", "Keys", "Values", "Item",
    "Insert", "RemoveAt", "IndexOf", "TrimExcess",
    // System.String static / instance
    "IsNullOrEmpty", "IsNullOrWhiteSpace", "Join", "Split", "Replace",
    "Trim", "TrimStart", "TrimEnd", "ToUpper", "ToLower",
    "StartsWith", "EndsWith", "Substring", "IndexOf", "LastIndexOf",
    "PadLeft", "PadRight", "Format", "Concat",
    // System.Collections.Generic.List<T>
    "AddRange", "Sort", "Reverse", "Find", "FindAll", "ForEach",
    "ToArray", "AsReadOnly",
    // System.IO.Path static
    "Combine", "GetFileName", "GetFileNameWithoutExtension",
    "GetExtension", "GetDirectoryName", "GetFullPath",
    // Hashtable / PSCustomObject
    "Clone", "CopyTo",
    // Runspace / PowerShell automation
    "Invoke", "BeginInvoke", "EndInvoke", "Stop",
    "SetVariable", "GetVariable", "AddScript", "AddCommand",
    "AddParameter", "AddArgument",
    // Exception / error record
    "ThrowTerminatingError", "WriteError", "WriteObject",
    "WriteVerbose", "WriteWarning", "WriteDebug", "WriteProgress",
    "ShouldProcess", "ShouldContinue",
    // WinForms / WPF event glue
    "Add_Click", "Add_Load", "Add_Shown", "Add_Closing", "Add_Closed",
    "Add_TextChanged", "Add_SelectedIndexChanged", "Add_KeyDown",
    // WPF dependency properties / common control members
    "Visibility", "Visible", "Collapsed", "Hidden",
    "Text", "Content", "IsEnabled", "IsChecked", "IsSelected",
    "Foreground", "Background", "FontSize", "FontWeight",
    "Width", "Height", "Margin", "Padding",
    "HorizontalAlignment", "VerticalAlignment",
    "Dispatcher",
    // Common dialog results
    "OK", "Cancel",
    // Misc .NET patterns
    "new", "GetValue", "SetValue", "GetFields", "GetProperties",
    "GetMethods", "InvokeMember",
];

