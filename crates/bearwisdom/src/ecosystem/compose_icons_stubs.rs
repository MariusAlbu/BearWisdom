// =============================================================================
// ecosystem/compose_icons_stubs.rs — Jetpack Compose Material Icons stubs
//
// Compose code references the Material icon catalog via nested objects:
//
//     Icon(imageVector = Icons.Filled.Visibility, ...)
//     Icon(imageVector = Icons.AutoMirrored.Filled.ArrowBack, ...)
//     Icon(imageVector = Icons.Outlined.Settings, ...)
//
// The chain-walker resolves `Icons` against the imported class, then tries
// to follow `.Filled` — but `Filled` is a nested object in a library
// (`androidx.compose.material.icons.Icons`) that isn't walked into, so the
// chain dies and `scan_all_type_refs` emits `Filled` as a bare unresolved
// TypeRef. Same pattern for `Outlined`, `Rounded`, `Sharp`, `TwoTone`, and
// `AutoMirrored` (plus its nested variants).
//
// Synthesise the variant hierarchy as Namespace symbols so the Kotlin
// resolver's by_name fallback (step 7, commit 263c036) resolves the bare
// segment refs. The leaf icons (`Visibility`, `ArrowBack`, …) are thousands
// of objects — we don't enumerate them; the chain walker treats the last
// segment as an unresolved member access on a known type, which is fine.
//
// Activation: Kotlin language present.
// =============================================================================

use std::path::Path;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("compose-icons-stubs");
const LEGACY_ECOSYSTEM_TAG: &str = "compose-icons-stubs";
const LANGUAGES: &[&str] = &["kotlin"];

// The five Material Design icon variants plus AutoMirrored (which has its
// own nested Filled/Outlined/… tree for RTL-safe icons).
const ROOT_VARIANTS: &[&str] = &["Filled", "Outlined", "Rounded", "Sharp", "TwoTone"];
const AUTO_MIRRORED_VARIANTS: &[&str] = &["Filled", "Outlined", "Rounded", "Sharp", "TwoTone"];

fn sym(name: &str, qualified_name: &str, kind: SymbolKind, signature: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qualified_name.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(signature.to_string()),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

fn synthesize_file() -> ParsedFile {
    const BASE: &str = "androidx.compose.material.icons.Icons";
    let mut symbols = Vec::new();

    symbols.push(sym("Icons", BASE, SymbolKind::Namespace, "object Icons"));

    for variant in ROOT_VARIANTS {
        symbols.push(sym(
            variant,
            &format!("{BASE}.{variant}"),
            SymbolKind::Namespace,
            &format!("object {variant}"),
        ));
    }

    let am_base = format!("{BASE}.AutoMirrored");
    symbols.push(sym(
        "AutoMirrored",
        &am_base,
        SymbolKind::Namespace,
        "object AutoMirrored",
    ));
    for variant in AUTO_MIRRORED_VARIANTS {
        symbols.push(sym(
            variant,
            &format!("{am_base}.{variant}"),
            SymbolKind::Namespace,
            &format!("object {variant}"),
        ));
    }

    let n_syms = symbols.len();
    ParsedFile {
        path: "ext:compose-icons-stubs:Icons.kt".to_string(),
        language: "kotlin".to_string(),
        content_hash: format!("compose-icons-stubs-{n_syms}"),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: vec![None; n_syms],
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: vec![false; n_syms],
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    }
}

fn synthetic_dep_root() -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "compose-icons-stubs".to_string(),
        version: String::new(),
        root: std::path::PathBuf::from("ext:compose-icons-stubs"),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

pub struct ComposeIconsStubsEcosystem;

impl Ecosystem for ComposeIconsStubsEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("kotlin")
    }

    fn locate_roots(&self, _ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        vec![synthetic_dep_root()]
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn parse_metadata_only(&self, _dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        Some(vec![synthesize_file()])
    }
}

impl ExternalSourceLocator for ComposeIconsStubsEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        vec![synthetic_dep_root()]
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        Some(vec![synthesize_file()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_variants_present() {
        let pf = synthesize_file();
        for name in ["Filled", "Outlined", "Rounded", "Sharp", "TwoTone"] {
            let qname = format!("androidx.compose.material.icons.Icons.{name}");
            let s = pf.symbols.iter().find(|s| s.qualified_name == qname);
            assert!(s.is_some(), "{qname} must be synthesized");
            assert_eq!(s.unwrap().kind, SymbolKind::Namespace);
        }
    }

    #[test]
    fn auto_mirrored_nested_variants_present() {
        let pf = synthesize_file();
        assert!(
            pf.symbols
                .iter()
                .any(|s| s.qualified_name == "androidx.compose.material.icons.Icons.AutoMirrored"),
            "AutoMirrored namespace must be synthesized"
        );
        for name in ["Filled", "Outlined", "Rounded"] {
            let qname = format!(
                "androidx.compose.material.icons.Icons.AutoMirrored.{name}"
            );
            assert!(
                pf.symbols.iter().any(|s| s.qualified_name == qname),
                "{qname} must be synthesized"
            );
        }
    }

    #[test]
    fn bare_filled_findable_by_name() {
        // Kotlin's by_name fallback resolves bare `Filled` via the `name`
        // field. Two entries (root Icons.Filled + AutoMirrored.Filled)
        // share the name — either is acceptable.
        let pf = synthesize_file();
        assert!(
            pf.symbols.iter().filter(|s| s.name == "Filled").count() >= 1,
            "bare `Filled` must be findable by name"
        );
    }

    #[test]
    fn parallel_vecs_are_consistent() {
        let pf = synthesize_file();
        assert_eq!(pf.symbols.len(), pf.symbol_origin_languages.len());
        assert_eq!(pf.symbols.len(), pf.symbol_from_snippet.len());
    }

    #[test]
    fn activation_is_kotlin() {
        let e = ComposeIconsStubsEcosystem;
        assert_eq!(e.languages(), &["kotlin"]);
        assert_eq!(e.kind(), EcosystemKind::Stdlib);
    }
}
