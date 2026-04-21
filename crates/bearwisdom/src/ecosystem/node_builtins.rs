// =============================================================================
// ecosystem/node_builtins.rs — Node.js built-in module stubs (stdlib)
//
// Provides synthetic symbols for Node's built-in modules (`fs`, `path`,
// `http`, `os`, `util`, `crypto`, `stream`, `events`, `child_process`,
// `url`, `querystring`, `buffer`, `process`, `timers`, `assert`, `console`).
//
// Purpose: when a JS/TS project does `require('fs')` or `import fs from
// 'node:fs'` but has no `@types/node` installed, the reference would
// otherwise be unresolved. This ecosystem intercepts bare module names and
// `node:` prefixed names, emitting synthetic ParsedFile entries with the
// module's top-level surface so the resolver finds a target.
//
// Activation: any JS or TS file present in the project. Degrades
// transparently if a richer `@types/node` package is already indexed via
// the npm ecosystem — npm registers first, so its symbols win on lookup.
//
// See ts_lib_dom.rs and godot_api.rs for the peer pattern this follows.
// =============================================================================

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;
use std::path::Path;

pub const ID: EcosystemId = EcosystemId::new("node-builtins");
const LEGACY_ECOSYSTEM_TAG: &str = "node-builtins";
const LANGUAGES: &[&str] = &["javascript", "typescript", "tsx"];

// =============================================================================
// Module surface definitions
// =============================================================================

/// A single top-level member of a Node built-in module.
struct Member {
    name: &'static str,
    kind: SymbolKind,
}

const fn fn_(name: &'static str) -> Member {
    Member { name, kind: SymbolKind::Function }
}
const fn cls(name: &'static str) -> Member {
    Member { name, kind: SymbolKind::Class }
}
const fn var(name: &'static str) -> Member {
    Member { name, kind: SymbolKind::Variable }
}

struct ModuleSpec {
    /// Canonical module name without `node:` prefix.
    name: &'static str,
    members: &'static [Member],
}

// ---------------------------------------------------------------------------
// Per-module member lists
// ---------------------------------------------------------------------------

static FS_MEMBERS: &[Member] = &[
    fn_("readFile"),
    fn_("writeFile"),
    fn_("readFileSync"),
    fn_("writeFileSync"),
    fn_("stat"),
    fn_("statSync"),
    fn_("readdir"),
    fn_("readdirSync"),
    fn_("mkdir"),
    fn_("rmdir"),
    fn_("unlink"),
    fn_("access"),
    fn_("existsSync"),
    fn_("createReadStream"),
    fn_("createWriteStream"),
    var("promises"),
];

static PATH_MEMBERS: &[Member] = &[
    fn_("join"),
    fn_("resolve"),
    fn_("dirname"),
    fn_("basename"),
    fn_("extname"),
    fn_("relative"),
    fn_("normalize"),
    fn_("parse"),
    fn_("format"),
    fn_("isAbsolute"),
    var("sep"),
    var("delimiter"),
];

static HTTP_MEMBERS: &[Member] = &[
    fn_("createServer"),
    fn_("request"),
    fn_("get"),
    cls("Server"),
    cls("Agent"),
    cls("IncomingMessage"),
    cls("ServerResponse"),
];

// https mirrors http surface.
static HTTPS_MEMBERS: &[Member] = &[
    fn_("createServer"),
    fn_("request"),
    fn_("get"),
    cls("Server"),
    cls("Agent"),
    cls("IncomingMessage"),
    cls("ServerResponse"),
];

static OS_MEMBERS: &[Member] = &[
    fn_("platform"),
    fn_("arch"),
    fn_("cpus"),
    fn_("totalmem"),
    fn_("freemem"),
    fn_("hostname"),
    fn_("homedir"),
    fn_("tmpdir"),
    fn_("type"),
    fn_("release"),
    fn_("userInfo"),
];

static UTIL_MEMBERS: &[Member] = &[
    fn_("promisify"),
    fn_("inspect"),
    fn_("format"),
    fn_("deprecate"),
    fn_("callbackify"),
    var("types"),
];

static CRYPTO_MEMBERS: &[Member] = &[
    fn_("createHash"),
    fn_("createHmac"),
    fn_("randomBytes"),
    fn_("randomUUID"),
    fn_("pbkdf2"),
    fn_("pbkdf2Sync"),
    fn_("scrypt"),
    fn_("createCipheriv"),
    fn_("createDecipheriv"),
];

static STREAM_MEMBERS: &[Member] = &[
    cls("Readable"),
    cls("Writable"),
    cls("Transform"),
    cls("Duplex"),
    fn_("pipeline"),
    fn_("finished"),
];

static EVENTS_MEMBERS: &[Member] = &[
    cls("EventEmitter"),
    fn_("once"),
    fn_("on"),
];

static CHILD_PROCESS_MEMBERS: &[Member] = &[
    fn_("spawn"),
    fn_("exec"),
    fn_("execSync"),
    fn_("fork"),
    fn_("execFile"),
];

static URL_MEMBERS: &[Member] = &[
    cls("URL"),
    cls("URLSearchParams"),
    fn_("fileURLToPath"),
    fn_("pathToFileURL"),
];

static QUERYSTRING_MEMBERS: &[Member] = &[
    fn_("parse"),
    fn_("stringify"),
    fn_("escape"),
    fn_("unescape"),
];

static BUFFER_MEMBERS: &[Member] = &[
    cls("Buffer"),
    cls("Blob"),
    var("constants"),
];

static PROCESS_MEMBERS: &[Member] = &[
    var("env"),
    var("argv"),
    fn_("exit"),
    fn_("cwd"),
    fn_("chdir"),
    var("platform"),
    var("version"),
    var("versions"),
    var("pid"),
];

static TIMERS_MEMBERS: &[Member] = &[
    fn_("setTimeout"),
    fn_("setInterval"),
    fn_("setImmediate"),
    fn_("clearTimeout"),
    fn_("clearInterval"),
    fn_("clearImmediate"),
];

static ASSERT_MEMBERS: &[Member] = &[
    fn_("ok"),
    fn_("equal"),
    fn_("strictEqual"),
    fn_("deepEqual"),
    fn_("deepStrictEqual"),
    fn_("throws"),
    fn_("rejects"),
    fn_("fail"),
];

static CONSOLE_MEMBERS: &[Member] = &[
    fn_("log"),
    fn_("error"),
    fn_("warn"),
    fn_("info"),
    fn_("debug"),
    fn_("table"),
    fn_("trace"),
    fn_("dir"),
    fn_("time"),
    fn_("timeEnd"),
    fn_("group"),
    fn_("groupEnd"),
];

static MODULES: &[ModuleSpec] = &[
    ModuleSpec { name: "fs",            members: FS_MEMBERS },
    ModuleSpec { name: "path",          members: PATH_MEMBERS },
    ModuleSpec { name: "http",          members: HTTP_MEMBERS },
    ModuleSpec { name: "https",         members: HTTPS_MEMBERS },
    ModuleSpec { name: "os",            members: OS_MEMBERS },
    ModuleSpec { name: "util",          members: UTIL_MEMBERS },
    ModuleSpec { name: "crypto",        members: CRYPTO_MEMBERS },
    ModuleSpec { name: "stream",        members: STREAM_MEMBERS },
    ModuleSpec { name: "events",        members: EVENTS_MEMBERS },
    ModuleSpec { name: "child_process", members: CHILD_PROCESS_MEMBERS },
    ModuleSpec { name: "url",           members: URL_MEMBERS },
    ModuleSpec { name: "querystring",   members: QUERYSTRING_MEMBERS },
    ModuleSpec { name: "buffer",        members: BUFFER_MEMBERS },
    ModuleSpec { name: "process",       members: PROCESS_MEMBERS },
    ModuleSpec { name: "timers",        members: TIMERS_MEMBERS },
    ModuleSpec { name: "assert",        members: ASSERT_MEMBERS },
    ModuleSpec { name: "console",       members: CONSOLE_MEMBERS },
];

// =============================================================================
// Synthesis helpers
// =============================================================================

/// Build one `ParsedFile` for a Node module. The virtual path is the
/// canonical module name; we also emit a second synthetic file under the
/// `node:` prefix so `import fs from 'node:fs'` and `require('fs')` both
/// resolve. The two files share identical symbols — resolution de-duplication
/// is handled naturally by qualified-name uniqueness at the DB level.
fn synth_module(spec: &ModuleSpec, prefix: &str) -> ParsedFile {
    let virtual_path = format!("ext:node-builtins:{}{}.d.ts", prefix, spec.name);
    let module_name = spec.name;

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();

    // Module-level namespace symbol so `import * as fs from 'fs'` resolves.
    symbols.push(ExtractedSymbol {
        name: module_name.to_string(),
        qualified_name: format!("{prefix}{module_name}"),
        kind: SymbolKind::Namespace,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("module \"{prefix}{module_name}\"")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });

    let module_parent_idx = 0usize;

    for member in spec.members {
        symbols.push(ExtractedSymbol {
            name: member.name.to_string(),
            qualified_name: format!("{prefix}{module_name}.{}", member.name),
            kind: member.kind,
            visibility: Some(Visibility::Public),
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: Some(format!("{prefix}{module_name}")),
            parent_index: Some(module_parent_idx),
        });
    }

    let content_hash = format!("node-builtins-{prefix}{module_name}-{}", symbols.len());

    ParsedFile {
        path: virtual_path,
        language: "typescript".to_string(),
        content_hash,
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    }
}

/// Synthesize all ParsedFiles — one per module, two per module (bare + `node:` prefix).
fn synthesize_all() -> Vec<ParsedFile> {
    let mut out = Vec::with_capacity(MODULES.len() * 2);
    for spec in MODULES {
        // Bare: `fs`, `path`, etc.
        out.push(synth_module(spec, ""));
        // Prefixed: `node:fs`, `node:path`, etc.
        out.push(synth_module(spec, "node:"));
    }
    out
}

// =============================================================================
// Synthetic dep root (no on-disk path needed)
// =============================================================================

fn synthetic_dep_root() -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "node-builtins".to_string(),
        version: String::new(),
        // Use a sentinel path that will never be walked — parse_metadata_only
        // drives everything; walk_root returns empty.
        root: std::path::PathBuf::from("ext:node-builtins"),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

// =============================================================================
// Ecosystem impl
// =============================================================================

pub struct NodeBuiltinsEcosystem;

impl Ecosystem for NodeBuiltinsEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::LanguagePresent("javascript"),
            EcosystemActivation::LanguagePresent("typescript"),
        ])
    }

    fn locate_roots(&self, _ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        // Single synthetic root — no disk probe needed.
        vec![synthetic_dep_root()]
    }

    /// walk_root returns empty; parse_metadata_only drives all symbol emission.
    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn parse_metadata_only(&self, _dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        Some(synthesize_all())
    }
}

impl ExternalSourceLocator for NodeBuiltinsEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        vec![synthetic_dep_root()]
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        Some(synthesize_all())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn all_symbols() -> Vec<ExtractedSymbol> {
        synthesize_all()
            .into_iter()
            .flat_map(|pf| pf.symbols)
            .collect()
    }

    #[test]
    fn fs_read_file_present() {
        let syms = all_symbols();
        assert!(
            syms.iter().any(|s| s.qualified_name == "fs.readFile"),
            "expected fs.readFile in synthesized symbols"
        );
    }

    #[test]
    fn path_join_present() {
        let syms = all_symbols();
        assert!(
            syms.iter().any(|s| s.qualified_name == "path.join"),
            "expected path.join in synthesized symbols"
        );
    }

    #[test]
    fn node_prefix_alias_works() {
        let syms = all_symbols();
        // `node:fs` prefix emits `node:fs.readFile`
        assert!(
            syms.iter().any(|s| s.qualified_name == "node:fs.readFile"),
            "expected node:fs.readFile alias in synthesized symbols"
        );
        // `node:path` prefix emits `node:path.join`
        assert!(
            syms.iter().any(|s| s.qualified_name == "node:path.join"),
            "expected node:path.join alias in synthesized symbols"
        );
    }

    #[test]
    fn symbol_count_reasonable() {
        // 17 modules × 2 prefixes; each has module-namespace + members.
        // Minimum: 17 * 2 * 2 = 68. Generous floor.
        let syms = all_symbols();
        assert!(
            syms.len() >= 68,
            "expected >= 68 symbols, got {}",
            syms.len()
        );
    }

    #[test]
    fn class_kinds_correct() {
        let syms = all_symbols();
        let event_emitter = syms
            .iter()
            .find(|s| s.qualified_name == "events.EventEmitter")
            .expect("events.EventEmitter must exist");
        assert_eq!(event_emitter.kind, SymbolKind::Class);

        let url_class = syms
            .iter()
            .find(|s| s.qualified_name == "url.URL")
            .expect("url.URL must exist");
        assert_eq!(url_class.kind, SymbolKind::Class);
    }

    #[test]
    fn all_modules_synthesized() {
        let parsed = synthesize_all();
        // 17 modules × 2 (bare + node: prefix) = 34
        assert_eq!(
            parsed.len(),
            34,
            "expected 34 ParsedFiles (17 modules × 2 prefixes), got {}",
            parsed.len()
        );
    }

    #[test]
    fn virtual_paths_follow_convention() {
        let parsed = synthesize_all();
        for pf in &parsed {
            assert!(
                pf.path.starts_with("ext:node-builtins:"),
                "virtual path must start with ext:node-builtins: — got {}",
                pf.path
            );
        }
    }
}
