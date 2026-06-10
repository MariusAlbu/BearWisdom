//! Angular template (`.component.html`, `.container.html`,
//! `.dialog.html`) language plugin.
//!
//! Angular templates are HTML with extra binding syntax:
//!
//!   * `{{ expr }}`                 — interpolation (TypeScript)
//!   * `[prop]="expr"`              — property binding (TypeScript)
//!   * `(event)="expr"`             — event binding (TypeScript)
//!   * `*ngFor="let x of xs"`       — structural directive
//!   * `*ngIf="cond"`               — structural directive
//!   * `<app-child [x]="y" />`      — component usage
//!
//! This plugin parses the host HTML with tree-sitter-html, emits:
//!
//!   * a file-stem `Class` host symbol,
//!   * a `Calls` ref for every PascalCase or kebab-case element tag
//!     (component usage — normalized to PascalCase),
//!   * an embedded TypeScript region per binding expression so
//!     identifier references inside `{{ expr }}`, `[prop]="expr"`, and
//!     `(evt)="expr"` resolve against project symbols.

pub mod embedded;
pub mod extract;
pub(crate) mod profile;

pub use profile::ANGULAR_TEMPLATE_PROFILE;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct AngularTemplatePlugin;

impl LanguagePlugin for AngularTemplatePlugin {
    fn id(&self) -> &str {
        "angular_template"
    }

    fn language_ids(&self) -> &[&str] {
        &["angular_template"]
    }

    fn extensions(&self) -> &[&str] {
        &[".component.html", ".container.html", ".dialog.html"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_html::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source, file_path)
    }

    fn embedded_regions(
        &self,
        source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<EmbeddedRegion> {
        embedded::detect_regions(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &["element", "self_closing_element"]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &["element", "self_closing_element", "attribute"]
    }

    fn keywords(&self) -> &'static [&'static str] {
        crate::languages::typescript::keywords::KEYWORDS
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::ANGULAR_TEMPLATE_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks> {
        // angular_template files (`.component.html`) carry component / directive
        // references that need the same resolver and external-classifier as the
        // `angular` plugin: selector-map lookup for resolved targets and a
        // bare-specifier import fallback for refs to third-party Angular libs
        // whose .d.ts hasn't been pulled into the demand-driven index.
        Some(&crate::languages::angular::hooks::ANGULAR_HOOKS)
    }

    fn companion_file_for_imports(&self, file_path: &str) -> Option<String> {
        crate::languages::angular::hooks::paired_ts_for_template(file_path)
    }
}
