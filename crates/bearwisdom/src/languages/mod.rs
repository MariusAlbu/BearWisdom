//! Language plugin system for BearWisdom.
//!
//! Each language is a self-contained plugin that provides:
//! - Tree-sitter grammar loading
//! - Scope configuration for qualified name construction
//! - Symbol + reference extraction from source code
//!
//! Resolution (turning reference names into resolved symbol edges) is provided
//! separately via [`crate::indexer::resolve::engine::LanguageResolver`] to avoid
//! circular dependencies — resolvers need the full symbol index, which isn't
//! available during extraction.
//!
//! # Adding a new language
//!
//! 1. Create `languages/<lang>/mod.rs` with a struct implementing [`LanguagePlugin`]
//! 2. Add extraction logic in `extract.rs` (and sub-files as needed)
//! 3. Optionally add a resolver in `resolve.rs` implementing `LanguageResolver`
//! 4. Register the plugin in [`default_registry()`]
//! 5. Register the resolver (if any) in [`default_resolvers()`]

pub mod registry;

use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub use registry::LanguageRegistry;

/// A language plugin provides grammar, scope config, and extraction for one or
/// more language IDs (e.g., TypeScript handles both "typescript" and "tsx").
pub trait LanguagePlugin: Send + Sync + 'static {
    /// Primary identifier (e.g., "typescript").
    fn id(&self) -> &str;

    /// All language IDs this plugin handles (e.g., `&["typescript", "tsx"]`).
    fn language_ids(&self) -> &[&str];

    /// File extensions this plugin claims (e.g., `&[".ts", ".tsx"]`).
    /// Used for documentation and validation; detection is in bearwisdom-profile.
    fn extensions(&self) -> &[&str];

    /// Get the tree-sitter grammar for a specific language ID.
    /// Returns `None` if the ID isn't handled by this plugin.
    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language>;

    /// Scope-creating node kinds for the scope tree builder.
    fn scope_kinds(&self) -> &[ScopeKind];

    /// Extract symbols and references from source code.
    ///
    /// - `source`: the file content
    /// - `file_path`: relative path (used for heuristics like `.tsx` detection)
    /// - `lang_id`: the language ID from detection (e.g., "typescript" or "tsx")
    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult;

    /// Node kinds that SHOULD produce symbols, per the extraction rules.
    /// Used by `bw coverage` to measure extraction completeness.
    fn symbol_node_kinds(&self) -> &[&str] { &[] }

    /// Node kinds that SHOULD produce refs/edges, per the extraction rules.
    /// Used by `bw coverage` to measure extraction completeness.
    fn ref_node_kinds(&self) -> &[&str] { &[] }

    /// Type names that are language builtins and should NOT produce TypeRef.
    /// Used by `bw coverage` to exclude builtin type_identifiers from the
    /// denominator — they're correctly skipped by extractors.
    fn builtin_type_names(&self) -> &[&str] { &[] }
}

// ---------------------------------------------------------------------------
// Built-in plugin registration
// ---------------------------------------------------------------------------

use std::sync::Arc;

pub mod angular;
pub mod astro;
pub mod bash;
pub mod bicep;
pub mod c_lang;
mod generic;
pub mod cmake;
pub mod csharp;
pub mod dart;
pub mod dockerfile;
pub mod elixir;
pub mod go;
pub mod graphql;
pub mod hcl;
pub mod java;
pub mod javascript;
pub mod kotlin;
pub mod make;
pub mod nix;
pub mod php;
pub mod prisma;
pub mod proto;
pub mod puppet;
pub mod python;
pub mod ruby;
pub mod rust_lang;
pub mod scala;
pub mod scss;
pub mod sql;
pub mod svelte;
pub mod swift;
pub mod typescript;
pub mod vue;
pub mod zig;

/// Build the default language registry with all built-in plugins.
///
/// During the migration period, this coexists with the match statement in
/// `indexer/full.rs`. Once all languages are migrated, the match disappears
/// and this becomes the sole dispatch mechanism.
pub fn default_registry() -> LanguageRegistry {
    // The generic plugin handles any language with a tree-sitter grammar
    // but no dedicated extractor.
    let generic = Arc::new(GenericPlugin);
    let mut reg = LanguageRegistry::new(generic);

    reg.register(Arc::new(angular::AngularPlugin));
    reg.register(Arc::new(astro::AstroPlugin));
    reg.register(Arc::new(bash::BashPlugin));
    reg.register(Arc::new(bicep::BicepPlugin));
    reg.register(Arc::new(c_lang::CLangPlugin));
    reg.register(Arc::new(cmake::CMakePlugin));
    reg.register(Arc::new(csharp::CSharpPlugin));
    reg.register(Arc::new(dart::DartPlugin));
    reg.register(Arc::new(dockerfile::DockerfilePlugin));
    reg.register(Arc::new(elixir::ElixirPlugin));
    reg.register(Arc::new(go::GoPlugin));
    reg.register(Arc::new(graphql::GraphQlPlugin));
    reg.register(Arc::new(hcl::HclPlugin));
    reg.register(Arc::new(java::JavaPlugin));
    reg.register(Arc::new(javascript::JavascriptPlugin));
    reg.register(Arc::new(kotlin::KotlinPlugin));
    reg.register(Arc::new(make::MakePlugin));
    reg.register(Arc::new(nix::NixPlugin));
    reg.register(Arc::new(php::PhpPlugin));
    reg.register(Arc::new(prisma::PrismaPlugin));
    reg.register(Arc::new(proto::ProtoPlugin));
    reg.register(Arc::new(puppet::PuppetPlugin));
    reg.register(Arc::new(python::PythonPlugin));
    reg.register(Arc::new(ruby::RubyPlugin));
    reg.register(Arc::new(rust_lang::RustLangPlugin));
    reg.register(Arc::new(scala::ScalaPlugin));
    reg.register(Arc::new(scss::ScssPlugin));
    reg.register(Arc::new(sql::SqlPlugin));
    reg.register(Arc::new(svelte::SveltePlugin));
    reg.register(Arc::new(swift::SwiftPlugin));
    reg.register(Arc::new(typescript::TypeScriptPlugin));
    reg.register(Arc::new(vue::VuePlugin));
    reg.register(Arc::new(zig::ZigPlugin));

    reg
}

/// Collect language-specific resolvers from all language modules.
///
/// During migration, this coexists with `indexer::resolve::rules::default_resolvers()`.
/// Once all resolvers are migrated, this replaces it.
pub fn default_resolvers() -> Vec<Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
    vec![
        Arc::new(c_lang::CLangResolver),
        Arc::new(csharp::CSharpResolver),
        Arc::new(dart::DartResolver),
        Arc::new(elixir::ElixirResolver),
        Arc::new(go::GoResolver),
        Arc::new(java::JavaResolver),
        Arc::new(kotlin::KotlinResolver),
        Arc::new(php::PhpResolver),
        Arc::new(python::PythonResolver),
        Arc::new(ruby::RubyResolver),
        Arc::new(rust_lang::RustResolver),
        Arc::new(scala::ScalaResolver),
        Arc::new(swift::SwiftResolver),
        Arc::new(typescript::TypeScriptResolver),
    ]
}

// ---------------------------------------------------------------------------
// Shared extraction utilities
// ---------------------------------------------------------------------------

/// When a call has a chain (e.g. `Foo::bar()`, `Foo.bar()`), emit a `TypeRef`
/// for the type prefix — the segment before the final method name — if it
/// looks like a type (starts with uppercase).
pub fn emit_chain_type_ref(
    chain: &Option<crate::types::MemberChain>,
    source_symbol_index: usize,
    func_node: &tree_sitter::Node,
    refs: &mut Vec<crate::types::ExtractedRef>,
) {
    let c = match chain.as_ref() {
        Some(c) if c.segments.len() >= 2 => c,
        _ => return,
    };
    let type_seg = &c.segments[c.segments.len() - 2];
    if type_seg.name.chars().next().map_or(false, |ch| ch.is_uppercase()) {
        refs.push(crate::types::ExtractedRef {
            source_symbol_index,
            target_name: type_seg.name.clone(),
            kind: crate::types::EdgeKind::TypeRef,
            line: func_node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }
}

// ---------------------------------------------------------------------------
// Generic fallback plugin
// ---------------------------------------------------------------------------

/// Fallback plugin that handles any language with a tree-sitter grammar
/// but no dedicated extractor. Uses heuristic extraction based on common
/// node kinds across languages.
struct GenericPlugin;

impl LanguagePlugin for GenericPlugin {
    fn id(&self) -> &str {
        "generic"
    }

    fn language_ids(&self) -> &[&str] {
        // The generic plugin doesn't claim any specific IDs — it's the fallback.
        &[]
    }

    fn extensions(&self) -> &[&str] {
        &[]
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        crate::parser::languages::get_language(lang_id)
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        // The generic extractor has its own per-language scope configs.
        // Those are consulted inside `generic::extract()`.
        &[]
    }

    fn extract(&self, source: &str, _file_path: &str, lang_id: &str) -> ExtractionResult {
        match generic::extract::extract(source, lang_id) {
            Some(r) => ExtractionResult::new(r.symbols, r.refs, r.has_errors),
            None => ExtractionResult::empty(),
        }
    }
}
