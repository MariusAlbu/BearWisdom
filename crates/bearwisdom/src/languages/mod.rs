//! Language plugin system for BearWisdom.
//!
//! Each language is a self-contained plugin that provides:
//! - Tree-sitter grammar loading
//! - Scope configuration for qualified name construction
//! - Symbol + reference extraction from source code
//!
//! Resolution (turning reference names into resolved symbol edges) is provided
//! separately via [`crate::indexer::resolve::legacy::LanguageResolver`] to avoid
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
pub mod demand_filter;
mod plugin_defaults;
pub mod registry;
pub mod string_dsl;

use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractedRef, ExtractedSymbol, ExtractionResult};

// Re-export the shared utility from common so existing callers using
// `crate::languages::emit_chain_type_ref` continue to work without changes.
pub use common::emit_chain_type_ref;
pub use plugin_defaults::Synthesized;
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
        plugin_defaults::language_id_for_extension(self, ext)
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
        demand: Option<&std::collections::HashSet<String>>,
    ) -> ExtractionResult {
        plugin_defaults::extract_with_demand(self, source, file_path, lang_id, demand)
    }

    /// Extract with access to the workspace `TypeArena`. Default impl
    /// runs `extract_with_demand` then populates
    /// `ExtractedSymbol::return_type` for type-defining and callable
    /// kinds via `common::populate_return_type_ids`. Plugins with richer
    /// AST-driven type extraction override to intern more complex shapes
    /// (parametrized fields, structural function types, …).
    fn extract_with_arena_and_demand(
        &self,
        source: &str,
        file_path: &str,
        lang_id: &str,
        demand: Option<&std::collections::HashSet<String>>,
        arena: &crate::type_checker::core::types::TypeArena,
    ) -> ExtractionResult {
        plugin_defaults::extract_with_arena_and_demand(self, source, file_path, lang_id, demand, arena)
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

    /// Synthesize symbols that a code generator / annotation processor / macro
    /// would emit but which never appear in the parsed source text — Lombok
    /// `@Data` getters/setters, Rust `#[derive]` impls, C# source-generated
    /// members. The resolver then binds refs to those generated members like
    /// any normal symbol.
    ///
    /// Invoked by `parse_file` AFTER `extract()` (and local-ref filtering) but
    /// BEFORE the file's symbol table is finalized, so the returned symbols
    /// flow through the normal write path and receive real DB ids. The
    /// recognizer reads the already-extracted `symbols` (classes, fields with
    /// their types in `signature`) and `refs` (annotations are emitted as
    /// `TypeRef`s whose `source_symbol_index` points at the annotated symbol),
    /// or re-scans `source` for text-substitution generators.
    ///
    /// Returned symbols MUST carry a correct `qualified_name` (= parent qname +
    /// separator + member name) and `scope_path`; `parent_index` may be left
    /// `None` (member parenting is reconstructed from `qualified_name`). A
    /// recognizer MUST NOT emit a member whose `qualified_name` already exists
    /// in `symbols` — a hand-written declaration always wins.
    ///
    /// `Synthesized::refs` are the synthesized symbols' OWN references — most
    /// importantly a return-type `TypeRef` per synthesized method, so a
    /// synthesized getter / builder method types identically to a real one and
    /// chains resolve through it. Their `source_symbol_index` is RELATIVE to
    /// `Synthesized::symbols` (0 = first synthesized symbol); `parse_file`
    /// rebases them onto the file's symbol table on splice.
    ///
    /// Leaf languages without a generator leave the default (no synthesis).
    fn synthesize_symbols(
        &self,
        _source: &str,
        _symbols: &[ExtractedSymbol],
        _refs: &[ExtractedRef],
    ) -> Synthesized {
        Synthesized::default()
    }

    /// Node kinds that SHOULD produce symbols, per the extraction rules.
    /// Used by `bw coverage` to measure extraction completeness.
    fn symbol_node_kinds(&self) -> &[&str] {
        &[]
    }

    /// Node kinds that SHOULD produce refs/edges, per the extraction rules.
    /// Used by `bw coverage` to measure extraction completeness.
    fn ref_node_kinds(&self) -> &[&str] {
        &[]
    }

    /// Language-intrinsic names: keywords, operators, compiler intrinsics,
    /// primitive types without indexable source, syntax literals, and
    /// generic type parameter conventions. Used by the resolution engine to
    /// classify unresolvable references as "external" rather than
    /// "unresolved", and by `bw coverage` to exclude these identifiers from
    /// the TypeRef denominator. Never includes stdlib function names,
    /// framework DSL names, or package-API names — those come from indexed
    /// ecosystems.
    fn keywords(&self) -> &'static [&'static str] {
        &[]
    }

    /// Relative module specifiers a just-materialized external declaration file
    /// reaches IN ADDITION to its imports/re-exports — e.g. an Angular NgModule
    /// declaration `.d.ts` reaches the `.component`/`.directive` `.d.ts` files it
    /// declares (those are referenced only by selector, so nothing demands them by
    /// name). The generic externals closure resolves each spec relative to the file
    /// and materializes it. Default: none. Keeps framework-specific reachability in
    /// the owning plugin, not the generic resolve pipeline.
    fn external_declaration_reachables(&self, _file_path: &str, _content: &str) -> Vec<String> {
        Vec::new()
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
    fn nested_ref_skip_pairs(&self) -> &[(&'static str, &'static str)] {
        &[]
    }

    /// Return the language profile for this plugin, if one is defined.
    ///
    /// The engine collects profiles from every registered plugin and routes
    /// `Engine::resolve` by `file_ctx.language`. Plugins without a profile
    /// fall through to the legacy per-language resolver path. Phase 5 wires
    /// TypeScript first; subsequent Wave A/B migrations add their profile
    /// in the same per-language file.
    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        None
    }

    // `connectors()` trait method removed (Phase H) — no `impl Connector for X`
    // blocks remain. Per-language flow detection now lives either in resolver
    // FlowEmission emissions or in free `discover_*` functions called from
    // `resolve_connection_points` hooks.

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
    ) {
    }

    /// Contribute reachability entry-points for this language. Each row
    /// names a symbol that anchors the dead-code BFS — `main`, exported
    /// library APIs, Rust `[[bin]]` targets, Python `__main__` blocks,
    /// `package.json` `bin`/`exports` entries, etc.
    ///
    /// Default returns empty. Cross-cutting entry-point sources (HTTP
    /// routes, event handlers, DI bindings, framework lifecycle hooks)
    /// live in `crate::query::entry_points` central contributors so
    /// every language gets them for free; plugin-level entry points are
    /// for language-natural roots that only a per-language plugin can
    /// recognize.
    fn entry_points(
        &self,
        _db: &crate::db::Database,
        _ctx: &crate::indexer::project_context::ProjectContext,
    ) -> Vec<crate::types::EntryPointRow> {
        Vec::new()
    }

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

    /// Re-populate cross-file plugin state after external sources have been
    /// merged into the parsed slice.
    ///
    /// `populate_project_state` runs before external dependencies are walked,
    /// so its `parsed` slice carries no `ext:` files. A plugin whose state
    /// must see externally-walked symbols — e.g. a Robot suite binding
    /// `Library SeleniumLibrary` to a site-packages `.py` file — overrides
    /// this to rebuild against the full slice. The `state` bag still holds
    /// the pre-externals result; an override replaces its own entry.
    ///
    /// Default: no-op. Plugins whose state is fully derived from project
    /// files (the common case) leave this unimplemented and keep the
    /// pre-externals result.
    fn populate_project_state_post_externals(
        &self,
        _state: &mut crate::indexer::plugin_state::PluginStateBag,
        _parsed: &[crate::types::ParsedFile],
        _project_root: &std::path::Path,
        _project_ctx: &crate::indexer::project_context::ProjectContext,
    ) {
    }

    /// Contribute synthetic wildcard imports derived from this plugin's own
    /// cross-file state for one file — e.g. Elixir's `use M` one-hop
    /// injection redirect, where M's `__using__` quote block imports or
    /// aliases a module the `use`ing file never mentions directly.
    ///
    /// Called once per file while `FileContext` is built, after the
    /// profile-driven import scan; the caller appends the result to
    /// `FileContext.imports`, so every existing import-consulting rule
    /// (wildcard, alias-module-qname, …) sees the synthetic entries without
    /// any rule-level change.
    ///
    /// Default: no-op. Most plugins carry no cross-file import state.
    fn extra_wildcard_imports(
        &self,
        _state: &crate::indexer::plugin_state::PluginStateBag,
        _file: &crate::types::ParsedFile,
    ) -> Vec<crate::indexer::resolve::engine::contract::ImportEntry> {
        Vec::new()
    }

    /// Synthesize member symbols a macro-injection construct creates but
    /// which never appear as literal text in the consuming module's own
    /// source — Elixir's `use ExMachina` giving a factory module real
    /// `build/2` etc., with no `def build` anywhere in that module's file.
    ///
    /// Called once per project, after `populate_project_state_post_externals`
    /// (so cross-file plugin state — e.g. Elixir's flattened `use`-injection
    /// map — is final) and before the resolve pass builds its `Compilation`,
    /// over the full merged `parsed` slice (project + externals). Returned
    /// pairs are `(file_path, new_symbols)`; the caller appends `new_symbols`
    /// to that file's `ParsedFile::symbols` and re-persists just that file,
    /// dropping any synthesized symbol whose `qualified_name` collides with
    /// one the file already declares — a hand-written definition always wins.
    ///
    /// A synthesized symbol MUST carry a correct dotted `qualified_name`
    /// (parent qname + "." + member name) and leave `parent_index: None` —
    /// containment is reconstructed from the qname, the same convention
    /// `synthesize_symbols` uses for its own per-file splice.
    ///
    /// Default: no-op. Most plugins have no macro-injected members to
    /// synthesize.
    fn synthesize_project_symbols(
        &self,
        _state: &crate::indexer::plugin_state::PluginStateBag,
        _parsed: &[crate::types::ParsedFile],
    ) -> Vec<(String, Vec<ExtractedSymbol>)> {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Built-in plugin registration
// ---------------------------------------------------------------------------
//
// Module declarations for every language plugin live here (Rust resolves
// `mod X;` against the declaring file's own directory, so they can't move).
// The registry construction that references them — one `reg.register(...)`
// call per plugin, plus the generic fallback — lives in `registry_init.rs`.

mod registry_init;
pub use registry_init::default_registry;

pub mod angular;
pub mod angular_template;
pub mod astro;
pub mod bash;
pub mod bicep;
pub mod blade;
pub mod c_lang;
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
mod generic;
pub mod gleam;
pub mod go;
pub mod gotemplate;
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
pub mod starlark;
pub mod svelte;
pub mod swift;
pub mod systemd;
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
pub mod erlang;
pub mod fsharp;
pub mod gdscript;
pub mod groovy;
pub mod perl;
pub mod powershell;

