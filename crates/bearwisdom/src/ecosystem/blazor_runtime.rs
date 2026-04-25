// =============================================================================
// ecosystem/blazor_runtime.rs — Blazor JS runtime synthetic stubs
//
// Blazor exposes a `Blazor` object on the browser's `window` global that
// consumer code (including Microsoft's own ReconnectModal.razor.js template
// and user-authored .razor.js companions) uses for SignalR circuit management
// and JS interop:
//
//   await Blazor.reconnect();
//   await Blazor.resumeCircuit();
//   Blazor.addEventListener("components-reconnect-state-changed", …);
//   Blazor.registerCustomEventType("custom", { … });
//
// The actual `Blazor` symbol is declared in the ASP.NET Core `@microsoft`
// internal TypeScript packages that ship INSIDE the SDK, not as a public
// npm dependency. So consumer projects never have the real types on disk —
// only the calls against them. Without synthetic stubs these land as
// unresolved `Blazor.reconnect` / `Blazor.resumeCircuit` refs across every
// Blazor template-derived project.
//
// Synthesis produces an `interface Blazor { reconnect, resumeCircuit, … }`
// that the TypeScript resolver's chain walker picks up via declaration
// merging with any consumer-local `interface Blazor { … }` augmentation.
//
// Activation: Razor OR Blazor files present in the project.
// =============================================================================

use std::path::Path;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("blazor-runtime");
const LEGACY_ECOSYSTEM_TAG: &str = "blazor-runtime";
const LANGUAGES: &[&str] = &["razor", "typescript", "javascript"];

// =============================================================================
// Blazor runtime API surface
// =============================================================================

/// Canonical methods on the global `Blazor` runtime object. Sourced from
/// Microsoft's ReconnectModal.razor.js template and `@microsoft/dotnet-js-interop`
/// internal types. Each becomes a `property` on the synthetic `Blazor`
/// interface — which makes `Blazor.reconnect()` resolve as a Calls edge to
/// the property (matches the same relaxation that handles DOM interface
/// methods).
const BLAZOR_METHODS: &[&str] = &[
    // Circuit lifecycle (SignalR reconnection).
    "reconnect",
    "resumeCircuit",
    "pauseCircuit",
    "start",
    "stop",
    // Event plumbing.
    "addEventListener",
    "removeEventListener",
    "registerCustomEventType",
    // WebAssembly runtime handles — both server and wasm hosting expose
    // these, frequently poked at from init scripts.
    "navigateTo",
    "rootComponents",
    "runtime",
    "platform",
    "disconnect",
    // Reconnection handler customisation.
    "defaultReconnectionHandler",
    // Internal (consumer code nonetheless touches `_internal` in practice).
    "_internal",
];

const BLAZOR_PROPS: &[&str] = &[
    // The `theme` sub-surface the fluentui-blazor Core.Assets code reads.
    "theme",
];

// =============================================================================
// Synthesis
// =============================================================================

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
    let mut symbols = Vec::new();

    // Top-level `Blazor` interface — consumer augmentation merges with this
    // via TypeScript declaration merging.
    symbols.push(sym("Blazor", "Blazor", SymbolKind::Interface, "interface Blazor"));

    // Members live as `Blazor.<name>` qualified names so chain-walker's
    // `{type}.{member}` lookup finds them.
    for name in BLAZOR_METHODS {
        symbols.push(sym(
            name,
            &format!("Blazor.{name}"),
            SymbolKind::Property,
            &format!("{name}: (...args: any[]) => any"),
        ));
    }
    for name in BLAZOR_PROPS {
        symbols.push(sym(
            name,
            &format!("Blazor.{name}"),
            SymbolKind::Property,
            &format!("{name}: any"),
        ));
    }

    // Expose `Blazor` as a Variable (global) too so single-segment refs
    // like `Blazor` (without a chain) don't go unresolved.
    symbols.push(sym(
        "Blazor",
        "Blazor",
        SymbolKind::Variable,
        "declare const Blazor: Blazor",
    ));

    let n_syms = symbols.len();
    ParsedFile {
        path: "ext:blazor-runtime:Blazor.d.ts".to_string(),
        language: "typescript".to_string(),
        content_hash: format!("blazor-runtime-{n_syms}"),
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

// =============================================================================
// Ecosystem impl
// =============================================================================

fn synthetic_dep_root() -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "blazor-runtime".to_string(),
        version: String::new(),
        root: std::path::PathBuf::from("ext:blazor-runtime"),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

pub struct BlazorRuntimeEcosystem;

impl Ecosystem for BlazorRuntimeEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::LanguagePresent("razor"),
            EcosystemActivation::LanguagePresent("csharp"),
        ])
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

impl ExternalSourceLocator for BlazorRuntimeEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        vec![synthetic_dep_root()]
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        Some(vec![synthesize_file()])
    }
}

#[cfg(test)]
#[path = "blazor_runtime_tests.rs"]
mod tests;
