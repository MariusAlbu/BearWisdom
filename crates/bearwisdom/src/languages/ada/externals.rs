/// Ada standard library packages and GNAT runtime extensions — always external
/// (never defined inside a project).
pub(crate) const EXTERNALS: &[&str] = &[
    // -------------------------------------------------------------------------
    // Ada standard library — top-level packages
    // -------------------------------------------------------------------------
    "Ada.Text_IO",
    "Ada.Integer_Text_IO",
    "Ada.Float_Text_IO",
    "Ada.Strings",
    "Ada.Strings.Fixed",
    "Ada.Strings.Unbounded",
    "Ada.Strings.Maps",
    "Ada.Characters.Handling",
    "Ada.Characters.Latin_1",
    "Ada.Containers",
    "Ada.Containers.Vectors",
    "Ada.Containers.Doubly_Linked_Lists",
    "Ada.Containers.Hashed_Maps",
    "Ada.Containers.Ordered_Maps",
    "Ada.Containers.Hashed_Sets",
    "Ada.Containers.Ordered_Sets",
    "Ada.Directories",
    "Ada.Environment_Variables",
    "Ada.Exceptions",
    "Ada.Calendar",
    "Ada.Calendar.Formatting",
    "Ada.Command_Line",
    "Ada.Numerics",
    "Ada.Numerics.Float_Random",
    "Ada.Numerics.Discrete_Random",
    "Ada.Numerics.Elementary_Functions",
    "Ada.Sequential_IO",
    "Ada.Direct_IO",
    "Ada.Streams",
    "Ada.Streams.Stream_IO",
    "Ada.Tags",
    "Ada.Unchecked_Deallocation",
    "Ada.Unchecked_Conversion",
    "Ada.Synchronous_Task_Control",
    "Ada.Task_Identification",
    "Ada.Task_Attributes",
    // -------------------------------------------------------------------------
    // System package
    // -------------------------------------------------------------------------
    "System",
    "System.Address_Image",
    "System.Storage_Elements",
    // -------------------------------------------------------------------------
    // Interfaces package
    // -------------------------------------------------------------------------
    "Interfaces",
    "Interfaces.C",
    "Interfaces.C.Strings",
    "Interfaces.Fortran",
    "Interfaces.COBOL",
    // -------------------------------------------------------------------------
    // GNAT runtime extensions
    // -------------------------------------------------------------------------
    "GNAT",
    "GNAT.OS_Lib",
    "GNAT.Strings",
    "GNAT.Command_Line",
    "GNAT.IO",
    "GNAT.Directory_Operations",
    "GNAT.Exception_Actions",
    "GNAT.Regpat",
    "GNAT.Source_Info",
    "GNAT.Traceback",
];
