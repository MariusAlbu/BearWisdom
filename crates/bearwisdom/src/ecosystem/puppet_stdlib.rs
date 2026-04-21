// =============================================================================
// ecosystem/puppet_stdlib.rs — Puppet core types & built-in functions
//
// Puppet's built-in resource types and functions are injected at runtime by
// the Puppet agent — they do not live on disk as parseable .pp source. This
// ecosystem synthesises ParsedFile entries for each built-in so the resolver
// can satisfy `include`, `class { 'file': ... }`, `service { ...: }`, etc.
// without raising unresolved-ref noise.
//
// Built-in resource types → SymbolKind::Class (Puppet resource types are
//   declared with `type` or used as class-like constructs; Class is the
//   closest structural analogue).
// Built-in functions → SymbolKind::Function
//
// Virtual file paths: `ext:puppet-stdlib:types/<name>.pp`
//                     `ext:puppet-stdlib:functions/<name>.pp`
//
// Activation: any .pp file in the project (LanguagePresent("puppet")).
// No on-disk walk — everything is synthesised in build_symbol_index.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("puppet-stdlib");

const LEGACY_ECOSYSTEM_TAG: &str = "puppet-stdlib";
const LANGUAGES: &[&str] = &["puppet"];

// =============================================================================
// Built-in catalogs
// =============================================================================

/// Core resource types that ship with the Puppet agent.
const BUILTIN_TYPES: &[&str] = &[
    "file",
    "service",
    "package",
    "exec",
    "user",
    "group",
    "cron",
    "notify",
    "host",
    "mount",
    "ssh_authorized_key",
    "tidy",
    "yumrepo",
    "zone",
];

/// Core functions available in all Puppet manifests without `require`.
const BUILTIN_FUNCTIONS: &[&str] = &[
    "include",
    "require",
    "contain",
    "notice",
    "warning",
    "err",
    "fail",
    "template",
    "epp",
    "inline_template",
    "hiera",
    "lookup",
    "alert",
    "regsubst",
    "sprintf",
];

// =============================================================================
// Ecosystem impl
// =============================================================================

pub struct PuppetStdlibEcosystem;

impl Ecosystem for PuppetStdlibEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("puppet")
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        // Synthetic — no on-disk root needed. Provide a sentinel so the indexer
        // has a dep root to pass to parse_metadata_only / build_symbol_index.
        vec![ExternalDepRoot {
            module_path: "puppet-stdlib".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            root: PathBuf::from("ext:puppet-stdlib"),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        }]
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        // No source walk; synthesis happens in parse_metadata_only.
        Vec::new()
    }

    fn parse_metadata_only(&self, _dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        Some(synthesise_stdlib())
    }

    /// The symbol index is populated from the synthesised ParsedFiles above,
    /// but we also expose it directly so demand-driven callers can look up
    /// symbols without a file round-trip.
    fn build_symbol_index(
        &self,
        _dep_roots: &[ExternalDepRoot],
    ) -> crate::ecosystem::symbol_index::SymbolLocationIndex {
        let mut index = crate::ecosystem::symbol_index::SymbolLocationIndex::new();
        let module = "puppet-stdlib".to_string();
        for name in BUILTIN_TYPES {
            let path = synthetic_path("types", name);
            index.insert(module.clone(), name.to_string(), path);
        }
        for name in BUILTIN_FUNCTIONS {
            let path = synthetic_path("functions", name);
            index.insert(module.clone(), name.to_string(), path);
        }
        index
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
}

impl ExternalSourceLocator for PuppetStdlibEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        Ecosystem::locate_roots(
            self,
            &crate::ecosystem::LocateContext {
                project_root: Path::new("."),
                manifests: &Default::default(),
                active_ecosystems: &[],
            },
        )
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        Some(synthesise_stdlib())
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<PuppetStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(PuppetStdlibEcosystem)).clone()
}

// =============================================================================
// Synthesis helpers
// =============================================================================

fn synthetic_path(subdir: &str, name: &str) -> PathBuf {
    PathBuf::from(format!("ext:puppet-stdlib:{subdir}/{name}.pp"))
}

fn synth_symbol(
    name: &str,
    kind: SymbolKind,
    signature: String,
    subdir: &str,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(signature),
        doc_comment: None,
        scope_path: Some(format!("puppet-stdlib::{subdir}")),
        parent_index: None,
    }
}

fn build_parsed_file(
    virtual_path: String,
    symbols: Vec<ExtractedSymbol>,
) -> ParsedFile {
    let content_hash = format!("puppet-stdlib-{}", symbols.len());
    ParsedFile {
        path: virtual_path,
        language: "puppet".to_string(),
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

/// Synthesise one ParsedFile per built-in type and one per built-in function.
fn synthesise_stdlib() -> Vec<ParsedFile> {
    let mut out: Vec<ParsedFile> = Vec::new();

    for name in BUILTIN_TYPES {
        let sig = format!("type {name}");
        let sym = synth_symbol(name, SymbolKind::Class, sig, "types");
        out.push(build_parsed_file(
            format!("ext:puppet-stdlib:types/{name}.pp"),
            vec![sym],
        ));
    }

    for name in BUILTIN_FUNCTIONS {
        let sig = format!("function {name}(...)");
        let sym = synth_symbol(name, SymbolKind::Function, sig, "functions");
        out.push(build_parsed_file(
            format!("ext:puppet-stdlib:functions/{name}.pp"),
            vec![sym],
        ));
    }

    out
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        let e = PuppetStdlibEcosystem;
        assert_eq!(e.id(), ID);
        assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
        assert_eq!(Ecosystem::languages(&e), &["puppet"]);
    }

    #[test]
    fn activation_is_language_present() {
        let e = PuppetStdlibEcosystem;
        assert!(matches!(
            Ecosystem::activation(&e),
            EcosystemActivation::LanguagePresent("puppet")
        ));
    }

    #[test]
    fn uses_demand_driven() {
        assert!(PuppetStdlibEcosystem.uses_demand_driven_parse());
    }

    #[test]
    fn synthesise_covers_all_builtins() {
        let files = synthesise_stdlib();
        // One file per built-in type + one per built-in function.
        assert_eq!(files.len(), BUILTIN_TYPES.len() + BUILTIN_FUNCTIONS.len());
    }

    #[test]
    fn types_are_class_kind() {
        let files = synthesise_stdlib();
        for f in &files {
            if f.path.contains("/types/") {
                assert_eq!(f.symbols.len(), 1);
                assert_eq!(f.symbols[0].kind, SymbolKind::Class, "type {} should be Class", f.symbols[0].name);
            }
        }
    }

    #[test]
    fn functions_are_function_kind() {
        let files = synthesise_stdlib();
        for f in &files {
            if f.path.contains("/functions/") {
                assert_eq!(f.symbols.len(), 1);
                assert_eq!(f.symbols[0].kind, SymbolKind::Function, "function {} should be Function", f.symbols[0].name);
            }
        }
    }

    #[test]
    fn symbol_index_covers_all_builtins() {
        let e = PuppetStdlibEcosystem;
        let index = e.build_symbol_index(&[]);
        // Check a sample from each category.
        assert!(index.locate("puppet-stdlib", "file").is_some());
        assert!(index.locate("puppet-stdlib", "service").is_some());
        assert!(index.locate("puppet-stdlib", "include").is_some());
        assert!(index.locate("puppet-stdlib", "lookup").is_some());
    }

    #[test]
    fn parse_metadata_only_returns_stdlib() {
        let e = PuppetStdlibEcosystem;
        let sentinel = ExternalDepRoot {
            module_path: "puppet-stdlib".into(),
            version: String::new(),
            root: PathBuf::from("ext:puppet-stdlib"),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        };
        let files = Ecosystem::parse_metadata_only(&e, &sentinel).unwrap();
        assert!(!files.is_empty());
        let names: Vec<&str> = files
            .iter()
            .flat_map(|f| f.symbols.iter().map(|s| s.name.as_str()))
            .collect();
        assert!(names.contains(&"file"));
        assert!(names.contains(&"include"));
        assert!(names.contains(&"lookup"));
    }
}
