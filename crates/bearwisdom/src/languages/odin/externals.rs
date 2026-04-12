// =============================================================================
// odin/externals.rs — Odin runtime globals and standard library package names
// =============================================================================

/// Odin standard library package names that are always external.
///
/// Odin's stdlib is accessed via package qualifiers (e.g. `fmt.println`).
/// These are the top-level package names from `core:`, `vendor:`, and `base:`.
pub(crate) const EXTERNALS: &[&str] = &[
    // core: packages
    "fmt",
    "mem",
    "os",
    "os2",
    "strings",
    "strconv",
    "slice",
    "sort",
    "math",
    "unicode",
    "unicode/utf8",
    "io",
    "bufio",
    "bytes",
    "sync",
    "sync/atomic",
    "time",
    "path",
    "path/filepath",
    "log",
    "reflect",
    "runtime",
    "hash",
    "hash/crc32",
    "hash/fnv",
    "compress/zlib",
    "compress/gzip",
    "encoding/json",
    "encoding/csv",
    "encoding/base64",
    "encoding/hex",
    "net",
    "net/http",
    "crypto/rand",
    "crypto/md5",
    "crypto/sha1",
    "crypto/sha256",
    // base: packages
    "base/intrinsics",
    "base/builtin",
    // vendor: packages
    "vendor:raylib",
    "vendor:sdl2",
    "vendor:glfw",
    "vendor:opengl",
    "vendor:vulkan",
    "vendor:stb/image",
    "vendor:stb/truetype",
    "vendor:imgui",
    // intrinsics
    "intrinsics",
];

