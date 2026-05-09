// =============================================================================
// ecosystem/gnat_project.rs — GNAT Project files (.gpr)
//
// GNAT Project files (`.gpr`) are the traditional pre-Alire build manifest
// for Ada projects: each declares the source directories that contribute to
// one library/executable, and `with "<path>";` clauses pull in other GPR
// projects whose source dirs are then visible to the importer.
//
// Activation: `ManifestMatch` on `*.gpr`. We do not probe-and-pray — if no
// GPR is present, no roots.
//
// What this walker resolves:
//   * Imported GPRs whose paths land OUTSIDE the project root — typically
//     installed Ada libraries pointed at via relative paths like
//     `../external/lib.gpr` or absolute paths (a real-world example: a
//     monorepo where multiple GPR-managed projects share a parent).
//   * Source dirs declared via `for Source_Dirs use (...)` — including the
//     `Var & "/path"` concatenation pattern Alire-generated GPRs commonly
//     emit. Internal Source_Dirs are already covered by the project walker;
//     external ones (resolved relative to the GPR file outside project
//     root) become ExternalDepRoots.
//
// What this walker does NOT do:
//   * Evaluate case statements / runtime-dependent variables. The walker
//     uses the simplest possible variable substitution (single-string
//     `Var := "..."` assignments) and drops anything more complex.
//   * Honor `GPR_PROJECT_PATH` recursively. Future work.
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("gnat-project");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["ada"];
const LEGACY_ECOSYSTEM_TAG: &str = "gnat-project";

pub struct GnatProjectEcosystem;

// ---------------------------------------------------------------------------
// Ecosystem trait impl
// ---------------------------------------------------------------------------

impl Ecosystem for GnatProjectEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn workspace_package_extensions(&self) -> &'static [(&'static str, &'static str)] {
        &[(".gpr", "ada")]
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        &["obj", "lib", "alire"]
    }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::ManifestMatch
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_gnat_project_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_gpr_root(dep)
    }

    fn supports_reachability(&self) -> bool { true }
    fn uses_demand_driven_parse(&self) -> bool { true }

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        build_gnat_project_symbol_index(dep_roots)
    }
}

// ---------------------------------------------------------------------------
// Legacy ExternalSourceLocator impl
// ---------------------------------------------------------------------------

impl ExternalSourceLocator for GnatProjectEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_gnat_project_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_gpr_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<GnatProjectEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(GnatProjectEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

pub fn discover_gnat_project_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    // Walk the project tree to find every *.gpr file, then for each one
    // resolve the source dirs and with-clauses. Source dirs that land
    // OUTSIDE project_root become external dep roots.
    let mut gpr_files: Vec<PathBuf> = Vec::new();
    collect_gpr_files(project_root, &mut gpr_files, 0);
    if gpr_files.is_empty() { return Vec::new() }

    let mut external_dirs: HashSet<PathBuf> = HashSet::new();

    for gpr_path in &gpr_files {
        let parsed = match parse_gpr_file(gpr_path) {
            Some(p) => p,
            None => continue,
        };

        // External `with` clauses pull in source from another GPR project.
        for with_path in &parsed.with_paths {
            let resolved = resolve_with_path(gpr_path, with_path);
            if let Some(dep_dir) = resolved {
                if !is_inside(&dep_dir, project_root) {
                    external_dirs.insert(dep_dir);
                }
            }
        }

        // External Source_Dirs: any absolute path or `..`-traversal that
        // ends up outside the project root is an external dep root.
        let gpr_dir = match gpr_path.parent() {
            Some(d) => d,
            None => continue,
        };
        for src_dir in &parsed.source_dirs {
            let resolved = resolve_source_dir(gpr_dir, src_dir);
            if let Some(dep_dir) = resolved {
                if !is_inside(&dep_dir, project_root) {
                    external_dirs.insert(dep_dir);
                }
            }
        }
    }

    let roots: Vec<ExternalDepRoot> = external_dirs
        .into_iter()
        .map(|dir| {
            let module_path = dir
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.to_string())
                .unwrap_or_else(|| "gnat-project".to_string());
            ExternalDepRoot {
                module_path,
                version: String::new(),
                root: dir,
                ecosystem: LEGACY_ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: Vec::new(),
            }
        })
        .collect();

    debug!(
        "gnat-project: {} external roots from {} GPR files",
        roots.len(),
        gpr_files.len()
    );
    roots
}

fn collect_gpr_files(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "obj" | "lib" | "alire" | ".git" | "node_modules" | "target")
                    || name.starts_with('.')
                {
                    continue;
                }
            }
            collect_gpr_files(&path, out, depth + 1);
        } else if ft.is_file() {
            if path.extension().and_then(|e| e.to_str()) == Some("gpr") {
                out.push(path);
            }
        }
    }
}

fn is_inside(candidate: &Path, root: &Path) -> bool {
    let candidate_canon = std::fs::canonicalize(candidate).unwrap_or_else(|_| candidate.to_path_buf());
    let root_canon = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    candidate_canon.starts_with(&root_canon)
}

/// Resolve a `with "..."` clause's path relative to its containing GPR.
/// Both `.gpr`-suffixed and unsuffixed forms appear in real projects.
fn resolve_with_path(gpr_path: &Path, with_path: &str) -> Option<PathBuf> {
    let gpr_dir = gpr_path.parent()?;
    let mut candidate = gpr_dir.join(with_path);
    if candidate.extension().and_then(|e| e.to_str()) != Some("gpr") {
        candidate.set_extension("gpr");
    }
    if !candidate.exists() {
        // Some GPR files use forward slashes in `with` paths even on Windows.
        candidate = gpr_dir.join(with_path.replace('\\', "/"));
        if candidate.extension().and_then(|e| e.to_str()) != Some("gpr") {
            candidate.set_extension("gpr");
        }
    }
    if candidate.exists() {
        candidate.parent().map(|p| p.to_path_buf())
    } else {
        None
    }
}

/// Resolve a Source_Dirs entry to an absolute directory.
fn resolve_source_dir(gpr_dir: &Path, src: &str) -> Option<PathBuf> {
    let trimmed = src.trim_end_matches("/**").trim_end_matches('/');
    if trimmed.is_empty() { return None }
    let resolved = gpr_dir.join(trimmed);
    if resolved.is_dir() {
        Some(resolved)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// GPR parser
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct ParsedGpr {
    pub with_paths: Vec<String>,
    pub source_dirs: Vec<String>,
    pub variables: HashMap<String, String>,
}

pub fn parse_gpr_file(path: &Path) -> Option<ParsedGpr> {
    let content = std::fs::read_to_string(path).ok()?;
    Some(parse_gpr_text(&content))
}

/// Parse a GPR file's text. Tolerant: unknown forms are skipped silently.
pub fn parse_gpr_text(content: &str) -> ParsedGpr {
    let mut parsed = ParsedGpr::default();
    let mut buf = String::new();

    // Pass 1: gather single-string variable assignments (`Var := "..."`).
    // These let us substitute into Source_Dirs concatenation in pass 2.
    for raw in content.lines() {
        let line = strip_ada_comment(raw);
        let trimmed = line.trim();
        if let Some((name, value)) = parse_simple_assignment(trimmed) {
            parsed.variables.insert(name, value);
        }
    }

    // Pass 2: collect with-clauses + walk paren-balanced Source_Dirs blocks.
    // We linearize the file by buffering until each statement terminator
    // (`;`) so multi-line lists join into a single string we can parse.
    let mut pending: Option<&str> = None; // "with" or "source_dirs"

    for raw in content.lines() {
        let line = strip_ada_comment(raw);
        buf.push_str(line);
        buf.push(' ');

        if !buf.contains(';') {
            continue;
        }

        // Process completed statements (one or more terminated by ';').
        while let Some(idx) = buf.find(';') {
            let stmt: String = buf[..idx].trim().to_owned();
            buf = buf[idx + 1..].to_string();

            if let Some(paths) = parse_with_clause(&stmt) {
                parsed.with_paths.extend(paths);
                pending = None;
                continue;
            }

            if let Some(dirs) = parse_source_dirs(&stmt, &parsed.variables) {
                parsed.source_dirs.extend(dirs);
                pending = None;
                continue;
            }
            let _ = pending;
        }
    }

    parsed
}

fn parse_simple_assignment(line: &str) -> Option<(String, String)> {
    // `Name := "value";` — the cheap case.
    let cleaned = line.trim_end_matches(';').trim();
    let (lhs, rhs) = cleaned.split_once(":=")?;
    let name = lhs.trim();
    if name.is_empty() {
        return None;
    }
    let rhs = rhs.trim();
    let value = rhs.strip_prefix('"')?.strip_suffix('"')?;
    Some((name.to_string(), value.to_string()))
}

fn parse_with_clause(stmt: &str) -> Option<Vec<String>> {
    // `with "x.gpr"` (or comma-separated: `with "a", "b";`)
    let trimmed = stmt.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("with ") && !lower.starts_with("with\t") {
        return None;
    }
    let body = &trimmed["with ".len()..];
    let mut paths = Vec::new();
    for chunk in body.split(',') {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            continue;
        }
        if let Some(unq) = chunk.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            paths.push(unq.to_string());
        }
    }
    Some(paths)
}

fn parse_source_dirs(stmt: &str, variables: &HashMap<String, String>) -> Option<Vec<String>> {
    // `for Source_Dirs use ( "a", Var & "/sub", "c/**" )`
    let lower = stmt.to_ascii_lowercase();
    let needle = "for source_dirs use";
    let idx = lower.find(needle)?;
    let after = stmt[idx + needle.len()..].trim();
    let body = after.strip_prefix('(').and_then(|s| s.rsplit_once(')')).map(|(b, _)| b)?;

    let mut out = Vec::new();
    for raw_entry in split_top_level(body, ',') {
        let entry = raw_entry.trim();
        if entry.is_empty() { continue }
        if let Some(s) = evaluate_string_expr(entry, variables) {
            out.push(s);
        }
    }
    Some(out)
}

/// Split on `delim` at depth 0, ignoring delimiters inside parens or quotes.
fn split_top_level(input: &str, delim: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut depth_paren = 0;
    let mut in_str = false;
    for c in input.chars() {
        if c == '"' {
            in_str = !in_str;
            buf.push(c);
            continue;
        }
        if !in_str {
            match c {
                '(' => { depth_paren += 1; buf.push(c); continue; }
                ')' => { depth_paren -= 1; buf.push(c); continue; }
                _ => {}
            }
            if c == delim && depth_paren == 0 {
                out.push(std::mem::take(&mut buf));
                continue;
            }
        }
        buf.push(c);
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

/// Evaluate a tiny GPR string expression: literal strings + `&`
/// concatenation with single-string variables. Returns None for anything
/// more complex (we drop the entry rather than guess).
fn evaluate_string_expr(expr: &str, variables: &HashMap<String, String>) -> Option<String> {
    let mut out = String::new();
    for piece in expr.split('&') {
        let piece = piece.trim();
        if piece.is_empty() { return None }
        if let Some(unq) = piece.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            out.push_str(unq);
            continue;
        }
        // Bare identifier — look up in vars.
        let value = variables.get(piece)?;
        out.push_str(value);
    }
    Some(out)
}

fn strip_ada_comment(line: &str) -> &str {
    match line.find("--") {
        Some(idx) => &line[..idx],
        None => line,
    }
}

// ---------------------------------------------------------------------------
// Walker
// ---------------------------------------------------------------------------

fn walk_gpr_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_dir_bounded(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "obj" | "lib" | "alire" | "tests" | "test" | "examples" | ".git")
                    || name.starts_with('.')
                {
                    continue;
                }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if ft.is_file() {
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else { continue };
            if ext != "ads" && ext != "adb" { continue }
            let rel = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:gnat-project:{}/{}", dep.module_path, rel);
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language: "ada",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol index
// ---------------------------------------------------------------------------

pub(crate) fn build_gnat_project_symbol_index(
    dep_roots: &[ExternalDepRoot],
) -> SymbolLocationIndex {
    let mut index = SymbolLocationIndex::new();
    for dep in dep_roots {
        let mut files = Vec::new();
        collect_ads_files(&dep.root, &mut files, 0);
        for path in files {
            if let Some(qname) = scan_package_decl(&path) {
                let key = qname.to_ascii_lowercase();
                index.insert(dep.module_path.clone(), key.clone(), path.clone());
                if key != qname {
                    index.insert(dep.module_path.clone(), qname, path);
                }
            }
        }
    }
    index
}

fn collect_ads_files(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "obj" | "lib" | "alire" | "tests" | "test" | "examples" | ".git")
                    || name.starts_with('.')
                {
                    continue;
                }
            }
            collect_ads_files(&path, out, depth + 1);
        } else if ft.is_file() {
            if path.extension().and_then(|e| e.to_str()) == Some("ads") {
                out.push(path);
            }
        }
    }
}

fn scan_package_decl(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for raw in content.lines() {
        let line = strip_ada_comment(raw).trim();
        if line.is_empty() { continue }
        let mut tail = line;
        for prefix in ["private ", "generic "] {
            if let Some(rest) = tail.strip_prefix(prefix) {
                tail = rest.trim_start();
            }
        }
        let after_kw = if let Some(r) = tail.strip_prefix("package body ") {
            r
        } else if let Some(r) = tail.strip_prefix("package ") {
            r
        } else {
            continue;
        };
        let qname: String = after_kw
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '.' || *c == '_')
            .collect();
        if qname.is_empty() { continue }
        return Some(qname);
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "gnat_project_tests.rs"]
mod tests;
