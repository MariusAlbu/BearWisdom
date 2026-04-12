// =============================================================================
// gdscript/externals.rs — GDScript runtime globals and engine singletons
// =============================================================================

/// GDScript engine globals and singletons that are always external.
///
/// These are identifiers that appear in GDScript code but are never defined
/// in project source — they are injected by the Godot engine at runtime.
pub(crate) const EXTERNALS: &[&str] = &[
    // Engine singletons accessible globally
    "Input",
    "InputMap",
    "OS",
    "Engine",
    "ProjectSettings",
    "ResourceLoader",
    "ResourceSaver",
    "Performance",
    "ClassDB",
    "Time",
    "RenderingServer",
    "PhysicsServer2D",
    "PhysicsServer3D",
    "AudioServer",
    "DisplayServer",
    "NavigationServer2D",
    "NavigationServer3D",
    "XRServer",
    "CameraServer",
    "VisualScriptEditor",
    "EditorInterface",
    "EditorPlugin",
    // Common autoloaded singleton names (project-conventional but engine-injected)
    "GameManager",
    "SignalBus",
    "EventBus",
];

