// =============================================================================
// zig/externals.rs — Zig standard library and runtime globals
// =============================================================================

/// Zig standard library top-level names that are always external.
///
/// These are the names bound by `const std = @import("std")` and accessed
/// as `std.X`. The extractor strips `std.` qualifiers for Calls edges, so
/// these bare names also appear without prefix and need to be classified.
pub(crate) const EXTERNALS: &[&str] = &[
    // std root namespaces
    "std",
    "builtin",
    // std.debug
    "debug",
    // std.mem
    "mem",
    // std.heap
    "heap",
    // std.fs / std.io
    "fs",
    "io",
    // std.fmt
    "fmt",
    // std.math
    "math",
    // std.os
    "os",
    // std.time
    "time",
    // std.process
    "process",
    // std.testing
    "testing",
    // std.meta
    "meta",
    // std.ascii
    "ascii",
    // std.unicode
    "unicode",
    // std.json
    "json",
    // std.log
    "log",
    // std.rand
    "rand",
    // std.crypto
    "crypto",
    // std.net
    "net",
    // std.http
    "http",
    // std.Thread / std.Mutex / std.atomic
    "Thread",
    "Mutex",
    "RwLock",
    "atomic",
    // std.ArrayList / std.ArrayListUnmanaged
    "ArrayList",
    "ArrayListUnmanaged",
    // std.HashMap / std.StringHashMap / std.AutoHashMap
    "HashMap",
    "StringHashMap",
    "AutoHashMap",
    "StringArrayHashMap",
    "AutoArrayHashMap",
    // std.BufMap / std.BufSet
    "BufMap",
    "BufSet",
    // std.PriorityQueue / std.TailQueue
    "PriorityQueue",
    "TailQueue",
    "DoublyLinkedList",
    "SinglyLinkedList",
    // std.SegmentedList / std.MultiArrayList
    "SegmentedList",
    "MultiArrayList",
    // std.mem.Allocator (accessed as type)
    "Allocator",
    // std.io.Writer / Reader
    "Writer",
    "Reader",
    "AnyWriter",
    "AnyReader",
];

