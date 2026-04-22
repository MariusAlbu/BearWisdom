// Tests for sdl_synthetics — in sibling file per feedback_tests_in_separate_files.md

use super::*;

#[test]
fn synthesized_file_parallel_vecs_consistent() {
    let pf = synthesize_file();
    assert_eq!(pf.symbols.len(), pf.symbol_origin_languages.len());
    assert_eq!(pf.symbols.len(), pf.symbol_from_snippet.len());
}

#[test]
fn core_sdl_functions_present() {
    let pf = synthesize_file();
    let names: Vec<&str> = pf.symbols.iter().map(|s| s.name.as_str()).collect();

    for expected in [
        "SDL_Init",
        "SDL_Quit",
        "SDL_CreateWindow",
        "SDL_DestroyWindow",
        "SDL_CreateRenderer",
        "SDL_DestroyRenderer",
        "SDL_RenderClear",
        "SDL_RenderPresent",
        "SDL_GL_SetAttribute",
        "SDL_GL_CreateContext",
        "SDL_GL_SwapWindow",
        "SDL_PollEvent",
        "SDL_GetError",
        "SDL_SetRenderDrawColor",
        "SDL_RenderGeometry",
        "SDL_GetPerformanceCounter",
        "SDL_GetPerformanceFrequency",
    ] {
        assert!(names.contains(&expected), "{expected} must be synthesized");
    }
}

#[test]
fn sdl3_additions_present() {
    let pf = synthesize_file();
    let names: Vec<&str> = pf.symbols.iter().map(|s| s.name.as_str()).collect();

    assert!(names.contains(&"SDL_RenderTexture"), "SDL3 SDL_RenderTexture must be present");
    assert!(names.contains(&"SDL_SetRenderClipRect"), "SDL3 SDL_SetRenderClipRect must be present");
    assert!(names.contains(&"SDL_CreateWindowAndRenderer"), "SDL3 SDL_CreateWindowAndRenderer must be present");
    assert!(names.contains(&"SDL_ShowWindow"));
    assert!(names.contains(&"SDL_SetWindowResizable"));
    assert!(names.contains(&"SDL_AddEventWatch"));
    assert!(names.contains(&"SDL_RenderLines"));
}

#[test]
fn math_utilities_present() {
    let pf = synthesize_file();
    let names: Vec<&str> = pf.symbols.iter().map(|s| s.name.as_str()).collect();

    assert!(names.contains(&"SDL_min"));
    assert!(names.contains(&"SDL_max"));
    assert!(names.contains(&"SDL_sinf"));
    assert!(names.contains(&"SDL_cosf"));
    assert!(names.contains(&"SDL_roundf"));
}

#[test]
fn memory_utilities_present() {
    let pf = synthesize_file();
    let names: Vec<&str> = pf.symbols.iter().map(|s| s.name.as_str()).collect();

    assert!(names.contains(&"SDL_malloc"));
    assert!(names.contains(&"SDL_calloc"));
    assert!(names.contains(&"SDL_free"));
}

#[test]
fn sdl_functions_are_function_kind() {
    let pf = synthesize_file();
    for sym in &pf.symbols {
        if SDL_FUNCTIONS.contains(&sym.name.as_str()) {
            assert_eq!(
                sym.kind,
                crate::types::SymbolKind::Function,
                "{} should be Function kind",
                sym.name
            );
        }
    }
}

#[test]
fn activation_covers_c_and_cpp() {
    let eco = SdlSyntheticsEcosystem;
    assert!(eco.languages().contains(&"c"));
    assert!(eco.languages().contains(&"cpp"));
    assert_eq!(eco.kind(), crate::ecosystem::EcosystemKind::Stdlib);
    assert!(eco.uses_demand_driven_parse());
}

#[test]
fn no_sdl_project_returns_empty_roots() {
    use std::path::Path;
    // A temp-dir with no SDL references — locate_roots should return empty.
    let tmp = std::env::temp_dir();
    // Note: if /tmp happens to have CMakeLists.txt or C files referencing SDL,
    // this test may be flaky. But /tmp is generally clean.
    // We use a newly-created empty dir instead.
    let dir = tempfile::tempdir().expect("tempdir");
    let eco = SdlSyntheticsEcosystem;
    let roots = ExternalSourceLocator::locate_roots(&eco, dir.path());
    assert!(
        roots.is_empty(),
        "locate_roots must return empty for projects without SDL"
    );
}

#[test]
fn sdl_cmake_project_returns_root() {
    use std::io::Write as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let cmake = dir.path().join("CMakeLists.txt");
    std::fs::File::create(&cmake)
        .unwrap()
        .write_all(b"find_package(SDL2 REQUIRED)\ntarget_link_libraries(app SDL2::SDL2)\n")
        .unwrap();
    let eco = SdlSyntheticsEcosystem;
    let roots = ExternalSourceLocator::locate_roots(&eco, dir.path());
    assert_eq!(roots.len(), 1, "locate_roots must return the synthetic dep root for SDL projects");
}

#[test]
fn gl_attr_enum_constants_present() {
    let pf = synthesize_file();
    let names: Vec<&str> = pf.symbols.iter().map(|s| s.name.as_str()).collect();

    assert!(names.contains(&"SDL_GL_CONTEXT_MAJOR_VERSION"));
    assert!(names.contains(&"SDL_GL_CONTEXT_PROFILE_MASK"));
    assert!(names.contains(&"SDL_GL_DOUBLEBUFFER"));
    assert!(names.contains(&"SDL_BLENDMODE_BLEND"));
}
