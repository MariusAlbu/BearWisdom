use std::collections::HashSet;

/// Runtime globals always external for C#.
///
/// These are well-known .NET BCL / framework type names that reliably appear as
/// unresolved references in real C# projects.  They supplement `is_csharp_builtin`
/// (which handles the always-in-scope primitive aliases and universal BCL types)
/// for cases where no `ProjectContext` is available.
///
/// Keep this list to types that appear *very* frequently across projects and are
/// unambiguously .NET / Microsoft.  Framework-specific types that are only present
/// with a particular NuGet package belong in `framework_globals` instead.
pub(crate) const EXTERNALS: &[&str] = &[
    // System.IO
    "File", "Directory", "Path", "Stream", "StreamReader", "StreamWriter",
    "FileStream", "MemoryStream", "BinaryReader", "BinaryWriter",
    "FileInfo", "DirectoryInfo", "DriveInfo",
    "TextReader", "TextWriter", "StringReader", "StringWriter",
    // System.Text
    "StringBuilder", "Encoding", "Regex",
    // System.Text.Json / Newtonsoft shim
    "JsonSerializer", "JsonDocument", "JsonElement", "JsonNode",
    // System.Net.Http
    "HttpClient", "HttpRequestMessage", "HttpResponseMessage",
    "HttpContent", "HttpMethod", "HttpStatusCode",
    // System.Reflection
    "Assembly", "MethodInfo", "PropertyInfo", "FieldInfo", "ParameterInfo",
    "ConstructorInfo", "TypeInfo", "MemberInfo",
    // System.Diagnostics
    "Debug", "Trace", "Stopwatch", "Process",
    // System.Runtime.CompilerServices
    "CallerMemberName", "CallerFilePath", "CallerLineNumber",
    // System.ComponentModel
    "PropertyChangedEventArgs", "PropertyChangedEventHandler",
    "INotifyPropertyChanged", "INotifyCollectionChanged",
    // System.Linq.Expressions
    "Expression", "LambdaExpression",
    // System.Collections.Concurrent
    "ConcurrentDictionary", "ConcurrentBag", "ConcurrentQueue", "ConcurrentStack",
    "BlockingCollection",
    // Common async patterns
    "AsyncLocal",
    // System.Security.Claims
    "ClaimsPrincipal", "ClaimsIdentity", "Claim",
    // Common attributes
    "Obsolete", "Serializable", "NonSerialized", "Flags",
    "DllImport", "StructLayout", "FieldOffset",
    "MethodImpl", "MethodImplOptions",
    // System.Runtime
    "RuntimeInformation", "OSPlatform",
];

/// Dependency-gated framework globals for C#.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    // xUnit / NUnit / MSTest
    for dep in ["xunit", "nunit", "MSTest"] {
        if deps.contains(dep) {
            globals.extend(&["Assert", "Fact", "Theory", "TestMethod", "SetUp", "TearDown"]);
            break;
        }
    }

    globals
}
