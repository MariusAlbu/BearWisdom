use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use rustc_hash::FxHashMap;

use super::LanguagePlugin;
use crate::ecosystem::externals::ExternalDepRoot;
use crate::indexer::project_context::ProjectContext;
use crate::types::ParsedFile;
use crate::walker::WalkedFile;

/// RAII guard for a registry-wide indexing session. Dropping the guard ends
/// plugins in reverse registration order, matching nested resource setup.
pub struct IndexSession<'a> {
    registry: &'a LanguageRegistry,
}

impl Drop for IndexSession<'_> {
    fn drop(&mut self) {
        for plugin in self.registry.plugins.iter().rev() {
            plugin.end_index_session();
        }
    }
}

/// Central registry mapping language IDs to their plugin implementations.
///
/// Every language ID resolves to either a dedicated plugin or the generic
/// fallback — `get()` never returns `None`.
pub struct LanguageRegistry {
    plugins: Vec<Arc<dyn LanguagePlugin>>,
    by_lang_id: FxHashMap<String, usize>,
    /// Flow configuration strategy key → owning plugin. This deliberately
    /// remains separate from parser language IDs: compact flow keys need not
    /// be accepted by `grammar()`.
    by_flow_strategy: FxHashMap<String, usize>,
    /// File-extension → language_id. Built from every plugin's
    /// `extensions()` + `language_id_for_extension()` during `register()`.
    /// Keys are stored lowercase-with-leading-dot; values are interned
    /// `&'static str` via one Box::leak per unique id (< 100 leaks ever).
    by_extension: FxHashMap<String, &'static str>,
    /// Extensions sorted by descending length — compound extensions like
    /// `.d.ts` / `.html.erb` / `.component.html` are probed before their
    /// single-segment subsumers so the longest match wins.
    ext_lookup_order: Vec<String>,
    generic: Arc<dyn LanguagePlugin>,
}

impl LanguageRegistry {
    /// Create a new registry with the given generic fallback plugin.
    pub fn new(generic: Arc<dyn LanguagePlugin>) -> Self {
        Self {
            plugins: Vec::new(),
            by_lang_id: FxHashMap::default(),
            by_flow_strategy: FxHashMap::default(),
            by_extension: FxHashMap::default(),
            ext_lookup_order: Vec::new(),
            generic,
        }
    }

    /// Register a language plugin. All language IDs it claims are mapped,
    /// and every extension is indexed so `language_by_extension()` can
    /// route files to the right plugin without callers hardcoding ext
    /// tables. First-registered wins on extension collisions.
    pub fn register(&mut self, plugin: Arc<dyn LanguagePlugin>) {
        let idx = self.plugins.len();
        for &lang_id in plugin.language_ids() {
            self.by_lang_id.insert(lang_id.to_string(), idx);
        }
        if let Some(config) = plugin.flow_config() {
            self.by_flow_strategy
                .entry(config.strategy_prefix.to_string())
                .or_insert(idx);
        }
        for &strategy_key in plugin.flow_strategy_aliases() {
            self.by_flow_strategy
                .entry(strategy_key.to_string())
                .or_insert(idx);
        }
        for &ext in plugin.extensions() {
            let key = ext.to_ascii_lowercase();
            if self.by_extension.contains_key(&key) {
                continue;
            }
            let lang_id = plugin
                .language_id_for_extension(ext)
                .unwrap_or_else(|| plugin.id());
            // Intern the id as &'static str. All plugin id/language_id values
            // are string literals in practice, but the trait's `&str` return
            // doesn't expose that to callers — Box::leak is the trivial way
            // to surface the static lifetime through the registry. At most a
            // few hundred unique ids over the process lifetime.
            let leaked: &'static str = Box::leak(lang_id.to_string().into_boxed_str());
            self.by_extension.insert(key.clone(), leaked);
            self.ext_lookup_order.push(key);
        }
        self.plugins.push(plugin);
        // Keep lookup-order sorted by length descending so compound
        // extensions (`.d.ts`, `.component.html`) match before their single
        // segment subsumers. Stable within same length so results are
        // deterministic across runs.
        self.ext_lookup_order.sort_by(|a, b| b.len().cmp(&a.len()));
    }

    /// Get the plugin for a language ID. Returns the dedicated plugin if one
    /// is registered, otherwise the generic fallback. Never returns None.
    pub fn get(&self, lang_id: &str) -> &dyn LanguagePlugin {
        match self.by_lang_id.get(lang_id) {
            Some(&idx) => self.plugins[idx].as_ref(),
            None => self.generic.as_ref(),
        }
    }

    /// Active source-language profile for `lang_id`, with the conservative
    /// default when its plugin does not provide one.
    pub fn profile_for(
        &self,
        lang_id: &str,
    ) -> &'static crate::type_checker::profile::language_profile::LanguageProfile {
        self.get(lang_id)
            .profile()
            .unwrap_or(&crate::type_checker::profile::language_profile::DEFAULT_PROFILE)
    }

    /// Get a dedicated plugin only (no fallback).
    pub fn get_dedicated(&self, lang_id: &str) -> Option<&dyn LanguagePlugin> {
        self.by_lang_id
            .get(lang_id)
            .map(|&idx| self.plugins[idx].as_ref())
    }

    /// Ask only the dedicated owner of a language to recognize an external
    /// virtual-path layout. Unknown language IDs deliberately have no layout.
    pub fn external_virtual_path(
        &self,
        lang_id: &str,
        normalized_absolute_path: &str,
    ) -> Option<String> {
        self.get_dedicated(lang_id)?
            .external_virtual_path(lang_id, normalized_absolute_path)
    }

    /// Return generated grammar-query data from the dedicated language owner.
    /// Unknown IDs and aliases without their own grammar data fail closed.
    pub fn query_builtin_data(&self, lang_id: &str) -> crate::languages::query_builtins::Data {
        self.get_dedicated(lang_id)
            .map(|plugin| plugin.query_builtin_data(lang_id))
            .unwrap_or(crate::languages::query_builtins::Data::EMPTY)
    }

    /// Get the plugin that owns a flow configuration strategy. Flow strategy
    /// keys are intentionally independent of parser language IDs, so callers
    /// cannot accidentally ask a plugin to parse a compact strategy spelling.
    /// Conflicting registrations are resolved first-wins during `register()`.
    pub fn flow_plugin(&self, strategy_key: &str) -> Option<&dyn LanguagePlugin> {
        self.by_flow_strategy
            .get(strategy_key)
            .map(|&idx| self.plugins[idx].as_ref())
    }

    /// All registered dedicated plugins (excludes the generic fallback).
    pub fn all(&self) -> &[Arc<dyn LanguagePlugin>] {
        &self.plugins
    }

    /// Begin an indexing session for every registered plugin. The returned
    /// guard closes the session in reverse registration order when dropped.
    pub fn index_session(&self, project_root: &Path) -> IndexSession<'_> {
        for plugin in &self.plugins {
            plugin.begin_index_session(project_root);
        }
        IndexSession { registry: self }
    }

    /// Dispatch framework route discovery to every registered plugin and
    /// return the total number of route rows it reports inserting.
    pub fn discover_routes(
        &self,
        conn: &rusqlite::Connection,
        project_root: &Path,
        project_ctx: &ProjectContext,
    ) -> u32 {
        self.plugins
            .iter()
            .map(|plugin| plugin.discover_routes(conn, project_root, project_ctx))
            .sum()
    }

    /// Ask active plugins to materialize lifecycle-owned external files while
    /// preserving the opaque project-state bag they share with resolvers.
    pub fn collect_lifecycle_external_files(
        &self,
        project_ctx: &mut ProjectContext,
        parsed: &[ParsedFile],
        external_roots: &[ExternalDepRoot],
        project_root: &Path,
    ) -> Vec<WalkedFile> {
        let mut state = std::mem::take(&mut project_ctx.plugin_state);
        let mut files = Vec::new();
        for plugin in &self.plugins {
            if !plugin
                .language_ids()
                .iter()
                .any(|language| project_ctx.language_presence.contains(*language))
            {
                continue;
            }
            files.extend(plugin.lifecycle_external_files(
                &mut state,
                parsed,
                external_roots,
                project_root,
                project_ctx,
            ));
        }
        project_ctx.plugin_state = state;
        files
    }

    /// Get the tree-sitter grammar for a language ID.
    /// Checks the dedicated plugin first, then the generic fallback.
    pub fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        self.get(lang_id).grammar(lang_id)
    }

    /// Returns true if a dedicated (non-generic) plugin is registered for this ID.
    pub fn has_dedicated(&self, lang_id: &str) -> bool {
        self.by_lang_id.contains_key(lang_id)
    }

    /// Resolve a file name or path to the language id that should be used
    /// when parsing it. Returns `None` when no registered plugin claims any
    /// matching extension.
    ///
    /// Matching is longest-suffix first — so `Foo.d.ts` resolves to
    /// TypeScript's mapping for `.d.ts` (or `.ts` if only the short form is
    /// registered) and `page.component.html` resolves to the Angular
    /// template plugin before the plain HTML plugin. Matching is
    /// case-insensitive for extensions (`.R` and `.r` both hit R).
    ///
    /// This replaces the chained if-else extension matches that used to
    /// live in `indexer::full::make_walked_file` and
    /// `indexer::expand::language_from_file_ext`.
    pub fn language_by_extension(&self, path_or_name: &str) -> Option<&'static str> {
        let lower = path_or_name.to_ascii_lowercase();
        // Strip directory components; extensions are only meaningful on the
        // file-name suffix, and matching with the full path confuses
        // `.component.html` against `.html` in a `src/app.component.html`
        // basename-first scan.
        let file_name = lower
            .rsplit(|c| c == '/' || c == '\\')
            .next()
            .unwrap_or(&lower);
        for ext in &self.ext_lookup_order {
            if file_name.ends_with(ext.as_str()) {
                return self.by_extension.get(ext).copied();
            }
        }
        None
    }

    /// Every registered extension key (lowercased, with leading dot) —
    /// handy for diagnostics and for validating that the extension-to-
    /// language table stays in sync with the set of registered plugins.
    pub fn registered_extensions(&self) -> &[String] {
        &self.ext_lookup_order
    }

    /// Number of dedicated plugins registered.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Whether the registry has no dedicated plugins.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// L1: Return every language actually observed in a parsed-file set —
    /// host file language plus any embedded sub-language recorded on
    /// individual symbols.
    ///
    /// Differs from a naive `files.iter().map(|f| f.language).collect()`
    /// pass in that it also picks up languages that only appear inside
    /// embedded regions (e.g. C# inside Razor `.cshtml`, TypeScript inside
    /// Vue `<script>` blocks). A Razor-only MVC project still reports
    /// `{razor, csharp, javascript}` when the host extractor dispatched
    /// into those sub-extractors.
    pub fn detected_languages(files: &[ParsedFile]) -> HashSet<String> {
        let mut langs = HashSet::new();
        for f in files {
            langs.insert(f.language.clone());
            for origin in f.symbol_origin_languages.iter().flatten() {
                langs.insert(origin.clone());
            }
        }
        langs
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn blank_file(lang: &str, origins: Vec<Option<String>>) -> ParsedFile {
        ParsedFile {
            path: format!("f.{lang}"),
            language: lang.to_string(),
            content_hash: String::new(),
            size: 0,
            line_count: 0,
            mtime: None,
            package_id: None,
            symbols: Vec::new(),
            refs: Vec::new(),
            routes: Vec::new(),
            db_sets: Vec::new(),
            symbol_origin_languages: origins,
            ref_origin_languages: Vec::new(),
            symbol_from_snippet: Vec::new(),
            content: None,
            has_errors: false,
            flow: crate::types::FlowMeta::default(),
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
            component_selectors: Vec::new(),

            plugin_flow_emissions: Vec::new(),
            declared_modules: Vec::new(),
        }
    }

    #[test]
    fn detected_languages_host_only() {
        let files = vec![
            blank_file("csharp", vec![]),
            blank_file("typescript", vec![]),
            blank_file("csharp", vec![]),
        ];
        let langs = LanguageRegistry::detected_languages(&files);
        assert_eq!(
            langs,
            HashSet::from(["csharp".to_string(), "typescript".to_string()])
        );
    }

    #[test]
    fn detected_languages_includes_embedded_origins() {
        // A .cshtml file hosts razor but its symbols come from csharp + javascript.
        let files = vec![blank_file(
            "razor",
            vec![
                None,
                Some("csharp".to_string()),
                Some("javascript".to_string()),
                Some("csharp".to_string()),
            ],
        )];
        let langs = LanguageRegistry::detected_languages(&files);
        assert_eq!(
            langs,
            HashSet::from([
                "razor".to_string(),
                "csharp".to_string(),
                "javascript".to_string()
            ])
        );
    }

    #[test]
    fn detected_languages_empty_input() {
        let langs = LanguageRegistry::detected_languages(&[]);
        assert!(langs.is_empty());
    }

    /// Minimal fake plugin for registry unit tests. Avoids pulling real
    /// tree-sitter grammars into the test graph.
    struct FakePlugin {
        id: &'static str,
        lang_ids: &'static [&'static str],
        exts: &'static [&'static str],
        override_ext: Option<(&'static str, &'static str)>,
    }
    impl LanguagePlugin for FakePlugin {
        fn id(&self) -> &str {
            self.id
        }
        fn language_ids(&self) -> &[&str] {
            self.lang_ids
        }
        fn extensions(&self) -> &[&str] {
            self.exts
        }
        fn language_id_for_extension(&self, ext: &str) -> Option<&str> {
            if let Some((e, id)) = self.override_ext {
                if ext.eq_ignore_ascii_case(e) {
                    return Some(id);
                }
            }
            if self.exts.iter().any(|x| x.eq_ignore_ascii_case(ext)) {
                Some(self.id)
            } else {
                None
            }
        }
        fn grammar(&self, _: &str) -> Option<tree_sitter::Language> {
            None
        }
        fn scope_kinds(&self) -> &[crate::parser::scope_tree::ScopeKind] {
            &[]
        }
        fn extract(&self, _: &str, _: &str, _: &str) -> crate::types::ExtractionResult {
            crate::types::ExtractionResult::default()
        }
    }

    struct RoutePlugin {
        id: &'static str,
        route_count: u32,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct LifecycleTrace(Vec<String>);

    struct LifecyclePlugin {
        id: &'static str,
        events: Arc<Mutex<Vec<String>>>,
    }

    impl LanguagePlugin for LifecyclePlugin {
        fn id(&self) -> &str {
            self.id
        }
        fn language_ids(&self) -> &[&str] {
            match self.id {
                "active" => &["active"],
                "inactive" => &["inactive"],
                _ => &[],
            }
        }
        fn extensions(&self) -> &[&str] {
            &[]
        }
        fn grammar(&self, _: &str) -> Option<tree_sitter::Language> {
            None
        }
        fn scope_kinds(&self) -> &[crate::parser::scope_tree::ScopeKind] {
            &[]
        }
        fn extract(&self, _: &str, _: &str, _: &str) -> crate::types::ExtractionResult {
            crate::types::ExtractionResult::default()
        }
        fn begin_index_session(&self, _: &Path) {
            self.events
                .lock()
                .unwrap()
                .push(format!("begin:{}", self.id));
        }
        fn end_index_session(&self) {
            self.events.lock().unwrap().push(format!("end:{}", self.id));
        }
        fn lifecycle_external_files(
            &self,
            state: &mut crate::indexer::plugin_state::PluginStateBag,
            _: &[ParsedFile],
            _: &[ExternalDepRoot],
            _: &Path,
            _: &ProjectContext,
        ) -> Vec<WalkedFile> {
            let mut trace = state
                .get::<LifecycleTrace>()
                .cloned()
                .unwrap_or_else(|| LifecycleTrace(Vec::new()));
            trace.0.push(self.id.to_string());
            state.set(trace);
            Vec::new()
        }
    }

    impl LanguagePlugin for RoutePlugin {
        fn id(&self) -> &str {
            self.id
        }
        fn language_ids(&self) -> &[&str] {
            &[]
        }
        fn extensions(&self) -> &[&str] {
            &[]
        }
        fn grammar(&self, _: &str) -> Option<tree_sitter::Language> {
            None
        }
        fn scope_kinds(&self) -> &[crate::parser::scope_tree::ScopeKind] {
            &[]
        }
        fn extract(&self, _: &str, _: &str, _: &str) -> crate::types::ExtractionResult {
            crate::types::ExtractionResult::default()
        }
        fn discover_routes(&self, _: &rusqlite::Connection, _: &Path, _: &ProjectContext) -> u32 {
            self.route_count
        }
    }

    fn fake_generic() -> Arc<dyn LanguagePlugin> {
        Arc::new(FakePlugin {
            id: "generic",
            lang_ids: &["generic"],
            exts: &[],
            override_ext: None,
        })
    }

    #[test]
    fn route_discovery_aggregates_every_registered_plugin() {
        let mut reg = LanguageRegistry::new(fake_generic());
        reg.register(Arc::new(RoutePlugin {
            id: "first",
            route_count: 2,
        }));
        reg.register(Arc::new(RoutePlugin {
            id: "second",
            route_count: 3,
        }));
        let conn = rusqlite::Connection::open_in_memory().unwrap();

        assert_eq!(
            reg.discover_routes(&conn, Path::new("."), &ProjectContext::default()),
            5
        );
    }

    #[test]
    fn index_session_closes_plugins_in_reverse_order() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut reg = LanguageRegistry::new(fake_generic());
        for id in ["first", "second"] {
            reg.register(Arc::new(LifecyclePlugin {
                id,
                events: Arc::clone(&events),
            }));
        }

        {
            let _session = reg.index_session(Path::new("."));
            assert_eq!(
                events.lock().unwrap().clone(),
                vec!["begin:first".to_string(), "begin:second".to_string()]
            );
        }
        assert_eq!(
            events.lock().unwrap().clone(),
            vec![
                "begin:first".to_string(),
                "begin:second".to_string(),
                "end:second".to_string(),
                "end:first".to_string()
            ]
        );
    }

    #[test]
    fn lifecycle_external_dispatch_runs_only_active_plugins() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut reg = LanguageRegistry::new(fake_generic());
        for id in ["active", "inactive"] {
            reg.register(Arc::new(LifecyclePlugin {
                id,
                events: Arc::clone(&events),
            }));
        }
        let mut ctx = ProjectContext::default();
        ctx.language_presence.insert("active".to_string());

        let files = reg.collect_lifecycle_external_files(&mut ctx, &[], &[], Path::new("."));

        assert!(files.is_empty());
        assert_eq!(
            ctx.plugin_state.get::<LifecycleTrace>(),
            Some(&LifecycleTrace(vec!["active".to_string()]))
        );
    }

    static FLOW_OWNER_CONFIG: crate::indexer::flow::FlowConfig = crate::indexer::flow::FlowConfig {
        strategy_prefix: "shared-flow",
        assignment_query: "",
        type_guard_query: "",
        discriminant_guard_query: "",
        type_args_query: "",
        literal_type_kinds: &[],
    };

    struct FlowOwnerPlugin {
        id: &'static str,
        aliases: &'static [&'static str],
    }

    impl LanguagePlugin for FlowOwnerPlugin {
        fn id(&self) -> &str {
            self.id
        }
        fn language_ids(&self) -> &[&str] {
            &[]
        }
        fn extensions(&self) -> &[&str] {
            &[]
        }
        fn grammar(&self, _: &str) -> Option<tree_sitter::Language> {
            None
        }
        fn scope_kinds(&self) -> &[crate::parser::scope_tree::ScopeKind] {
            &[]
        }
        fn extract(&self, _: &str, _: &str, _: &str) -> crate::types::ExtractionResult {
            crate::types::ExtractionResult::default()
        }
        fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
            Some(&FLOW_OWNER_CONFIG)
        }
        fn flow_strategy_aliases(&self) -> &'static [&'static str] {
            self.aliases
        }
    }

    #[test]
    fn flow_strategies_are_separate_and_first_registered_owner_wins() {
        let mut reg = LanguageRegistry::new(fake_generic());
        reg.register(Arc::new(FlowOwnerPlugin {
            id: "first",
            aliases: &["shared-alias"],
        }));
        reg.register(Arc::new(FlowOwnerPlugin {
            id: "second",
            aliases: &["shared-flow", "shared-alias"],
        }));

        assert_eq!(reg.flow_plugin("shared-flow").unwrap().id(), "first");
        assert_eq!(reg.flow_plugin("shared-alias").unwrap().id(), "first");
        assert!(reg.get_dedicated("shared-flow").is_none());
        assert!(!reg.has_dedicated("shared-flow"));
    }

    #[test]
    fn extension_lookup_returns_primary_id_by_default() {
        let mut reg = LanguageRegistry::new(fake_generic());
        reg.register(Arc::new(FakePlugin {
            id: "rust",
            lang_ids: &["rust"],
            exts: &[".rs"],
            override_ext: None,
        }));
        assert_eq!(reg.language_by_extension("main.rs"), Some("rust"));
        assert_eq!(reg.language_by_extension("MAIN.RS"), Some("rust"));
        assert_eq!(reg.language_by_extension("dir/subdir/lib.rs"), Some("rust"));
        assert_eq!(reg.language_by_extension("readme.md"), None);
    }

    #[test]
    fn extension_lookup_longest_suffix_wins() {
        // Simulate TypeScript + a ".d.ts" plugin competing — the one
        // claiming the longer extension must win.
        let mut reg = LanguageRegistry::new(fake_generic());
        reg.register(Arc::new(FakePlugin {
            id: "typescript",
            lang_ids: &["typescript"],
            exts: &[".ts"],
            override_ext: None,
        }));
        reg.register(Arc::new(FakePlugin {
            id: "dts",
            lang_ids: &["dts"],
            exts: &[".d.ts"],
            override_ext: None,
        }));
        assert_eq!(reg.language_by_extension("types.d.ts"), Some("dts"));
        assert_eq!(reg.language_by_extension("app.ts"), Some("typescript"));
    }

    #[test]
    fn extension_lookup_honors_language_id_override() {
        // A plugin that handles multiple grammars through distinct
        // language_ids — the override hook must be respected.
        let mut reg = LanguageRegistry::new(fake_generic());
        reg.register(Arc::new(FakePlugin {
            id: "typescript",
            lang_ids: &["typescript", "tsx"],
            exts: &[".ts", ".tsx"],
            override_ext: Some((".tsx", "tsx")),
        }));
        assert_eq!(reg.language_by_extension("app.ts"), Some("typescript"));
        assert_eq!(reg.language_by_extension("App.tsx"), Some("tsx"));
    }

    #[test]
    fn registered_extensions_exposes_every_registered_ext() {
        let mut reg = LanguageRegistry::new(fake_generic());
        reg.register(Arc::new(FakePlugin {
            id: "rust",
            lang_ids: &["rust"],
            exts: &[".rs"],
            override_ext: None,
        }));
        let exts = reg.registered_extensions();
        assert!(exts.iter().any(|e| e == ".rs"));
    }
}
