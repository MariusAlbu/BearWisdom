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
//!    and return it from [`LanguagePlugin::resolver()`]
//! 4. Register the plugin in [`default_registry()`]

pub mod common;
pub mod registry;

use crate::indexer::resolve::engine::LanguageResolver;
use crate::types::{EmbeddedRegion, ExtractionResult};
use crate::parser::scope_tree::ScopeKind;

// Re-export the shared utility from common so existing callers using
// `crate::languages::emit_chain_type_ref` continue to work without changes.
pub use common::emit_chain_type_ref;
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

    /// Return sub-language text regions contained in this file (e.g. the
    /// `<script lang="ts">` block inside a Vue SFC, the frontmatter inside an
    /// Astro file, the `@code { }` block inside a Razor view). The indexer
    /// dispatches each region to the plugin for its declared language,
    /// re-runs locals filtering against the sub-grammar, and splices the
    /// resulting symbols/refs back into the host file with line/column
    /// offsets applied.
    ///
    /// Host extractors that carry embedded sub-languages (Svelte/Vue/Astro/
    /// Razor/HTML/PHP/MDX) override this. Leaf languages leave the default.
    ///
    /// Called by `indexer/full::parse_file` AFTER `extract()` returns — so
    /// host extractors that parse their source once can cache the parse and
    /// serve both calls, or re-parse cheaply here.
    fn embedded_regions(
        &self,
        _source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<EmbeddedRegion> {
        Vec::new()
    }

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

    /// Primitive and built-in type names that can never appear in a project's
    /// symbol index. Used by the resolution engine to classify unresolvable
    /// references as "external" rather than "unresolved". Includes primitive
    /// keywords, wrapper types, and generic type parameter names.
    fn primitives(&self) -> &'static [&'static str] { &[] }

    /// Runtime/library globals that are always external for this language,
    /// regardless of project dependencies. Examples: `console`, `window` for
    /// JS/TS; `Logger` for JVM languages. NOT primitives (type names) — these
    /// are identifiers that appear in code but are never project-defined.
    fn externals(&self) -> &'static [&'static str] { &[] }

    /// Dependency-gated framework globals. Given the project's declared
    /// dependencies, returns names injected by frameworks/libraries that should
    /// be classified as external. Examples: Spring annotations when
    /// `org.springframework` is a dep, Jest globals when `jest` is a dep.
    ///
    /// **DEPRECATED — scheduled for removal in Phase 4 of the manifest-first
    /// externals plan.** The permanent replacement is `externals_locator`,
    /// External source locator for this language's package ecosystem.
    /// When present, the indexer uses it to find on-disk source for every
    /// dependency declared in the project's manifest and indexes that
    /// source with `origin='external'`. Languages without a package
    /// ecosystem (Bash, VBA, Cobol, SQL, Make, Dockerfile) return `None`.
    fn externals_locator(
        &self,
    ) -> Option<Arc<dyn crate::indexer::externals::ExternalSourceLocator>> {
        None
    }

    /// (child_kind, parent_kind) pairs where a ref-producing CST node should NOT
    /// be counted in the coverage denominator when it appears as a direct child of
    /// the given parent kind.
    ///
    /// Used for languages with structural nesting where a single semantic ref site
    /// produces multiple CST nodes of the same kind. For example, Nix curried
    /// application (`f a b` → two nested `apply_expression` nodes) should only
    /// count the outermost call. Declaring `("apply_expression", "apply_expression")`
    /// here tells the coverage walker to skip inner apply nodes whose parent is also
    /// an apply.
    fn nested_ref_skip_pairs(&self) -> &[(&'static str, &'static str)] { &[] }

    /// Return the language resolver for this plugin, if one exists.
    ///
    /// This ties plugin and resolver together — no separate registration list.
    /// The engine collects resolvers by calling this on every registered plugin.
    fn resolver(&self) -> Option<Arc<dyn LanguageResolver>> { None }

    /// Return language-specific connectors provided by this plugin.
    ///
    /// Each connector implements the full `Connector` trait (detect/extract/match).
    /// The registry collects these from all plugins alongside any remaining
    /// cross-cutting connectors.
    ///
    /// This is the primary mechanism for adding connector support to a language —
    /// all detection, extraction, and matching logic lives in the plugin directory.
    fn connectors(&self) -> Vec<Box<dyn crate::connectors::traits::Connector>> { vec![] }

    /// Post-index hook for language-specific enrichment that writes to tables
    /// other than `flow_edges` (e.g. `db_mappings`, `concepts`).
    ///
    /// Called by `full_index` after all symbols, edges, and flow connectors have
    /// been written.  The default implementation is a no-op.
    fn post_index(
        &self,
        _db: &crate::db::Database,
        _project_root: &std::path::Path,
        _ctx: &crate::indexer::project_context::ProjectContext,
    ) {}
}

// ---------------------------------------------------------------------------
// Built-in plugin registration
// ---------------------------------------------------------------------------

use std::sync::{Arc, LazyLock};

pub mod angular;
pub mod astro;
pub mod bash;
pub mod bicep;
pub mod blade;
pub mod c_lang;
mod generic;
pub mod cmake;
pub mod csharp;
pub mod dart;
pub mod dockerfile;
pub mod elixir;
pub mod go;
pub mod gleam;
pub mod graphql;
pub mod hare;
pub mod haskell;
pub mod hcl;
pub mod html;
pub mod java;
pub mod javascript;
pub mod jupyter;
pub mod kotlin;
pub mod lua;
pub mod make;
pub mod markdown;
pub mod mdx;
pub mod nim;
pub mod nix;
pub mod odin;
pub mod php;
pub mod polyglot_nb;
pub mod prisma;
pub mod proto;
pub mod puppet;
pub mod python;
pub mod r_lang;
pub mod razor;
pub mod rmarkdown;
pub mod robot;
pub mod ruby;
pub mod rust_lang;
pub mod scala;
pub mod scss;
pub mod sql;
pub mod starlark;
pub mod svelte;
pub mod swift;
pub mod twig;
pub mod typescript;
pub mod vue;
pub mod zig;
// --- Wave 3 plugins (SO 2025 lower-priority) ---
pub mod cobol;
pub mod pascal;
pub mod prolog;
pub mod vba;
// --- Wave 2 plugins (SO 2025 survey) ---
pub mod ada;
pub mod clojure;
pub mod fortran;
pub mod matlab;
pub mod ocaml;
pub mod vbnet;
// --- Wave 7 plugins (SO 2025 top languages) ---
pub mod powershell;
pub mod groovy;
pub mod perl;
pub mod erlang;
pub mod fsharp;
pub mod gdscript;

static DEFAULT_REGISTRY: LazyLock<LanguageRegistry> = LazyLock::new(|| {
    // The generic plugin handles any language with a tree-sitter grammar
    // but no dedicated extractor.
    let generic = Arc::new(GenericPlugin);
    let mut reg = LanguageRegistry::new(generic);

    reg.register(Arc::new(angular::AngularPlugin));
    reg.register(Arc::new(astro::AstroPlugin));
    reg.register(Arc::new(bash::BashPlugin));
    reg.register(Arc::new(bicep::BicepPlugin));
    reg.register(Arc::new(blade::BladePlugin));
    reg.register(Arc::new(c_lang::CLangPlugin));
    reg.register(Arc::new(cmake::CMakePlugin));
    reg.register(Arc::new(csharp::CSharpPlugin));
    reg.register(Arc::new(dart::DartPlugin));
    reg.register(Arc::new(dockerfile::DockerfilePlugin));
    reg.register(Arc::new(elixir::ElixirPlugin));
    reg.register(Arc::new(go::GoPlugin));
    reg.register(Arc::new(gleam::GleamPlugin));
    reg.register(Arc::new(graphql::GraphQlPlugin));
    reg.register(Arc::new(hare::HarePlugin));
    reg.register(Arc::new(haskell::HaskellPlugin));
    reg.register(Arc::new(hcl::HclPlugin));
    reg.register(Arc::new(html::HtmlPlugin));
    reg.register(Arc::new(java::JavaPlugin));
    reg.register(Arc::new(javascript::JavascriptPlugin));
    reg.register(Arc::new(kotlin::KotlinPlugin));
    reg.register(Arc::new(lua::LuaPlugin));
    reg.register(Arc::new(make::MakePlugin));
    reg.register(Arc::new(markdown::MarkdownPlugin));
    reg.register(Arc::new(mdx::MdxPlugin));
    reg.register(Arc::new(nim::NimPlugin));
    reg.register(Arc::new(nix::NixPlugin));
    reg.register(Arc::new(odin::OdinPlugin));
    reg.register(Arc::new(php::PhpPlugin));
    reg.register(Arc::new(prisma::PrismaPlugin));
    reg.register(Arc::new(proto::ProtoPlugin));
    reg.register(Arc::new(puppet::PuppetPlugin));
    reg.register(Arc::new(python::PythonPlugin));
    reg.register(Arc::new(r_lang::RLangPlugin));
    reg.register(Arc::new(razor::RazorPlugin));
    reg.register(Arc::new(robot::RobotPlugin));
    reg.register(Arc::new(ruby::RubyPlugin));
    reg.register(Arc::new(rust_lang::RustLangPlugin));
    reg.register(Arc::new(scala::ScalaPlugin));
    reg.register(Arc::new(scss::ScssPlugin));
    reg.register(Arc::new(sql::SqlPlugin));
    reg.register(Arc::new(starlark::StarlarkPlugin));
    reg.register(Arc::new(svelte::SveltePlugin));
    reg.register(Arc::new(swift::SwiftPlugin));
    reg.register(Arc::new(twig::TwigPlugin));
    reg.register(Arc::new(typescript::TypeScriptPlugin));
    reg.register(Arc::new(vue::VuePlugin));
    reg.register(Arc::new(zig::ZigPlugin));
    // Wave 3
    reg.register(Arc::new(cobol::CobolPlugin));
    reg.register(Arc::new(pascal::PascalPlugin));
    reg.register(Arc::new(prolog::PrologPlugin));
    reg.register(Arc::new(vba::VbaPlugin));
    // Wave 2
    reg.register(Arc::new(ada::AdaPlugin));
    reg.register(Arc::new(clojure::ClojurePlugin));
    reg.register(Arc::new(fortran::FortranPlugin));
    reg.register(Arc::new(matlab::MatlabPlugin));
    reg.register(Arc::new(ocaml::OcamlPlugin));
    reg.register(Arc::new(vbnet::VbNetPlugin));
    // Wave 7
    reg.register(Arc::new(powershell::PowerShellPlugin));
    reg.register(Arc::new(groovy::GroovyPlugin));
    reg.register(Arc::new(perl::PerlPlugin));
    reg.register(Arc::new(erlang::ErlangPlugin));
    reg.register(Arc::new(fsharp::FSharpPlugin));
    reg.register(Arc::new(gdscript::GDScriptPlugin));
    // E5 — notebook family
    reg.register(Arc::new(jupyter::JupyterPlugin));
    reg.register(Arc::new(rmarkdown::RMarkdownPlugin));
    reg.register(Arc::new(rmarkdown::QuartoPlugin));
    reg.register(Arc::new(polyglot_nb::PolyglotNbPlugin));

    reg
});

/// Return a reference to the shared default language registry.
///
/// The registry is built once on first access (all 59 plugins + lookup map)
/// and reused for the lifetime of the process. During the migration period
/// this coexists with the match statement in `indexer/full.rs`; once all
/// languages are migrated, the match disappears and this becomes the sole
/// dispatch mechanism.
pub fn default_registry() -> &'static LanguageRegistry {
    &DEFAULT_REGISTRY
}

/// Collect language-specific resolvers from all registered plugins.
///
/// Derived from `LanguagePlugin::resolver()` — no separate list to maintain.
pub fn default_resolvers() -> Vec<Arc<dyn LanguageResolver>> {
    default_registry()
        .all()
        .iter()
        .filter_map(|plugin| plugin.resolver())
        .collect()
}

/// Collect language-specific connectors from all registered plugins.
///
/// Derived from `LanguagePlugin::connectors()` — no separate list to maintain.
/// The registry calls this to discover plugin-provided connectors alongside
/// any remaining cross-cutting connectors.
pub fn collect_plugin_connectors() -> Vec<Box<dyn crate::connectors::traits::Connector>> {
    default_registry()
        .all()
        .iter()
        .flat_map(|plugin| plugin.connectors())
        .collect()
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
