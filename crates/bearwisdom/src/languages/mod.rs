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
pub mod string_dsl;

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

    /// Resolve a file extension claimed by this plugin to the specific
    /// `language_id` that should be stamped on a `WalkedFile` for files with
    /// that extension. Default returns the plugin's primary id (`self.id()`).
    ///
    /// Overridden by plugins that handle multiple variants through a single
    /// extractor — e.g., TypeScript claims both `.ts` → `"typescript"` and
    /// `.tsx` → `"tsx"` because the two files use different tree-sitter
    /// grammars (`LANGUAGE_TYPESCRIPT` vs `LANGUAGE_TSX`) even though the
    /// extractor logic is shared.
    ///
    /// Returning `None` is equivalent to "this extension is not mine" — the
    /// registry falls back to the next plugin claiming the same extension.
    fn language_id_for_extension(&self, ext: &str) -> Option<&str> {
        if self.extensions().iter().any(|e| e.eq_ignore_ascii_case(ext)) {
            // Default to the first declared `language_ids` entry — that's
            // what the registry's `by_lang_id` is keyed on. Falling back to
            // `self.id()` (as the previous default did) silently routed
            // every file to the generic fallback whenever a plugin's
            // directory name diverged from its language tag (PRs 104, 109
            // chased these in rust_lang / bash / c_lang). Plugins with
            // multiple language ids — TypeScript splits .ts vs .tsx, C
            // splits .c vs .cpp — must override this method to pick per
            // extension; the default is for single-id plugins where any
            // member of the list is correct.
            self.language_ids().first().copied().or_else(|| Some(self.id()))
        } else {
            None
        }
    }

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

    /// R6 demand-driven extraction. Same as `extract` except that `demand`,
    /// when `Some`, is the set of top-level declaration names the caller
    /// cares about — declarations outside the set may be dropped. Used when
    /// parsing external sources (node_modules `.d.ts`, PyPI site-packages,
    /// Maven sources jars) so a 1.8MB `lib.dom.d.ts` that the project only
    /// uses 20 types from gets extracted as ~20 declarations instead of tens
    /// of thousands.
    ///
    /// Default impl ignores the demand set and falls back to the full
    /// `extract` path. Languages that have wired up demand filtering
    /// (TypeScript today) override this method.
    fn extract_with_demand(
        &self,
        source: &str,
        file_path: &str,
        lang_id: &str,
        _demand: Option<&std::collections::HashSet<String>>,
    ) -> ExtractionResult {
        self.extract(source, file_path, lang_id)
    }

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

    /// Detect cross-service / cross-module wiring points (HTTP routes, client
    /// calls, DI registrations, IPC commands, event pub/sub, message-queue
    /// bindings, GraphQL resolvers) during extraction. Each plugin emits
    /// connection points for the frameworks its language typically hosts —
    /// e.g. Go plugin emits gin/echo/net-http routes, C# plugin emits
    /// ASP.NET `[HttpGet]` handlers and `IMediator.Send` calls.
    ///
    /// This is the per-plugin half of the "connectors flattened into
    /// language plugins" refactor. Stage 3 of the pipeline folds the
    /// emitted connection points into `flow_edges` by matching
    /// `(kind, key)` pairs with one Start and one Stop role. No DB
    /// round-trip, no connector re-parse.
    ///
    /// Default impl returns an empty vec; languages migrate connector
    /// detection into this hook one framework at a time. The global
    /// `connectors/registry.rs` eager path keeps firing for un-migrated
    /// connectors so behavior is preserved during the rollout.
    ///
    /// Called by `indexer/full::parse_file` AFTER `extract()` returns, so
    /// a host extractor that already parsed the source can cache its
    /// tree-sitter tree across both calls if needed.
    fn extract_connection_points(
        &self,
        _source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<crate::types::ConnectionPoint> {
        Vec::new()
    }

    /// Post-parse hook for connectors that need cross-file joins (class
    /// inheritance lookups, DI-container resolution, method enumeration by
    /// kind under a given parent class). Called AFTER symbols + edges have
    /// been written to the DB, so plugins can query the indexed graph.
    ///
    /// Returned points go through the same `(file_id, line, protocol,
    /// direction, key, method)` dedupe as plugin-emitted points from
    /// `extract_connection_points` and registry-emitted points from
    /// `Connector::extract`.
    ///
    /// Default impl returns empty. Plugins with DB-dependent connectors
    /// (`*Server` inheritance for gRPC stops, class+method joins for
    /// REST route controllers, DI registration chains) override this
    /// instead of registering `Connector::extract` impls at the registry
    /// level.
    fn resolve_connection_points(
        &self,
        _db: &crate::db::Database,
        _project_root: &std::path::Path,
        _ctx: &crate::indexer::project_context::ProjectContext,
    ) -> Vec<crate::connectors::types::ConnectionPoint> {
        Vec::new()
    }

    /// Incremental variant — pass `changed_paths` so plugins owning
    /// connectors with disk-read scans (e.g. C# DI / event-handler regex
    /// sweeps) can scope to changed files. Default falls back to the
    /// full `resolve_connection_points` for plugins that don't
    /// distinguish.
    fn resolve_connection_points_incremental(
        &self,
        db: &crate::db::Database,
        project_root: &std::path::Path,
        ctx: &crate::indexer::project_context::ProjectContext,
        _changed_paths: &std::collections::HashSet<String>,
    ) -> Vec<crate::connectors::types::ConnectionPoint> {
        self.resolve_connection_points(db, project_root, ctx)
    }

    /// Node kinds that SHOULD produce symbols, per the extraction rules.
    /// Used by `bw coverage` to measure extraction completeness.
    fn symbol_node_kinds(&self) -> &[&str] { &[] }

    /// Node kinds that SHOULD produce refs/edges, per the extraction rules.
    /// Used by `bw coverage` to measure extraction completeness.
    fn ref_node_kinds(&self) -> &[&str] { &[] }

    /// Language-intrinsic names: keywords, operators, compiler intrinsics,
    /// primitive types without indexable source, syntax literals, and
    /// generic type parameter conventions. Used by the resolution engine to
    /// classify unresolvable references as "external" rather than
    /// "unresolved", and by `bw coverage` to exclude these identifiers from
    /// the TypeRef denominator. Never includes stdlib function names,
    /// framework DSL names, or package-API names — those come from indexed
    /// ecosystems.
    fn keywords(&self) -> &'static [&'static str] { &[] }

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

    /// Return the language type checker for this plugin, if one exists.
    ///
    /// One impl per typed language (TypeScript, C#, Rust, etc.). Untyped /
    /// dynamically-typed languages return `None` and the engine treats them
    /// as having no type-level capabilities. Aggregated by
    /// `crate::type_checker::default_type_checkers()`.
    ///
    /// PR 1 of the type-checker consolidation — see decision-2026-04-27-e75.
    /// Trait surface stays minimal until subsequent PRs port behavior in.
    fn type_checker(&self) -> Option<Arc<dyn crate::type_checker::TypeChecker>> { None }

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

    /// R5 per-file flow-typing configuration. Return `Some(&FLOW_CONFIG)` to
    /// opt into forward inference, conditional narrowing, and call-site
    /// generics. The default `None` disables flow-typing for this language
    /// at zero cost — the resolver's cache stays empty and chain walkers'
    /// `local_type` lookups all return None.
    ///
    /// See `crate::indexer::flow::FlowConfig` for the query contract.
    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        None
    }

    /// Populate cross-file plugin state once per index pass.
    ///
    /// Called after all files are parsed, before resolution. The result is
    /// stored in `ProjectContext::plugin_state` and made available to
    /// resolvers. Default: no-op.
    fn populate_project_state(
        &self,
        _state: &mut crate::indexer::plugin_state::PluginStateBag,
        _parsed: &[crate::types::ParsedFile],
        _project_root: &std::path::Path,
        _project_ctx: &crate::indexer::project_context::ProjectContext,
    ) {
    }
}

// ---------------------------------------------------------------------------
// Built-in plugin registration
// ---------------------------------------------------------------------------

use std::sync::{Arc, LazyLock};

pub mod angular;
pub mod angular_template;
pub mod astro;
pub mod bash;
pub mod bicep;
pub mod blade;
pub mod c_lang;
mod generic;
pub mod cmake;
pub mod crontab;
pub mod csharp;
pub mod dart;
pub mod dockerfile;
pub mod eex;
pub mod ejs;
pub mod elixir;
pub mod erb;
pub mod freemarker;
pub mod go;
pub mod gotemplate;
pub mod gleam;
pub mod graphql;
pub mod gsp;
pub mod haml;
pub mod handlebars;
pub mod hare;
pub mod haskell;
pub mod hcl;
pub mod heex;
pub mod html;
pub mod java;
pub mod javascript;
pub mod jinja;
pub mod jsp;
pub mod jupyter;
pub mod kotlin;
pub mod liquid;
pub mod lua;
pub mod make;
pub mod mako;
pub mod markdown;
pub mod mdx;
pub mod nginx;
pub mod nim;
pub mod nix;
pub mod nunjucks;
pub mod odin;
pub mod php;
pub mod polyglot_nb;
pub mod prisma;
pub mod proto;
pub mod pug;
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
pub mod shakespeare;
pub mod slim;
pub mod smarty;
pub mod sql;
pub mod systemd;
pub mod starlark;
pub mod svelte;
pub mod swift;
pub mod templ;
pub mod thymeleaf;
pub mod twig;
pub mod typescript;
pub mod velocity;
pub mod vue;
pub mod yaml;
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
    reg.register(Arc::new(angular_template::AngularTemplatePlugin));
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
    reg.register(Arc::new(yaml::YamlPlugin));
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
    // E8 — Node template engines
    reg.register(Arc::new(handlebars::HandlebarsPlugin));
    reg.register(Arc::new(pug::PugPlugin));
    reg.register(Arc::new(ejs::EjsPlugin));
    reg.register(Arc::new(nunjucks::NunjucksPlugin));
    // E9 — Ruby template engines
    reg.register(Arc::new(erb::ErbPlugin));
    reg.register(Arc::new(slim::SlimPlugin));
    reg.register(Arc::new(haml::HamlPlugin));
    // E10 — Python templates, E11 — Liquid
    reg.register(Arc::new(jinja::JinjaPlugin));
    reg.register(Arc::new(liquid::LiquidPlugin));
    // E12 — Go templates + Templ
    reg.register(Arc::new(gotemplate::GoTemplatePlugin));
    reg.register(Arc::new(templ::TemplPlugin));
    // E13 — Phoenix HEEx
    reg.register(Arc::new(heex::HeexPlugin));
    // E22 — Elixir EEx
    reg.register(Arc::new(eex::EexPlugin));
    // E23 — Python Mako, PHP Smarty
    reg.register(Arc::new(mako::MakoPlugin));
    reg.register(Arc::new(smarty::SmartyPlugin));
    // E25 — Nginx
    reg.register(Arc::new(nginx::NginxPlugin));
    // E26 — systemd + crontab
    reg.register(Arc::new(systemd::SystemdPlugin));
    reg.register(Arc::new(crontab::CrontabPlugin));
    // E20 — JVM template engines
    reg.register(Arc::new(freemarker::FreemarkerPlugin));
    reg.register(Arc::new(jsp::JspPlugin));
    reg.register(Arc::new(velocity::VelocityPlugin));
    reg.register(Arc::new(gsp::GspPlugin));
    reg.register(Arc::new(thymeleaf::ThymeleafPlugin));
    // E21 — Yesod Shakespearean template plugins
    reg.register(Arc::new(shakespeare::HamletPlugin));
    reg.register(Arc::new(shakespeare::CassiusPlugin));
    reg.register(Arc::new(shakespeare::LuciusPlugin));
    reg.register(Arc::new(shakespeare::JuliusPlugin));

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

/// Drive a legacy `Connector` (with `detect` + `extract`) from inside a
/// plugin's `resolve_connection_points` impl. Keeps the existing per-connector
/// bodies while moving invocation ownership from the registry to the plugin.
/// Returns empty Vec when the connector's detect returns false OR its extract
/// errors (logged, non-fatal).
pub fn drive_connector(
    c: &dyn crate::connectors::traits::Connector,
    db: &crate::db::Database,
    project_root: &std::path::Path,
    ctx: &crate::indexer::project_context::ProjectContext,
) -> Vec<crate::connectors::types::ConnectionPoint> {
    if !c.detect(ctx) {
        return Vec::new();
    }
    match c.extract(db.conn(), project_root) {
        Ok(pts) => pts,
        Err(e) => {
            tracing::warn!(
                "connector {}: resolve_connection_points failed: {e}",
                c.descriptor().name
            );
            Vec::new()
        }
    }
}

/// Incremental variant of `drive_connector` — routes to the connector's
/// `incremental_extract` so disk scans get scoped to `changed_paths`.
pub fn drive_connector_incremental(
    c: &dyn crate::connectors::traits::Connector,
    db: &crate::db::Database,
    project_root: &std::path::Path,
    ctx: &crate::indexer::project_context::ProjectContext,
    changed_paths: &std::collections::HashSet<String>,
) -> Vec<crate::connectors::types::ConnectionPoint> {
    if !c.detect(ctx) {
        return Vec::new();
    }
    match c.incremental_extract(db.conn(), project_root, changed_paths) {
        Ok(pts) => pts,
        Err(e) => {
            tracing::warn!(
                "connector {}: incremental_resolve_connection_points failed: {e}",
                c.descriptor().name
            );
            Vec::new()
        }
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
