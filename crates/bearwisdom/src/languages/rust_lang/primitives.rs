// =============================================================================
// rust_lang/primitives.rs — Rust primitive types
// =============================================================================

/// Primitive and built-in type names for Rust.
/// Includes numeric primitives, standard library prelude types, generic type
/// parameter names, and function trait families.
pub(crate) const PRIMITIVES: &[&str] = &[
    // Numeric primitives
    "i8", "i16", "i32", "i64", "i128",
    "u8", "u16", "u32", "u64", "u128",
    "f32", "f64", "bool", "char", "str", "usize", "isize",
    // Prelude types
    "String", "Vec", "Option", "Result", "Box", "Rc", "Arc", "Self",
    // Smart pointers and wrappers
    "Mutex", "RwLock", "RefCell", "Cell", "OnceCell", "OnceLock",
    "Cow", "Pin", "Weak",
    // Collections
    "HashMap", "HashSet", "BTreeMap", "BTreeSet",
    "LinkedList", "VecDeque", "BinaryHeap",
    // Iterators
    "Iterator", "IntoIterator", "FromIterator", "ExactSizeIterator",
    "DoubleEndedIterator",
    // Traits (prelude + common)
    "Clone", "Copy", "Debug", "Display", "Default", "Drop",
    "PartialEq", "Eq", "PartialOrd", "Ord", "Hash",
    "Send", "Sync", "Sized", "Unpin",
    "From", "Into", "TryFrom", "TryInto", "AsRef", "AsMut",
    "Deref", "DerefMut",
    "Serialize", "Deserialize",
    // IO
    "Read", "Write", "BufRead", "BufReader", "BufWriter",
    "File", "Path", "PathBuf", "OsStr", "OsString",
    // Error handling
    "Error",
    // Formatting
    "Formatter",
    // Async
    "Future", "Stream", "Sink", "Poll", "Waker",
    // Generic type parameters
    "T", "U", "K", "V", "E", "R", "S", "P", "A", "B", "C", "D", "N", "M",
    // Function traits
    "Fn", "FnMut", "FnOnce",
];
