// =============================================================================
// gdscript/keywords.rs — GDScript primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for GDScript (Godot 4).
pub(crate) const KEYWORDS: &[&str] = &[
    // global functions
    "load", "preload", "push_error", "push_warning",
    "print", "prints", "printt", "printerr", "print_rich",
    "range", "len", "typeof", "str", "int", "float", "bool",
    "is_instance_valid", "is_inf", "is_nan", "is_zero_approx",
    "is_equal_approx",
    // math
    "abs", "sign", "clamp", "lerp", "inverse_lerp", "remap",
    "smoothstep", "step_decimals", "snapped",
    "wrap", "wrapf", "min", "max",
    "ceil", "floor", "round", "fmod", "fposmod", "posmod",
    "pow", "log", "exp", "sqrt",
    "sin", "cos", "tan", "asin", "acos", "atan", "atan2",
    "deg_to_rad", "rad_to_deg",
    // random
    "randomize", "randi", "randf", "randi_range", "randf_range", "seed",
    // misc
    "hash", "weakref", "assert", "error_string",
    // node helpers
    "get_tree", "get_node", "get_parent", "get_children", "get_child",
    "add_child", "remove_child", "queue_free", "free",
    "connect", "disconnect", "emit_signal",
    "call", "call_deferred", "set", "get",
    "has_method", "has_signal", "is_connected",
    "duplicate", "instance_from_id",
    // singletons
    "ClassDB", "Engine", "OS", "ProjectSettings", "Input", "InputMap",
    "ResourceLoader", "ResourceSaver", "Time", "Performance",
    "EditorInterface", "EditorPlugin",
    // math types
    "Vector2", "Vector2i", "Vector3", "Vector3i",
    "Vector4", "Vector4i",
    "Rect2", "Rect2i",
    "Transform2D", "Transform3D", "Basis", "Quaternion",
    "AABB", "Plane", "Projection", "Color",
    // reference types
    "StringName", "NodePath", "RID", "Callable", "Signal",
    // packed arrays
    "PackedByteArray", "PackedInt32Array", "PackedInt64Array",
    "PackedFloat32Array", "PackedFloat64Array", "PackedStringArray",
    "PackedVector2Array", "PackedVector3Array", "PackedColorArray",
    "PackedVector4Array",
    // collections
    "Array", "Dictionary",
    // core node types
    "Node", "Node2D", "Node3D", "Control", "Resource",
    "RefCounted", "Object",
    // common nodes
    "Sprite2D", "Sprite3D",
    "Camera2D", "Camera3D",
    "RigidBody2D", "RigidBody3D",
    "CharacterBody2D", "CharacterBody3D",
    "StaticBody2D", "StaticBody3D",
    "Area2D", "Area3D",
    "CollisionShape2D", "CollisionShape3D",
    "AnimationPlayer", "AnimationTree",
    "AudioStreamPlayer", "Timer",
    "SceneTree", "Viewport", "Window",
    // UI nodes
    "Label", "Button", "TextureRect", "LineEdit", "TextEdit",
    "Panel", "HBoxContainer", "VBoxContainer", "GridContainer",
    "MarginContainer", "ScrollContainer", "TabContainer",
    "ItemList", "Tree", "PopupMenu",
    "FileDialog", "AcceptDialog", "ConfirmationDialog",
    // other scene nodes
    "CanvasLayer", "ParallaxBackground", "Tween",
    "TileMap", "TileSet",
    // 3D
    "Shader", "ShaderMaterial", "Material", "Mesh", "MeshInstance3D",
    "DirectionalLight3D", "OmniLight3D", "SpotLight3D",
    "WorldEnvironment", "Environment",
    // resources
    "GDScript", "PackedScene", "Theme", "Font",
    "Texture2D", "Image", "ImageTexture", "AtlasTexture",
    "AnimatedTexture", "StreamTexture2D", "PhysicsMaterial",
    // servers
    "NavigationServer2D", "NavigationServer3D",
    "PhysicsServer2D", "PhysicsServer3D",
    "RenderingServer", "AudioServer", "DisplayServer",
    // OO keyword
    "super",
    // GDScript 3 coroutine keyword used with call syntax: yield(signal, "done").
    // Removed in GDScript 4 (replaced by `await`), but appears in Godot 3
    // projects and is a language primitive, not a user-defined symbol.
    "yield",
    // Control-flow keywords that tree-sitter occasionally surfaces as call-node
    // callees — e.g. `else: if (cond)` on one line produces a spurious `call`
    // whose callee reads as "if".
    "if", "elif", "else", "while", "for", "match", "break", "continue",
    "return", "pass", "await", "and", "or", "not", "in", "is", "as",
    "var", "const", "func", "class", "extends", "signal", "enum",
    "static", "setget", "onready", "export", "tool", "null", "true", "false",
];
