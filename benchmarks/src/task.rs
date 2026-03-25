// =============================================================================
// task.rs  —  BenchmarkTask data model and TaskSet persistence
// =============================================================================

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;

// ---------------------------------------------------------------------------
// Category
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskCategory {
    ImpactAnalysis,
    CallHierarchy,
    ArchitectureOverview,
    CrossFileReferences,
    ConceptDiscovery,
    SymbolSearch,
}

impl TaskCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ImpactAnalysis => "ImpactAnalysis",
            Self::CallHierarchy => "CallHierarchy",
            Self::ArchitectureOverview => "ArchitectureOverview",
            Self::CrossFileReferences => "CrossFileReferences",
            Self::ConceptDiscovery => "ConceptDiscovery",
            Self::SymbolSearch => "SymbolSearch",
        }
    }
}

// ---------------------------------------------------------------------------
// Ground truth
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundTruth {
    /// Qualified symbol names that the answer must mention.
    pub expected_items: Vec<String>,
    /// File paths that must be cited.
    pub expected_files: Vec<String>,
    /// Arbitrary extra metadata (category-specific).
    pub metadata: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Single task
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkTask {
    pub id: String,
    pub category: TaskCategory,
    /// Natural-language question sent to Claude.
    pub question: String,
    /// The symbol targeted by this task (None for overview tasks).
    pub target_symbol: Option<String>,
    pub ground_truth: GroundTruth,
    pub generated_at: DateTime<Utc>,
    pub project_path: String,
}

// ---------------------------------------------------------------------------
// Task set
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSet {
    pub tasks: Vec<BenchmarkTask>,
    pub project_path: String,
    pub generated_at: DateTime<Utc>,
}

impl TaskSet {
    pub fn new(tasks: Vec<BenchmarkTask>, project_path: String) -> Self {
        Self {
            tasks,
            project_path,
            generated_at: Utc::now(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)
            .context("Failed to serialise TaskSet")?;
        std::fs::write(path, json)
            .with_context(|| format!("Failed to write task set to {}", path.display()))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read task set from {}", path.display()))?;
        serde_json::from_str(&data)
            .with_context(|| format!("Failed to parse task set from {}", path.display()))
    }
}
