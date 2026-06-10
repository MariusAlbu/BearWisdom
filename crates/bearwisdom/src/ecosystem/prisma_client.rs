// =============================================================================
// ecosystem/prisma_client.rs — Prisma generated TypeScript client discovery
//
// Prisma is a TS ORM. The user declares `schema.prisma` listing models;
// `prisma generate` emits a typed client to a target directory (default
// `node_modules/.prisma/client/`). The generated `index.d.ts` declares
// `PrismaClient` plus per-model query types (`User`, `findUnique`,
// `findMany`, etc.) — names that show up across every TS codebase using
// Prisma but never live in the project source.
//
// **Discovery:** activates when any `schema.prisma` file is present in the
// project. The schema's `generator client { ... }` block names the
// output dir; we read it cheaply (line-by-line, no full PEG parser).
// Default path is `node_modules/.prisma/client/` relative to the schema
// dir.
//
// **No synthesis.** The generated `.d.ts` files contain real symbol
// declarations the TS extractor already parses. We just point the walker
// at the output dir as an external root.
// =============================================================================

use std::fs;
use std::path::{Path, PathBuf};

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("prisma-client");
const ECOSYSTEM_TAG: &str = "prisma-client";
const LANGUAGES: &[&str] = &["typescript", "javascript"];

pub struct PrismaClientEcosystem;

impl Ecosystem for PrismaClientEcosystem {
    fn id(&self) -> EcosystemId {
        ID
    }
    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Package
    }
    fn languages(&self) -> &'static [&'static str] {
        LANGUAGES
    }

    fn activation(&self) -> EcosystemActivation {
        // Project has a `schema.prisma`. Activation rule: file pattern in
        // the project tree, not language presence — many TS projects don't
        // use Prisma. The empty `field_path` + empty `value` reduces this
        // to "file exists" without inspecting fields.
        EcosystemActivation::ManifestFieldContains {
            manifest_glob: "**/schema.prisma",
            field_path: "",
            value: "",
        }
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_prisma_client_dirs(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_ts_tree(&dep.root)
    }
}

impl ExternalSourceLocator for PrismaClientEcosystem {
    fn ecosystem(&self) -> &'static str {
        ECOSYSTEM_TAG
    }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_prisma_client_dirs(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_ts_tree(&dep.root)
    }
}

/// Walk the project tree looking for `schema.prisma` files (depth-limited,
/// skips `node_modules`/`.git`/`build`). For each schema, parse its
/// `generator client { output = "..." }` block to learn the output dir,
/// fall back to `<schema_dir>/../../node_modules/.prisma/client` (the
/// `@prisma/client` package's default re-export point).
fn discover_prisma_client_dirs(project_root: &Path) -> Vec<ExternalDepRoot> {
    let mut roots = Vec::new();
    let mut schemas = Vec::new();
    collect_schema_files(project_root, &mut schemas, 0);
    for schema in &schemas {
        for out_dir in client_output_dirs(schema, project_root) {
            if !out_dir.is_dir() {
                continue;
            }
            // Avoid duplicates (multiple schemas often emit to the same dir).
            if roots.iter().any(|r: &ExternalDepRoot| r.root == out_dir) {
                continue;
            }
            roots.push(ExternalDepRoot {
                module_path: "@prisma/client".to_string(),
                version: String::new(),
                root: out_dir,
                ecosystem: ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: Vec::new(),
            });
        }
    }
    roots
}

fn collect_schema_files(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(
                name,
                "node_modules" | ".git" | "target" | "build" | "dist" | ".bearwisdom"
            ) {
                continue;
            }
            collect_schema_files(&path, out, depth + 1);
        } else if path.file_name().and_then(|n| n.to_str()) == Some("schema.prisma") {
            out.push(path);
        }
    }
}

/// Yield candidate output dirs for the client(s) declared in `schema`:
///   1. Each `generator client { output = "path" }` block (path is
///      schema-relative or absolute).
///   2. Default Prisma output — `<project_root>/node_modules/.prisma/client`.
fn client_output_dirs(schema: &Path, project_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(content) = fs::read_to_string(schema) {
        for path in parse_generator_outputs(&content) {
            let abs = if Path::new(&path).is_absolute() {
                PathBuf::from(path)
            } else {
                schema.parent().unwrap_or(project_root).join(path)
            };
            out.push(abs);
        }
    }
    // Default location — `@prisma/client` package re-exports from here.
    out.push(
        project_root
            .join("node_modules")
            .join(".prisma")
            .join("client"),
    );
    out
}

/// Cheap line-by-line scan of `schema.prisma` for `generator { output = "..." }`
/// blocks. Pulls every `output = "value"` line within a `generator ... { }`
/// block. Avoids depending on a full Prisma schema parser.
fn parse_generator_outputs(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_generator = false;
    let mut depth: i32 = 0;
    for line in content.lines() {
        let trimmed = line.trim();
        if !in_generator {
            if trimmed.starts_with("generator ") && trimmed.contains('{') {
                in_generator = true;
                depth = trimmed.matches('{').count() as i32 - trimmed.matches('}').count() as i32;
            }
            continue;
        }
        depth += trimmed.matches('{').count() as i32;
        depth -= trimmed.matches('}').count() as i32;
        if let Some(value) = trimmed
            .strip_prefix("output")
            .and_then(|rest| rest.trim_start().strip_prefix('='))
        {
            let v = value.trim();
            let v = v.trim_start_matches('"').trim_end_matches('"');
            if !v.is_empty() {
                out.push(v.to_string());
            }
        }
        if depth <= 0 {
            in_generator = false;
        }
    }
    out
}

fn walk_ts_tree(dir: &Path) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir_ts(dir, &mut out, 0);
    out
}

fn walk_dir_ts(dir: &Path, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            walk_dir_ts(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let lang = if name.ends_with(".d.ts") || name.ends_with(".ts") {
                "typescript"
            } else if name.ends_with(".js") {
                "javascript"
            } else {
                continue;
            };
            let display = path.to_string_lossy().replace('\\', "/");
            out.push(WalkedFile {
                relative_path: format!("ext:prisma:{display}"),
                absolute_path: path,
                language: lang,
            });
        }
    }
}

#[cfg(test)]
#[path = "prisma_client_tests.rs"]
mod tests;
