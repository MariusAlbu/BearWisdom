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

/// Core resource types that ship with the Puppet agent + commonly-bundled
/// types from the puppetlabs-supported modules (concat, stdlib, etc.) that
/// virtually every manifest references regardless of whether the modules
/// are physically installed in the project tree.
const BUILTIN_TYPES: &[&str] = &[
    // Core Puppet agent types
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
    "sshkey",
    "tidy",
    "yumrepo",
    "zone",
    "augeas",
    "selboolean",
    "selmodule",
    "scheduled_task",
    "schedule",
    "stage",
    "filebucket",
    "interface",
    // puppetlabs-concat (used by virtually every Puppet codebase)
    "Concat",
    "Concat_file",
    "Concat_fragment",
    // puppetlabs-stdlib defined types / data types
    "Anchor",
    "File_line",
    // puppetlabs-mysql (so mysql-using projects get internal type refs even
    // when the .rb defined-type sources aren't indexed)
    "Mysql_user",
    "Mysql_database",
    "Mysql_grant",
    "Mysql_datadir",
    "Mysql_login_path",
    "Mysql_plugin",
    // Apache HTTPD module common types
    "A2mod",
    // Puppet built-in deferred function value
    "Deferred",
];

/// Functions always available in Puppet manifests:
///   - Built-ins shipped with the Puppet language itself
///   - puppetlabs-stdlib functions (de facto required dependency on every
///     real-world Puppet codebase — over 100 functions)
///
/// Names match the function name as called from Puppet DSL. Sources:
///   - https://puppet.com/docs/puppet/latest/function.html
///   - https://github.com/puppetlabs/puppetlabs-stdlib/tree/main/lib/puppet/functions
const BUILTIN_FUNCTIONS: &[&str] = &[
    // -----------------------------------------------------------------------
    // Puppet language built-ins
    // -----------------------------------------------------------------------
    "include",
    "require",
    "contain",
    "realize",
    "tag",
    "tagged",
    "defined",
    "create_resources",
    "ensure_resource",
    "ensure_packages",
    "notice",
    "warning",
    "err",
    "fail",
    "alert",
    "info",
    "debug",
    "emerg",
    "crit",
    "template",
    "epp",
    "inline_template",
    "inline_epp",
    "hiera",
    "hiera_array",
    "hiera_hash",
    "hiera_include",
    "lookup",
    "regsubst",
    "sprintf",
    "split",
    "join",
    "strftime",
    "fqdn_rand",
    "generate",
    "binary_file",
    "file",
    "find_file",
    "find_template",
    "type",
    "assert_type",
    "new",
    "next",
    "return",
    "break",
    "step",
    "with",
    "each",
    "each_pair",
    "map",
    "reduce",
    "filter",
    "slice",
    "reverse_each",
    "any",
    "all",
    "match",
    "scanf",
    "size",
    "length",
    "empty",
    "keys",
    "values",
    "downcase",
    "upcase",
    "capitalize",
    "chomp",
    "chop",
    "lstrip",
    "rstrip",
    "strip",
    "tr",
    "abs",
    "ceiling",
    "floor",
    "max",
    "min",
    "round",
    "convert_to",
    "merge",
    "delete",
    "dig",
    "get",
    "then",
    "lest",
    "case",
    "select",
    // -----------------------------------------------------------------------
    // puppetlabs-stdlib core
    // -----------------------------------------------------------------------
    "abs",
    "any2array",
    "any2bool",
    "assert_private",
    "base64",
    "basename",
    "bool2num",
    "bool2str",
    "camelcase",
    "capitalize",
    "ceiling",
    "chomp",
    "chop",
    "clamp",
    "concat",
    "convert_base",
    "count",
    "deep_merge",
    "delete",
    "delete_at",
    "delete_regex",
    "delete_undef_values",
    "delete_values",
    "deprecation",
    "difference",
    "dig",
    "dirname",
    "dos2unix",
    "downcase",
    "empty",
    "enclose_ipv6",
    "ensure_packages",
    "ensure_resource",
    "ensure_resources",
    "extlib",
    "fact",
    "flatten",
    "floor",
    "fqdn_rand_string",
    "fqdn_rotate",
    "fqdn_uuid",
    "get_module_path",
    "getparam",
    "getvar",
    "glob",
    "grep",
    "has_interface_with",
    "has_ip_address",
    "has_ip_network",
    "has_key",
    "hash",
    "intersection",
    "ip_in_range",
    "is_a",
    "is_array",
    "is_bool",
    "is_email_address",
    "is_float",
    "is_function_available",
    "is_hash",
    "is_integer",
    "is_ip_address",
    "is_ipv4_address",
    "is_ipv6_address",
    "is_mac_address",
    "is_numeric",
    "is_string",
    "join",
    "join_keys_to_values",
    "keys",
    "length",
    "load_module_metadata",
    "loadjson",
    "loadyaml",
    "lstrip",
    "max",
    "member",
    "merge",
    "min",
    "num2bool",
    "os_version_gte",
    "parsehocon",
    "parsejson",
    "parsepson",
    "parseyaml",
    "pick",
    "pick_default",
    "powershell_escape",
    "prefix",
    "private",
    "pry",
    "pw_hash",
    "range",
    "regexpescape",
    "reject",
    "reverse",
    "round",
    "rstrip",
    "seeded_rand",
    "seeded_rand_string",
    "shell_escape",
    "shell_join",
    "shell_split",
    "shuffle",
    "size",
    "sort",
    "sprintf_hash",
    "squeeze",
    "stdlib",
    "str2bool",
    "str2saltedpbkdf2",
    "str2saltedsha512",
    "strftime",
    "strip",
    "suffix",
    "swapcase",
    "time",
    "to_bytes",
    "to_json",
    "to_json_pretty",
    "to_python",
    "to_ruby",
    "to_yaml",
    "type_of",
    "union",
    "unique",
    "unix2dos",
    "upcase",
    "uriescape",
    "validate_absolute_path",
    "validate_array",
    "validate_augeas",
    "validate_bool",
    "validate_cmd",
    "validate_domain_name",
    "validate_email_address",
    "validate_hash",
    "validate_integer",
    "validate_ip_address",
    "validate_ipv4_address",
    "validate_ipv6_address",
    "validate_legacy",
    "validate_numeric",
    "validate_re",
    "validate_slength",
    "validate_string",
    "validate_x509_rsa_key_pair",
    "values",
    "values_at",
    "zip",
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
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
    }
}

/// Synthesise one ParsedFile per built-in type and one per built-in function.
/// Deduplicates names because some functions appear in both the Puppet
/// language built-ins and the puppetlabs-stdlib lists (e.g. `dig`, `merge`).
fn synthesise_stdlib() -> Vec<ParsedFile> {
    let mut out: Vec<ParsedFile> = Vec::new();
    let mut seen: std::collections::HashSet<&'static str> = std::collections::HashSet::new();

    for name in BUILTIN_TYPES {
        if !seen.insert(name) {
            continue;
        }
        let sig = format!("type {name}");
        let sym = synth_symbol(name, SymbolKind::Class, sig, "types");
        out.push(build_parsed_file(
            format!("ext:puppet-stdlib:types/{name}.pp"),
            vec![sym],
        ));
    }

    for name in BUILTIN_FUNCTIONS {
        if !seen.insert(name) {
            continue;
        }
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
