//! GDScript language plugin.
//!
//! Covers `.gd` files (Godot Engine scripting language).
//!
//! What we extract:
//! - `class_name_statement` → Class (top-level class declaration)
//! - `class_definition` → Class (inner class)
//! - `function_definition` → Function / Method
//! - `signal_statement` → Event
//! - `export_variable_statement` → Property
//! - `variable_statement` / `const_statement` → Variable / Field
//! - `enum_definition` → Enum

pub mod primitives;
pub mod extract;

mod builtins;
pub(crate) mod resolve;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct GDScriptPlugin;

impl LanguagePlugin for GDScriptPlugin {
    fn id(&self) -> &str { "gdscript" }

    fn language_ids(&self) -> &[&str] { &["gdscript"] }

    fn extensions(&self) -> &[&str] { &[".gd"] }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_gdscript::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_name_statement",
            "class_definition",
            "function_definition",
            "constructor_definition",
            "signal_statement",
            "export_variable_statement",
            "variable_statement",
            "const_statement",
            "enum_definition",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call",
            "extends_statement",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        &[
            "int", "float", "bool", "String", "StringName", "NodePath",
            "Vector2", "Vector2i", "Vector3", "Vector3i", "Vector4", "Vector4i",
            "Color", "Rect2", "Rect2i", "Transform2D", "Transform3D",
            "Basis", "Quaternion", "Plane", "AABB",
            "Array", "Dictionary", "PackedByteArray", "PackedInt32Array",
            "PackedInt64Array", "PackedFloat32Array", "PackedFloat64Array",
            "PackedStringArray", "PackedVector2Array", "PackedVector3Array",
            "PackedColorArray", "Object", "Node", "RefCounted", "Resource",
            "void", "Variant", "Callable", "Signal", "RID",
        ]
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::GDScriptResolver))
    }
}
