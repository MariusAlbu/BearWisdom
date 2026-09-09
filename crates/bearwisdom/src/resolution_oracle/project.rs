//! Pinned real-project call cohorts. Unsupported compiler evidence stays visible.
use super::*;
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectBindingMode {
    Legacy,
    ConfiguredProgram,
}

#[derive(Debug, Serialize)]
pub struct ProgramSourceGap {
    pub file: FixtureFileId,
    pub reason: ProgramSourceGapKind,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramSourceGapKind {
    UnsupportedLanguage,
    UnknownScope,
    MissingGlobalCapture,
    IncompleteGlobalCapture,
}

#[derive(Debug, Deserialize)]
pub struct ProjectManifest {
    pub version: u32,
    pub compiler: serde_json::Value,
    pub root: PathBuf,
    pub config: PathBuf,
    pub selection: serde_json::Value,
    pub compiler_options: serde_json::Value,
    pub files: Vec<ProjectFile>,
    #[serde(default)]
    pub source_binding_order: Option<Vec<FixtureFileId>>,
    pub inputs: Vec<ProjectInput>,
    pub calls: Vec<ProjectCall>,
    pub diagnostics: Vec<serde_json::Value>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProjectFile {
    pub id: FixtureFileId,
    pub path: PathBuf,
    pub index_path: String,
    pub sha256: String,
    pub language: Option<String>,
    pub selected: bool,
    #[serde(default)]
    pub source_scope: Option<crate::indexer::programs::SourceScope>,
}

#[derive(Debug, Deserialize)]
pub struct ProjectInput {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
pub struct ProjectCall {
    pub site: ReferenceSite,
    pub target: Option<DeclarationSite>,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProjectReport {
    pub binding_mode: ProjectBindingMode,
    /// Source ingestion barriers only, not a claim of complete merge semantics.
    pub configured_source_gaps: Vec<ProgramSourceGap>,
    pub compiler: serde_json::Value,
    pub selection: serde_json::Value,
    pub compiler_diagnostics: usize,
    pub compiler_calls: usize,
    pub unlabelled_compiler_calls: BTreeMap<String, usize>,
    pub compiler_sites_not_extracted: usize,
    pub extractor_only_sites: usize,
    pub duplicate_extractor_emissions: usize,
    pub selected_files: usize,
    pub supplied_files: usize,
    pub elapsed_ms: u128,
    pub fresh: OracleReport,
    pub cold: OracleReport,
    pub snapshot_changes: Vec<OracleChange>,
    /// Subset of strict mismatches: identical source declaration coordinates,
    /// different syntactic kind. These are not silently promoted to correct.
    pub fresh_declaration_kind_disagreements: usize,
    pub cold_declaration_kind_disagreements: usize,
    pub gate_eligible: bool,
    pub limitations: Vec<String>,
}

impl ProjectManifest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.version, 1 | 2),
            "Unsupported real-project manifest version"
        );
        ensure!(
            self.root.is_absolute() && self.config.is_absolute(),
            "Project paths must be absolute"
        );
        ensure!(
            self.compiler["name"] == "TypeScript" && self.compiler["version"] == "5.9.3",
            "Unreviewed compiler adapter"
        );
        ensure!(
            self.selection["kind"] == "all_call_expressions"
                && self.selection["split"] == "development",
            "Unsupported population contract"
        );
        ensure!(
            !self.files.is_empty() && !self.calls.is_empty(),
            "Empty real-project population"
        );
        let mut files = HashMap::new();
        let mut paths = HashSet::new();
        let mut index_paths = HashSet::new();
        for file in &self.files {
            ensure!(
                self.version < 2 || file.source_scope.is_some(),
                "Configured source scope missing"
            );
            ensure!(
                file.id.0 > 0 && files.insert(file.id, file).is_none(),
                "Duplicate/zero manifest file ID"
            );
            ensure!(
                file.path.is_absolute()
                    && paths.insert(&file.path)
                    && index_paths.insert(&file.index_path),
                "Duplicate/invalid source path"
            );
            ensure!(
                !file.index_path.is_empty() && !file.index_path.contains('\\'),
                "Invalid index source address"
            );
            if let Some(language) = &file.language {
                ensure!(
                    matches!(
                        language.as_str(),
                        "typescript" | "tsx" | "javascript" | "jsx"
                    ),
                    "Unsupported compiler-source language"
                );
            }
            ensure!(
                !file.selected || file.language.is_some(),
                "Selected source cannot be omitted from parsing"
            );
        }
        if let Some(order) = &self.source_binding_order {
            let unique: HashSet<_> = order.iter().copied().collect();
            ensure!(
                order.len() == files.len()
                    && unique.len() == files.len()
                    && unique.iter().all(|id| files.contains_key(id)),
                "Invalid compiler source binding order"
            );
        }
        let mut sites = HashSet::new();
        for call in &self.calls {
            ensure!(
                call.site.kind == EdgeKind::Calls && sites.insert(call.site),
                "Duplicate/unsupported compiler reference site"
            );
            ensure!(
                files.get(&call.site.file).is_some_and(|f| f.selected),
                "Compiler call outside selected source population"
            );
            ensure!(matches!((&call.target, &call.reason), (Some(_), None)) || matches!((&call.target, &call.reason), (None, Some(reason)) if !reason.is_empty()), "A call needs a target OR an explicit unsupported reason; absence is not a negative label");
            if let Some(target) = call.target {
                ensure!(
                    files
                        .get(&target.file)
                        .is_some_and(|f| f.language.is_some()),
                    "Target missing from parser supply"
                );
            }
        }
        ensure!(
            self.inputs.iter().any(|i| i.path == self.config),
            "Configuration was not fingerprinted"
        );
        Ok(())
    }

    pub fn verify_inputs(&self) -> Result<()> {
        self.validate()?;
        let mut sources = HashMap::new();
        for (path, hash) in self
            .inputs
            .iter()
            .map(|i| (&i.path, &i.sha256))
            .chain(self.files.iter().map(|f| (&f.path, &f.sha256)))
        {
            let bytes = std::fs::read(path)
                .with_context(|| format!("Read oracle input {}", path.display()))?;
            ensure!(
                format!("{:x}", Sha256::digest(&bytes)) == *hash,
                "Oracle input changed: {}",
                path.display()
            );
            std::str::from_utf8(&bytes)
                .with_context(|| format!("Non-UTF-8 oracle input {}", path.display()))?;
            sources.insert(path, String::from_utf8(bytes)?);
        }
        let files: HashMap<_, _> = self.files.iter().map(|f| (f.id, f)).collect();
        for call in &self.calls {
            let text = &sources[&files[&call.site.file].path];
            let byte = call.site.byte_offset as usize;
            ensure!(
                byte < text.len() && text.is_char_boundary(byte),
                "Invalid compiler call byte address: {:?}",
                call.site
            );
            if let Some(target) = call.target {
                let text = &sources[&files[&target.file].path];
                let line = text
                    .split('\n')
                    .nth(target.line as usize)
                    .context("Target line outside source")?;
                ensure!(
                    (target.col as usize) < line.len()
                        && line.is_char_boundary(target.col as usize),
                    "Invalid compiler declaration coordinate: {target:?}"
                );
            }
        }
        Ok(())
    }
}

fn declaration_kind_disagreements(report: &OracleReport) -> usize {
    report
        .references
        .iter()
        .filter(|reference| {
            let (Some(expected), Some(ObservedBinding::Resolved(actual))) =
                (reference.expected.target, reference.actual)
            else {
                return false;
            };
            reference.verdict == Verdict::Incorrect
                && expected.kind != actual.kind
                && (expected.file, expected.line, expected.col)
                    == (actual.file, actual.line, actual.col)
        })
        .count()
}

/// Runs against an explicit pinned source supply without writing the source
/// project or an existing index. Source/configuration changes fail closed.
pub fn evaluate_manifest(path: &Path) -> Result<ProjectReport> {
    evaluate_manifest_with_mode(path, ProjectBindingMode::Legacy)
}

/// Configuration evidence is supplied only by the pinned oracle manifest;
/// this is not runtime compiler integration or ecosystem discovery.
pub fn evaluate_manifest_with_mode(path: &Path, mode: ProjectBindingMode) -> Result<ProjectReport> {
    let bytes = std::fs::read(path)?;
    let manifest: ProjectManifest = serde_json::from_slice(&bytes)?;
    manifest.validate()?;
    manifest.verify_inputs()?;
    ensure!(
        mode != ProjectBindingMode::ConfiguredProgram || manifest.version == 2,
        "Configured evaluation requires version-two source-scope evidence"
    );
    let report = runner::run(
        &manifest,
        CorpusRevision(Sha256::digest(&bytes).into()),
        mode,
    )?;
    manifest.verify_inputs()?;
    Ok(report)
}

#[path = "project_runner.rs"]
mod runner;
#[cfg(test)]
#[path = "project_tests.rs"]
mod tests;
