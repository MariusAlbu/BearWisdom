// =============================================================================
// csharp/predicates.rs — C# builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// C# / .NET built-in types that are always in scope without a `using` directive.
///
/// Covers:
/// - C# primitive type aliases (`string`, `int`, `bool`, etc.) and their BCL
///   counterparts (`String`, `Int32`, `Boolean`, etc.)
/// - Core BCL types universally available in any .NET project
/// - LINQ entry points (`Enumerable`, `Queryable`) — nearly always present
/// - System/Microsoft namespace prefixes when used as targets
///
/// This function is the fallback for when no `ProjectContext` is available (unit
/// tests, partial indexing). With a full `ProjectContext` the resolver walks the
/// file's using directives instead, which is more precise.
pub(super) fn is_csharp_builtin(name: &str) -> bool {
    matches!(
        name,
        // -----------------------------------------------------------------------
        // C# primitive keyword aliases (always in scope, no import needed)
        // -----------------------------------------------------------------------
        "string" | "int" | "bool" | "float" | "double" | "decimal"
            | "object" | "void" | "byte" | "char" | "long" | "short"
            | "uint" | "ulong" | "ushort" | "sbyte" | "nint" | "nuint"
            | "dynamic"
        // -----------------------------------------------------------------------
        // System namespace — BCL core types (implicitly available in all .NET projects)
        // -----------------------------------------------------------------------
        | "String" | "Int32" | "Int64" | "Int16" | "UInt32" | "UInt64"
            | "UInt16" | "Byte" | "SByte" | "Char" | "Double" | "Single"
            | "Decimal" | "Boolean" | "Object" | "Void"
            | "IntPtr" | "UIntPtr"
        // Console, Math, Convert — used without import in virtually every C# file
        | "Console" | "Math" | "Convert" | "Environment" | "GC"
        // Common value types
        | "DateTime" | "DateTimeOffset" | "TimeSpan" | "DateOnly" | "TimeOnly"
        | "Guid" | "Uri"
        // Nullable
        | "Nullable"
        // Tuples
        | "Tuple" | "ValueTuple"
        // Common reference types
        | "Array" | "String" | "Exception" | "Attribute" | "Enum" | "Delegate"
        | "EventArgs" | "EventHandler" | "Type" | "Action" | "Func" | "Predicate"
        | "Comparer" | "EqualityComparer"
        // -----------------------------------------------------------------------
        // System.Collections / System.Collections.Generic
        // -----------------------------------------------------------------------
        | "List" | "Dictionary" | "HashSet" | "SortedDictionary" | "SortedSet"
        | "SortedList" | "Queue" | "Stack" | "LinkedList" | "ObservableCollection"
        | "ReadOnlyCollection" | "ReadOnlyDictionary"
        | "IEnumerable" | "IEnumerator" | "ICollection" | "IList" | "IDictionary"
        | "IReadOnlyCollection" | "IReadOnlyList" | "IReadOnlyDictionary"
        | "ISet" | "IReadOnlySet"
        | "KeyValuePair"
        // -----------------------------------------------------------------------
        // System.Threading / System.Threading.Tasks
        // -----------------------------------------------------------------------
        | "Task" | "ValueTask" | "CancellationToken" | "CancellationTokenSource"
        | "Thread" | "Mutex" | "Semaphore" | "SemaphoreSlim" | "Monitor"
        | "Interlocked" | "Volatile"
        // -----------------------------------------------------------------------
        // System.Memory / System.Buffers
        // -----------------------------------------------------------------------
        | "Span" | "ReadOnlySpan" | "Memory" | "ReadOnlyMemory"
        | "ArraySegment" | "MemoryPool"
        // -----------------------------------------------------------------------
        // Core interfaces
        // -----------------------------------------------------------------------
        | "IDisposable" | "IAsyncDisposable"
        | "IComparable" | "IEquatable" | "ICloneable" | "IConvertible"
        | "IFormattable" | "IParsable"
        | "ILogger" | "ILoggerFactory" | "ILoggerProvider"
        | "IServiceProvider" | "IServiceCollection" | "IServiceScope"
        // -----------------------------------------------------------------------
        // LINQ entry points (System.Linq — imported by default in modern .NET SDK)
        // -----------------------------------------------------------------------
        | "Enumerable" | "Queryable" | "ParallelEnumerable"
        // -----------------------------------------------------------------------
        // System/Microsoft top-level namespace prefixes
        // -----------------------------------------------------------------------
        | "System" | "Microsoft"
    )
}

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class" | "struct"),
        EdgeKind::Implements => matches!(sym_kind, "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "struct" | "interface" | "enum" | "type_alias" | "namespace" | "delegate"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "struct"),
        _ => true,
    }
}

/// Fallback for when no ProjectContext is available.
/// Only recognizes the two always-present .NET SDK prefixes.
pub(super) fn is_external_namespace_fallback(ns: &str) -> bool {
    ns.starts_with("System") || ns.starts_with("Microsoft")
}
