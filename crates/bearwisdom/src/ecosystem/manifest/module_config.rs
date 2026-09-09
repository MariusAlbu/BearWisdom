//! Build-system source configuration. Text is an ingestion recipe, not identity.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetKind {
    Library,
    Executable,
    Development,
    Build,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleTarget {
    pub name: String,
    /// Relative to the package directory; custom manifest paths stay exact.
    pub path: String,
    pub kind: TargetKind,
    /// Requires a build configuration we have not evaluated.
    pub conditional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleDependency {
    pub alias: String,
    pub package: String,
    /// A path-dependency's package directory, relative to the project root.
    /// None means no source-location evidence, not permission to guess by name.
    pub root: Option<String>,
    pub renamed: bool,
    pub kind: TargetKind,
    pub conditional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModulePackage {
    /// Relative to the project root. Kept with the record when manifests union.
    pub root: String,
    pub name: String,
    pub fingerprint: String,
    pub targets: Vec<ModuleTarget>,
    pub dependencies: Vec<ModuleDependency>,
}

#[cfg(test)]
#[path = "module_config_tests.rs"]
mod tests;
