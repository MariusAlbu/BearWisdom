// =============================================================================
// ada/primitives.rs — Ada primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for Ada.
pub(crate) const PRIMITIVES: &[&str] = &[
    // Ada.Text_IO
    "Ada.Text_IO.Put_Line", "Ada.Text_IO.Put",
    "Ada.Text_IO.Get_Line", "Ada.Text_IO.New_Line",
    // Ada.Integer_Text_IO / Ada.Float_Text_IO
    "Ada.Integer_Text_IO.Put", "Ada.Integer_Text_IO.Get",
    "Ada.Float_Text_IO.Put", "Ada.Float_Text_IO.Get",
    // Ada.Strings
    "Ada.Strings.Fixed.Trim", "Ada.Strings.Fixed.Index",
    "Ada.Strings.Fixed.Replace_Slice",
    "Ada.Strings.Fixed.Head", "Ada.Strings.Fixed.Tail",
    "Ada.Strings.Unbounded",
    // Ada.Containers
    "Ada.Containers.Vectors",
    "Ada.Containers.Doubly_Linked_Lists",
    "Ada.Containers.Hashed_Maps",
    "Ada.Containers.Ordered_Maps",
    "Ada.Containers.Hashed_Sets",
    "Ada.Containers.Ordered_Sets",
    "Ada.Containers.Indefinite_Ordered_Maps",
    "Ada.Containers.Indefinite_Vectors",
    // Ada.Directories
    "Ada.Directories.Exists",
    "Ada.Directories.Create_Directory",
    "Ada.Directories.Delete_File",
    "Ada.Directories.Rename",
    "Ada.Directories.Full_Name",
    "Ada.Directories.Simple_Name",
    "Ada.Directories.Containing_Directory",
    "Ada.Directories.Extension",
    // Ada.Exceptions
    "Ada.Exceptions.Exception_Message",
    "Ada.Exceptions.Exception_Name",
    "Ada.Exceptions.Raise_Exception",
    // Ada.Calendar
    "Ada.Calendar.Clock",
    // Ada.Command_Line
    "Ada.Command_Line.Argument_Count",
    "Ada.Command_Line.Argument",
    "Ada.Command_Line.Command_Name",
    // Ada.Environment_Variables
    "Ada.Environment_Variables.Value",
    "Ada.Environment_Variables.Exists",
    // Ada.Numerics
    "Ada.Numerics.Float_Random.Random",
    "Ada.Numerics.Discrete_Random",
    "Ada.Numerics.Elementary_Functions.Sqrt",
    "Ada.Numerics.Elementary_Functions.Sin",
    "Ada.Numerics.Elementary_Functions.Cos",
    "Ada.Numerics.Elementary_Functions.Log",
    "Ada.Numerics.Elementary_Functions.Exp",
    // standard types
    "Integer", "Natural", "Positive", "Float",
    "Long_Float", "Long_Long_Float",
    "Character", "Wide_Character", "Wide_Wide_Character",
    "Boolean", "Duration",
    "String", "Wide_String", "Wide_Wide_String",
    "Access", "Unbounded_String", "Standard",
    "True", "False", "null", "others",
    // common Ada library shortcuts
    "Trace.Debug", "Trace.Warning", "Trace.Error",
    "Trace.Always", "Trace.Info",
    "TTY.Emph", "TTY.Terminal",
    "TIO.Put_Line", "TIO.Put", "TIO.Get_Line", "TIO.New_Line",
    "TOML", "AAA.Strings",
    "To_Lower_Case", "To_Upper_Case",
    "echo", "Tail", "Define_Switch",
];
