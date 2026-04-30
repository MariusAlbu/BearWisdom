// =============================================================================
// ecosystem/ember_handlebars_helpers.rs — Ember/Handlebars built-in helpers
//
// Ember-flavoured Handlebars templates use a fixed set of built-in helpers
// (`if`, `unless`, `each`, `let`, `with`, `mut`, `fn`, `hash`, `array`,
// `concat`, `unique-id`, `helper`) plus the de-facto-standard
// `ember-truth-helpers` addon (`eq`, `not-eq`, `gt`, `gte`, `lt`, `lte`,
// `not`, `or`, `and`, `is-array`, `is-empty`, `is-equal`, `xor`) and
// `@ember/render-modifiers` (`did-insert`, `did-update`, `will-destroy`).
//
// These helpers aren't extracted by parsing the .hbs templates because they
// originate from the Ember runtime / addon JavaScript. Without them, every
// `{{eq this.status 'active'}}` shows up as an unresolved Calls ref.
//
// Activation: any `.hbs` file is present AND the project uses Ember (heuristic:
// `package.json` mentions `ember-source` or the file tree includes an
// `app/` directory with helper files). The default falls back to "any .hbs
// file" since vanilla Handlebars users typically don't define `eq`/`not`/etc.
// at all, so the synthetic symbols never collide.
//
// Helper-name normalization: the Handlebars embedded wrapper rewrites
// `did-insert` → `did_insert` so the JS parser sees a single identifier
// (else `-` parses as subtraction). Synthetic symbols mirror that and use
// the underscore form so name matching is direct.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("ember-handlebars-helpers");
const TAG: &str = "ember-handlebars-helpers";
const LANGUAGES: &[&str] = &["handlebars"];

/// Ember built-in helpers that ship with the framework — always available
/// in any `.hbs` template without import. Names use the underscore form
/// because the Handlebars→JS wrapper rewrites `-` → `_` to keep tokens
/// as single identifiers.
const EMBER_BUILTIN_HELPERS: &[&str] = &[
    // Control / flow
    "if",
    "unless",
    "each",
    "each_in",
    "let",
    "with",
    "yield",
    "outlet",
    "component",
    "in_element",
    "input",
    "textarea",
    // State
    "mut",
    "fn",
    "action",
    "pipe",
    "queue",
    // Data construction
    "hash",
    "array",
    "concat",
    "join",
    // Identity / lookup
    "unique_id",
    "helper",
    "modifier",
    "get",
    "readonly",
    "unbound",
    // Logical (built into Ember/Glimmer or supplied by ember-truth-helpers)
    "eq",
    "not_eq",
    "neq",
    "gt",
    "gte",
    "lt",
    "lte",
    "not",
    "or",
    "and",
    "xor",
    "is_array",
    "is_empty",
    "is_equal",
    "is_undefined",
    // String/format helpers commonly bundled
    "format_number",
    "format_date",
    "format_time",
    "format_relative",
    "format_message",
    "moment_format",
    "moment_to",
    "moment_from",
    "moment_calendar",
    "moment_duration",
    "now",
    "pluralize",
    "singularize",
    "truncate",
    "capitalize",
    "uppercase",
    "lowercase",
    "titleize",
    "humanize",
    // Asset addons (ember-svg-jar, ember-cli-svg-jar)
    "svg_jar",
    "inline_svg",
    "svg",
    // Render-modifier (@ember/render-modifiers)
    "did_insert",
    "did_update",
    "will_destroy",
    // ember-on-modifier / ember-on-helper
    "on",
    "on_key",
    "on_click_outside",
    "on_resize",
    // ember-keyboard
    "key",
    "keyboard",
    // ember-power-select / common form helpers
    "power_select",
    "power_select_multiple",
    "ember_power_select_is_selected",
    "is_selected",
    // ember-css-transitions
    "css_transition",
    "css_transitions",
    // ember-cli-htmlbars / @ember/template
    "html_safe",
    "is_html_safe",
    // ember-moment additional helpers
    "moment_from_now",
    "moment_to_now",
    "moment_unix",
    // ember-route-action-helper / route-actions
    "route_action",
    "toggle_action",
    "pipe_action",
    "queue_actions",
    // Routing
    "link_to",
    "transition_to",
    "replace_with",
    "current_url",
    // Misc commonly-used
    "log",
    "debugger",
    "page_title",
    // node-plop / plop generator template helpers (case conversions)
    "camelCase",
    "snakeCase",
    "dotCase",
    "pathCase",
    "lowerCase",
    "upperCase",
    "sentenceCase",
    "constantCase",
    "titleCase",
    "dashCase",
    "kabobCase",
    "kebabCase",
    "properCase",
    "pascalCase",
];

// ---------------------------------------------------------------------------
// Detection: does this project use Handlebars at all?
// ---------------------------------------------------------------------------

fn project_uses_handlebars(project_root: &Path) -> bool {
    scan_for_hbs(project_root, 0)
}

fn scan_for_hbs(dir: &Path, depth: u32) -> bool {
    if depth > 4 {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || matches!(name, "node_modules" | "target" | "dist" | "build" | "tmp") {
                continue;
            }
            if scan_for_hbs(&path, depth + 1) {
                return true;
            }
        } else if ft.is_file() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.ends_with(".hbs") || name.ends_with(".handlebars") {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Symbol synthesis
// ---------------------------------------------------------------------------

fn fn_sym(name: &str) -> ExtractedSymbol {
    // Helpers are looked up via the TS resolver's bare-name fallback at
    // `__npm_globals__.<name>`. Setting qualified_name to that form lets
    // `by_qualified_name` find the symbol from a Handlebars-embedded JS
    // call without requiring an explicit import statement.
    let qname = format!(
        "{}.{}",
        crate::ecosystem::npm::NPM_GLOBALS_MODULE,
        name
    );
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname,
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("/* Ember/Handlebars helper */ {name}(...)")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

fn synthesize_file() -> ParsedFile {
    let symbols: Vec<ExtractedSymbol> = EMBER_BUILTIN_HELPERS.iter().map(|n| fn_sym(n)).collect();
    let n = symbols.len();
    // The synthetic file uses .ts extension so the TS extractor's view of
    // `__npm_globals__` matches what the chain-walker / npm-globals fallback
    // expects. The actual content is just the symbols list, no parsing.
    ParsedFile {
        path: "ext:ember-handlebars-helpers:globals.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: format!("ember-handlebars-helpers-{n}"),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: vec![None; n],
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: vec![false; n],
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
    }
}

fn synthetic_dep_root() -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "ember-handlebars-helpers".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        root: PathBuf::from("ext:ember-handlebars-helpers"),
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Ecosystem impl
// ---------------------------------------------------------------------------

pub struct EmberHandlebarsHelpersEcosystem;

impl Ecosystem for EmberHandlebarsHelpersEcosystem {
    fn id(&self) -> EcosystemId {
        ID
    }

    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Stdlib
    }

    fn languages(&self) -> &'static [&'static str] {
        LANGUAGES
    }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("handlebars")
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        if !project_uses_handlebars(ctx.project_root) {
            return Vec::new();
        }
        vec![synthetic_dep_root()]
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn uses_demand_driven_parse(&self) -> bool {
        true
    }

    fn parse_metadata_only(&self, _dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        Some(vec![synthesize_file()])
    }
}

impl ExternalSourceLocator for EmberHandlebarsHelpersEcosystem {
    fn ecosystem(&self) -> &'static str {
        TAG
    }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        if !project_uses_handlebars(project_root) {
            return Vec::new();
        }
        vec![synthetic_dep_root()]
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        Some(vec![synthesize_file()])
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<EmberHandlebarsHelpersEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(EmberHandlebarsHelpersEcosystem)).clone()
}

#[cfg(test)]
#[path = "ember_handlebars_helpers_tests.rs"]
mod tests;
