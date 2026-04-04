//! Clojure language plugin.
//!
//! Grammar: tree-sitter-clojure 0.1
//! Everything is a `list_lit`. Pattern-match on the first symbol:
//! - `defn` / `defn-` → Function
//! - `defprotocol` → Interface
//! - `defrecord` / `deftype` → Struct
//! - `ns` → Namespace + import edges
//! - `def` → Variable
//! - `defmacro` → Function (macro)

pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct ClojurePlugin;

impl LanguagePlugin for ClojurePlugin {
    fn id(&self) -> &str { "clojure" }

    fn language_ids(&self) -> &[&str] { &["clojure"] }

    fn extensions(&self) -> &[&str] { &[".clj", ".cljs", ".cljc", ".edn"] }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_clojure::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        // Clojure's grammar uses list_lit for all declarations
        &["list_lit"]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &["list_lit", "sym_lit"]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            "String", "Integer", "Long", "Double", "Float", "Boolean",
            "Character", "Byte", "Short", "Number",
            "Object", "Class", "Symbol", "Keyword",
            "List", "Vector", "Map", "Set", "Seq", "Fn",
        ]
    }
}
