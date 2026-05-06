// =============================================================================
// ecosystem/puppet_stdlib.rs — Puppet runtime resource types + built-in functions
//
// Puppet's resource types (`file`, `service`, `package`, `exec`, ~50 of them)
// and built-in functions (`include`, `require`, `notify`, `lookup`, ~100 of
// them) are runtime-injected by the puppet agent. They're not declared in any
// `.pp` source — they live in the puppet Ruby gem under
// `lib/puppet/type/<name>.rb` and `lib/puppet/functions/**/*.rb`.
//
// This walker probes for an installed puppet gem on disk, scans those
// directories, extracts the names defined there via targeted regex on each
// file's `Puppet::Type.newtype(:<name>)` /
// `Puppet::Functions.create_function(:<name>)` /
// `Puppet::Parser::Functions.newfunction(:<name>)` patterns, and emits one
// `ParsedFile` per Ruby file with the extracted name as a Puppet-language
// symbol. Resolves bare `file { ... }`, `notify(...)`, `each(...)` etc. in
// `.pp` source against the user's actual installed puppet version.
//
// Discovery order:
//   1. $BEARWISDOM_PUPPET_GEM — explicit gem root override.
//   2. `gem env gemdir` → `<gemdir>/gems/puppet-<version>` (newest version).
//   3. Common system gem locations on each OS.
//
// When no puppet gem is found the walker emits nothing and resource-type /
// built-in-function references in `.pp` source go unresolved. Substrate
// activation (`LanguagePresent("puppet")`) is correct: every Puppet project
// uses these names.
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, FlowMeta, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("puppet-stdlib");
const TAG: &str = "puppet-stdlib";
const LANGUAGES: &[&str] = &["puppet"];

// ---------------------------------------------------------------------------
// Ecosystem impl
// ---------------------------------------------------------------------------

pub struct PuppetStdlibEcosystem;

impl Ecosystem for PuppetStdlibEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("puppet")
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_puppet_gem()
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        // Symbols are emitted via parse_metadata_only — Ruby gem source isn't
        // fed through the puppet extractor (different language) so the normal
        // walk path is empty.
        Vec::new()
    }

    fn parse_metadata_only(&self, dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        let parsed = parse_puppet_gem(&dep.root);
        if parsed.is_empty() { None } else { Some(parsed) }
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
}

impl ExternalSourceLocator for PuppetStdlibEcosystem {
    fn ecosystem(&self) -> &'static str { TAG }

    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_puppet_gem()
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        let roots = discover_puppet_gem();
        let mut out = Vec::new();
        for r in roots {
            out.extend(parse_puppet_gem(&r.root));
        }
        if out.is_empty() { None } else { Some(out) }
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<PuppetStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(PuppetStdlibEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Gem discovery
// ---------------------------------------------------------------------------

fn discover_puppet_gem() -> Vec<ExternalDepRoot> {
    let Some(gem_root) = probe_puppet_gem_root() else {
        debug!("puppet-stdlib: no puppet gem probed");
        return Vec::new();
    };
    let version = gem_root
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("puppet-"))
        .unwrap_or("")
        .to_string();
    debug!("puppet-stdlib: using {}", gem_root.display());
    vec![ExternalDepRoot {
        module_path: "puppet".to_string(),
        version,
        root: gem_root,
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }]
}

fn probe_puppet_gem_root() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_PUPPET_GEM") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { return Some(p); }
    }
    if let Some(p) = probe_via_gem_env() {
        return Some(p);
    }
    probe_standard_gem_paths()
}

fn probe_via_gem_env() -> Option<PathBuf> {
    // `gem env gemdir` prints something like:
    //   /usr/lib/ruby/gems/3.0.0
    //   C:/Ruby32-x64/lib/ruby/gems/3.2.0
    // We then look for `gems/puppet-<version>` inside it.
    let output = Command::new("gem").args(["env", "gemdir"]).output().ok()?;
    if !output.status.success() { return None; }
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let gemdir = PathBuf::from(stdout.trim());
    find_latest_puppet_gem(&gemdir)
}

fn probe_standard_gem_paths() -> Option<PathBuf> {
    // Common parent dirs that hold a versioned ruby subdir (e.g. `3.0.0/`)
    // which in turn holds `gems/puppet-X.Y.Z`. Probe shallowly and pick the
    // newest puppet across all matches.
    let parents: &[&str] = &[
        // Linux distros
        "/usr/lib/ruby/gems",
        "/var/lib/gems",
        "/usr/share/gems",
        "/usr/local/lib/ruby/gems",
        // macOS
        "/Library/Ruby/Gems",
        "/usr/local/lib/ruby/gems",
        // Windows RubyInstaller
        "C:/Ruby34-x64/lib/ruby/gems",
        "C:/Ruby33-x64/lib/ruby/gems",
        "C:/Ruby32-x64/lib/ruby/gems",
        "C:/Ruby31-x64/lib/ruby/gems",
        "C:/tools/ruby34/lib/ruby/gems",
        "C:/tools/ruby33/lib/ruby/gems",
        "C:/tools/ruby32/lib/ruby/gems",
    ];
    let mut candidates: Vec<PathBuf> = Vec::new();
    for parent_str in parents {
        let parent = Path::new(parent_str);
        let Ok(entries) = std::fs::read_dir(parent) else { continue };
        for entry in entries.flatten() {
            let versioned = entry.path();
            if !versioned.is_dir() { continue }
            if let Some(p) = find_latest_puppet_gem(&versioned) {
                candidates.push(p);
            }
        }
    }
    // Also try ~/.gem/ruby/<ver>/gems/
    if let Some(home) = dirs::home_dir() {
        let user_gems = home.join(".gem").join("ruby");
        if let Ok(entries) = std::fs::read_dir(&user_gems) {
            for entry in entries.flatten() {
                let versioned = entry.path();
                if !versioned.is_dir() { continue }
                if let Some(p) = find_latest_puppet_gem(&versioned) {
                    candidates.push(p);
                }
            }
        }
    }
    candidates.sort();
    candidates.into_iter().next_back()
}

/// Look for `<gemdir>/gems/puppet-<version>` and return the lex-newest match.
fn find_latest_puppet_gem(gemdir: &Path) -> Option<PathBuf> {
    let gems_dir = gemdir.join("gems");
    let entries = std::fs::read_dir(&gems_dir).ok()?;
    let mut puppets: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("puppet-") && !n.starts_with("puppet-agent"))
                    .unwrap_or(false)
        })
        .collect();
    puppets.sort();
    puppets.into_iter().next_back()
}

// ---------------------------------------------------------------------------
// Gem walk + name extraction
// ---------------------------------------------------------------------------

/// Walk the puppet gem for resource type and built-in function definitions,
/// emitting one `ParsedFile` per source file with the discovered name.
pub(crate) fn parse_puppet_gem(gem_root: &Path) -> Vec<ParsedFile> {
    let mut out = Vec::new();

    let type_dir = gem_root.join("lib").join("puppet").join("type");
    walk_resource_types(&type_dir, &mut out);

    let modern_fn_dir = gem_root.join("lib").join("puppet").join("functions");
    walk_modern_functions(&modern_fn_dir, &modern_fn_dir, &mut out);

    let legacy_fn_dir = gem_root
        .join("lib")
        .join("puppet")
        .join("parser")
        .join("functions");
    walk_legacy_functions(&legacy_fn_dir, &mut out);

    debug!("puppet-stdlib: emitted {} symbols from {}", out.len(), gem_root.display());
    out
}

fn walk_resource_types(dir: &Path, out: &mut Vec<ParsedFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() { continue }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if !name.ends_with(".rb") { continue }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        // Resource type files normally contain exactly one
        // `Puppet::Type.newtype(:<name>)` call near the top. Some files
        // declare multiple (e.g. nagios) — emit a symbol for each.
        for (line_idx, type_name) in find_newtype_calls(&content) {
            out.push(make_parsed_file(
                &path,
                gem_relative(&path),
                type_name.clone(),
                SymbolKind::Class,
                line_idx,
                Some(format!("/* puppet resource type */ {} {{ ... }}", type_name)),
            ));
        }
    }
}

fn walk_modern_functions(root: &Path, dir: &Path, out: &mut Vec<ParsedFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            walk_modern_functions(root, &path, out);
            continue;
        }
        if !ft.is_file() { continue }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if !name.ends_with(".rb") { continue }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        // Modern API: `Puppet::Functions.create_function(:foo)` — one per file
        // by convention. Use the regex match for the authoritative name; fall
        // back to the filename stem for cases the regex misses.
        let mut emitted = false;
        for (line_idx, fn_name) in find_create_function_calls(&content) {
            // Strip `'` quotes from scoped names like `:'mymod::myfn'`.
            let bare = fn_name.trim_matches('\'').to_string();
            // Emit both the bare-leaf form (for unqualified `mymod::myfn(...)`
            // calls in .pp source — Puppet resolves leaf names too) and the
            // full scoped form. Skip the leaf when it's identical to the
            // qualified.
            let leaf = bare.split("::").last().unwrap_or(&bare).to_string();
            out.push(make_parsed_file(
                &path,
                gem_relative(&path),
                bare.clone(),
                SymbolKind::Function,
                line_idx,
                Some(format!("/* puppet function */ {}(...)", bare)),
            ));
            if leaf != bare {
                out.push(make_parsed_file(
                    &path,
                    gem_relative(&path),
                    leaf,
                    SymbolKind::Function,
                    line_idx,
                    Some(format!("/* puppet function */ {}(...)", bare)),
                ));
            }
            emitted = true;
        }
        if !emitted {
            // Fallback: filename without `.rb` is the function name.
            let stem = name.trim_end_matches(".rb").to_string();
            if !stem.is_empty() {
                out.push(make_parsed_file(
                    &path,
                    gem_relative(&path),
                    stem.clone(),
                    SymbolKind::Function,
                    0,
                    Some(format!("/* puppet function */ {}(...)", stem)),
                ));
            }
        }
    }
}

fn walk_legacy_functions(dir: &Path, out: &mut Vec<ParsedFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() { continue }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if !name.ends_with(".rb") { continue }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        // Legacy 3.x DSL: `newfunction(:foo, :type => :rvalue) do ... end`
        for (line_idx, fn_name) in find_legacy_newfunction_calls(&content) {
            out.push(make_parsed_file(
                &path,
                gem_relative(&path),
                fn_name.clone(),
                SymbolKind::Function,
                line_idx,
                Some(format!("/* puppet function (legacy) */ {}(...)", fn_name)),
            ));
        }
    }
}

/// Strip everything before `lib/puppet/` so the synthesized path is stable
/// across machines (the gem's parent dir varies by ruby version + install).
fn gem_relative(path: &Path) -> String {
    let display = path.to_string_lossy().replace('\\', "/");
    if let Some(idx) = display.find("/lib/puppet/") {
        display[idx + 1..].to_string()
    } else {
        display
    }
}

fn make_parsed_file(
    abs_path: &Path,
    rel_path: String,
    sym_name: String,
    kind: SymbolKind,
    line: u32,
    signature: Option<String>,
) -> ParsedFile {
    let symbol = ExtractedSymbol {
        name: sym_name.clone(),
        qualified_name: sym_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: 0,
        end_col: 0,
        signature,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    };
    let mtime = std::fs::metadata(abs_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    ParsedFile {
        path: format!("ext:puppet-stdlib:{rel_path}"),
        language: "puppet".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime,
        package_id: None,
        symbols: vec![symbol],
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: vec![None],
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: vec![false],
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Targeted name extraction (no full Ruby parse)
// ---------------------------------------------------------------------------

/// Match `Puppet::Type.newtype(:<name>)` — the canonical resource-type
/// declaration. The `:<name>` is a Ruby symbol; names are
/// `[A-Za-z_][A-Za-z0-9_]*`.
pub(crate) fn find_newtype_calls(content: &str) -> Vec<(u32, String)> {
    find_symbol_arg(content, &["Puppet::Type.newtype", "Type.newtype"])
}

/// Match `Puppet::Functions.create_function(:<name>)` — modern function API.
/// The name may be quoted (`:'mymod::myfn'`).
pub(crate) fn find_create_function_calls(content: &str) -> Vec<(u32, String)> {
    find_symbol_arg(content, &["Puppet::Functions.create_function", "Functions.create_function"])
}

/// Match legacy `newfunction(:<name>, ...)` inside the
/// `module Puppet::Parser::Functions` namespace.
pub(crate) fn find_legacy_newfunction_calls(content: &str) -> Vec<(u32, String)> {
    find_symbol_arg(content, &["newfunction"])
}

/// Generic helper: for each line containing one of the call patterns, pull
/// the first symbol-literal argument (`:<name>` or `:'<name>'`).
fn find_symbol_arg(content: &str, patterns: &[&str]) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        // Cheap pre-filter: must contain at least one of the patterns and a
        // Ruby symbol argument indicator `(:`. Rules out comments/docs.
        if !patterns.iter().any(|p| line.contains(p)) { continue }
        let Some(pos) = line.find("(:") else { continue };
        let after = &line[pos + 2..];
        // Quoted variant: `:'name'` — strip leading apostrophe, read until
        // closing apostrophe.
        let name = if let Some(rest) = after.strip_prefix('\'') {
            rest.split('\'').next().unwrap_or("").to_string()
        } else {
            // Bare symbol — read while the char is a valid ruby symbol char.
            after
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect()
        };
        if name.is_empty() { continue }
        out.push((idx as u32, name));
    }
    out
}

#[cfg(test)]
#[path = "puppet_stdlib_tests.rs"]
mod tests;
