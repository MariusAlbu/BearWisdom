// =============================================================================
// ecosystem/sdl_synthetics.rs — SDL2/SDL3 C header synthetic stubs
//
// SDL2 and SDL3 are distributed as compiled libraries with C headers.
// When a project includes SDL via a system installation or a FetchContent
// build step, the headers are not present in the project tree as parseable
// source files. Tree-sitter can therefore not index SDL symbols, leaving
// every `SDL_CreateWindow`, `SDL_GL_SetAttribute`, etc. as unresolved refs.
//
// This module synthesises the SDL API surface reachable from the cpp-clay
// test project, covering:
//   * Window / renderer / texture / surface lifecycle
//   * OpenGL context management (SDL_GL_*)
//   * Event polling
//   * Input (mouse, keyboard)
//   * Audio
//   * Timing / performance counters
//   * Logging / error reporting
//   * Math utilities (SDL_min, SDL_max, SDL_sinf, SDL_cosf, SDL_roundf)
//   * Memory (SDL_malloc, SDL_calloc, SDL_free)
//   * SDL3 additions (SDL_AppResult, SDL_RenderTexture, SDL_SetRenderClipRect,
//     SDL_CreateWindowAndRenderer, SDL_ShowWindow, SDL_SetWindowResizable,
//     SDL_AddEventWatch, SDL_FColor, SDL_RenderLines)
//
// SDL types (SDL_Window, SDL_Event, Uint8, Uint32, SDL_GLContext, etc.) are
// already handled in `c_lang/predicates.rs::is_c_builtin` as type-level
// builtins, so we only need to synthesize the functions and enum constants
// that appear as unresolved call-site refs.
//
// Activation: C or C++ files are present AND any SDL header include is
// detected in the project source files. locate_roots scans the project for
// SDL include patterns and returns empty when SDL is absent, preventing
// false activations on unrelated C projects.
// =============================================================================

use std::path::Path;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("sdl-synthetics");
const TAG: &str = "sdl-synthetics";
const LANGUAGES: &[&str] = &["c", "cpp"];

// ---------------------------------------------------------------------------
// SDL functions observed in cpp-clay unresolved refs
// ---------------------------------------------------------------------------

/// SDL2/SDL3 functions that appear in cpp-clay's unresolved-refs table.
/// Covers both SDL2 and SDL3 API surfaces (names differ in some cases).
const SDL_FUNCTIONS: &[&str] = &[
    // Initialization / teardown
    "SDL_Init",
    "SDL_Quit",
    // Window management
    "SDL_CreateWindow",
    "SDL_CreateWindowAndRenderer",
    "SDL_DestroyWindow",
    "SDL_GetWindowSize",
    "SDL_ShowWindow",
    "SDL_SetWindowResizable",
    // Renderer lifecycle
    "SDL_CreateRenderer",
    "SDL_DestroyRenderer",
    "SDL_RenderClear",
    "SDL_RenderPresent",
    // Texture operations
    "SDL_CreateTextureFromSurface",
    "SDL_DestroyTexture",
    // Surface operations
    "SDL_FreeSurface",
    // Drawing
    "SDL_SetRenderDrawColor",
    "SDL_SetRenderDrawBlendMode",
    "SDL_RenderFillRect",
    "SDL_RenderFillRectF",
    "SDL_RenderCopy",
    "SDL_RenderGeometry",
    "SDL_RenderLines",
    "SDL_RenderTexture",   // SDL3 name for SDL_RenderCopy equivalent
    "SDL_RenderSetClipRect",
    "SDL_SetRenderClipRect",  // SDL3 variant
    // OpenGL
    "SDL_GL_SetAttribute",
    "SDL_GL_CreateContext",
    "SDL_GL_SwapWindow",
    "SDL_GL_GetDrawableSize",
    // Events
    "SDL_PollEvent",
    "SDL_AddEventWatch",
    // Input
    "SDL_GetMouseState",
    // Timing / performance
    "SDL_Delay",
    "SDL_GetPerformanceCounter",
    "SDL_GetPerformanceFrequency",
    // Hints
    "SDL_SetHint",
    // Error / logging
    "SDL_GetError",
    "SDL_Log",
    "SDL_LogError",
    // Memory utilities
    "SDL_malloc",
    "SDL_calloc",
    "SDL_free",
    // Math utilities (inline functions / macros in SDL_stdinc.h)
    "SDL_min",
    "SDL_max",
    "SDL_sinf",
    "SDL_cosf",
    "SDL_roundf",
];

// ---------------------------------------------------------------------------
// SDL enum / constant values observed as unresolved refs
// ---------------------------------------------------------------------------

/// SDL_GLattr enum values used with SDL_GL_SetAttribute.
const SDL_ENUM_CONSTANTS: &[&str] = &[
    // SDL_GLattr enum values — used as first arg to SDL_GL_SetAttribute
    "SDL_GL_CONTEXT_MAJOR_VERSION",
    "SDL_GL_CONTEXT_MINOR_VERSION",
    "SDL_GL_CONTEXT_PROFILE_MASK",
    "SDL_GL_CONTEXT_PROFILE_CORE",
    "SDL_GL_CONTEXT_PROFILE_ES",
    "SDL_GL_DOUBLEBUFFER",
    "SDL_GL_DEPTH_SIZE",
    "SDL_GL_STENCIL_SIZE",
    "SDL_GL_MULTISAMPLEBUFFERS",
    "SDL_GL_MULTISAMPLESAMPLES",
    "SDL_GL_ACCELERATED_VISUAL",
    // SDL_BlendMode enum values
    "SDL_BLENDMODE_NONE",
    "SDL_BLENDMODE_BLEND",
    "SDL_BLENDMODE_ADD",
    "SDL_BLENDMODE_MOD",
    // SDL3: SDL_AppResult
    "SDL_APP_CONTINUE",
    "SDL_APP_SUCCESS",
    "SDL_APP_FAILURE",
];

// ---------------------------------------------------------------------------
// SDL types not already in predicates.rs::is_c_builtin
// ---------------------------------------------------------------------------

/// SDL3 and lesser-used SDL2 types not yet in is_c_builtin.
const SDL_TYPES: &[&str] = &[
    // SDL3 additions
    "SDL_AppResult",
    "SDL_FColor",
    // SDL_BUTTON macro — technically a macro but referenced as a function call
    // in tree-sitter's view; keep it here so it resolves.
    "SDL_BUTTON",
];

// ---------------------------------------------------------------------------
// Detection: does this project use SDL?
// ---------------------------------------------------------------------------

/// Scan the project for SDL include directives.
///
/// Checks common indicators: CMakeLists.txt referencing SDL, or source files
/// containing `#include <SDL.h>`, `#include <SDL2/SDL.h>`, or
/// `#include <SDL3/SDL.h>`.
fn project_uses_sdl(project_root: &Path) -> bool {
    // Fast path: check CMakeLists.txt for SDL references.
    let cmake = project_root.join("CMakeLists.txt");
    if cmake.is_file() {
        if let Ok(content) = std::fs::read_to_string(&cmake) {
            if content.contains("SDL") {
                return true;
            }
        }
    }
    // Scan source files for SDL includes. Stop at first match.
    scan_for_sdl_include(project_root, 0)
}

fn scan_for_sdl_include(dir: &Path, depth: u32) -> bool {
    if depth > 3 { return false }
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip hidden dirs and vendor noise.
            if name.starts_with('.') || matches!(name, "node_modules" | "target") {
                continue;
            }
            if scan_for_sdl_include(&path, depth + 1) {
                return true;
            }
        } else if ft.is_file() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !matches!(
                name.rsplit('.').next().unwrap_or(""),
                "c" | "cpp" | "cc" | "cxx" | "h" | "hpp" | "txt"
            ) {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                // Match any SDL include or CMake find_package / FetchContent pattern.
                if content.contains("<SDL") || content.contains("SDL2") || content.contains("SDL3") {
                    return true;
                }
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Symbol construction helpers
// ---------------------------------------------------------------------------

fn fn_sym(name: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("/* SDL header */ SDL_DECLSPEC {name}()")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

fn enum_sym(name: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::EnumMember,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("/* SDL enum */ {name}")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

fn type_sym(name: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Struct,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("/* SDL type */ typedef {name}")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

// ---------------------------------------------------------------------------
// ParsedFile synthesis
// ---------------------------------------------------------------------------

fn synthesize_file() -> ParsedFile {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();

    for &f in SDL_FUNCTIONS {
        symbols.push(fn_sym(f));
    }
    for &c in SDL_ENUM_CONSTANTS {
        symbols.push(enum_sym(c));
    }
    for &t in SDL_TYPES {
        symbols.push(type_sym(t));
    }

    let n = symbols.len();
    ParsedFile {
        path: "ext:sdl-synthetics:sdl_generated.h".to_string(),
        language: "c".to_string(),
        content_hash: format!("sdl-synthetics-{n}"),
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
        module_path: "sdl-synthetics".to_string(),
        version: String::new(),
        root: std::path::PathBuf::from("ext:sdl-synthetics"),
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Ecosystem impl
// ---------------------------------------------------------------------------

pub struct SdlSyntheticsEcosystem;

impl Ecosystem for SdlSyntheticsEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        // SDL is declared in the C/C++ project's build manifest. Plain
        // C/C++ projects without an SDL dep don't pay the synthetic-stub
        // cost. Three canonical declaration channels: CMake's
        // find_package(SDL2) / find_package(SDL3), vcpkg.json's sdl2 dep,
        // and Conan's sdl requirement.
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestFieldContains {
                manifest_glob: "**/CMakeLists.txt",
                field_path: "",
                value: "find_package(SDL",
            },
            EcosystemActivation::ManifestFieldContains {
                manifest_glob: "**/vcpkg.json",
                field_path: "dependencies",
                value: "sdl2",
            },
            EcosystemActivation::ManifestFieldContains {
                manifest_glob: "**/vcpkg.json",
                field_path: "dependencies",
                value: "sdl3",
            },
            EcosystemActivation::ManifestFieldContains {
                manifest_glob: "**/conanfile.txt",
                field_path: "",
                value: "sdl/",
            },
            EcosystemActivation::ManifestFieldContains {
                manifest_glob: "**/conanfile.py",
                field_path: "",
                value: "\"sdl/",
            },
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        if !project_uses_sdl(ctx.project_root) {
            return Vec::new();
        }
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

impl ExternalSourceLocator for SdlSyntheticsEcosystem {
    fn ecosystem(&self) -> &'static str { TAG }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        if !project_uses_sdl(project_root) {
            return Vec::new();
        }
        vec![synthetic_dep_root()]
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        Some(vec![synthesize_file()])
    }
}

#[cfg(test)]
#[path = "sdl_synthetics_tests.rs"]
mod tests;
