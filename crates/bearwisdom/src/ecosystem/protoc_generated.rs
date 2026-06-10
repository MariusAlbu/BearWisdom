// =============================================================================
// ecosystem/protoc_generated.rs — protoc / buf-generated output discovery
//
// `*.proto` files declare messages and services; `protoc` (or `buf`)
// generates language-specific stubs (Rust via tonic-build, Go via
// protoc-gen-go, TS via grpc-web/proto-loader, Java via protobuf-java,
// Python via grpcio-tools). Output directories are configured per
// build tool — there's no universal "where does my generated code live".
//
// **Discovery:** when any `*.proto` file is present in the project,
// probe a set of common output dirs relative to project root and to
// each .proto file's directory. Walk anything we find with the per-file
// extractor (TS/Go/Java/Python all extract their own generated stubs).
//
// **Why probe?** Build-tool-specific config parsing (`build.gradle`
// `protobuf { ... }`, `Cargo.toml` `[build-dependencies] tonic-build`,
// `buf.gen.yaml`, etc.) is per-tool work. The common-path probe is the
// 80/20 fallback — it catches the conventional layouts most projects
// use without requiring per-tool config readers.
// =============================================================================

use std::fs;
use std::path::{Path, PathBuf};

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("protoc-generated");
const ECOSYSTEM_TAG: &str = "protoc-generated";
const LANGUAGES: &[&str] = &["rust", "go", "typescript", "javascript", "java", "python"];

/// Output directories conventional protoc / buf / language plugins use.
/// Probed relative to the project root and to each `.proto` file's
/// parent directory.
const PROTOC_OUTPUT_DIRS: &[&str] = &[
    "target/generated-sources/protobuf",
    "build/generated/source/proto",
    "build/generated/source/proto/main/java",
    "build/generated-src/proto",
    "src/main/java/generated",
    "src/main/proto-gen",
    "internal/genproto",
    "pkg/proto",
    "gen/proto",
    "gen",
    "_proto",
    "proto/gen",
    "src/generated",
    "src/proto",
];

pub struct ProtocGeneratedEcosystem;

impl Ecosystem for ProtocGeneratedEcosystem {
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
        EcosystemActivation::ManifestFieldContains {
            manifest_glob: "**/*.proto",
            field_path: "",
            value: "",
        }
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_protoc_output_dirs(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_generated_tree(&dep.root)
    }
}

impl ExternalSourceLocator for ProtocGeneratedEcosystem {
    fn ecosystem(&self) -> &'static str {
        ECOSYSTEM_TAG
    }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_protoc_output_dirs(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_generated_tree(&dep.root)
    }
}

fn discover_protoc_output_dirs(project_root: &Path) -> Vec<ExternalDepRoot> {
    let mut roots = Vec::new();
    let mut proto_dirs: Vec<PathBuf> = Vec::new();
    collect_proto_dirs(project_root, &mut proto_dirs, 0);
    if proto_dirs.is_empty() {
        return roots;
    }
    // Probe project-relative output dirs.
    for relative in PROTOC_OUTPUT_DIRS {
        let abs = project_root.join(relative);
        if abs.is_dir() && roots.iter().all(|r: &ExternalDepRoot| r.root != abs) {
            roots.push(ExternalDepRoot {
                module_path: format!("protoc:{relative}"),
                version: String::new(),
                root: abs,
                ecosystem: ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: Vec::new(),
            });
        }
    }
    // Probe per-proto-file siblings + common subdirs.
    for proto_dir in &proto_dirs {
        for candidate in [proto_dir.join("gen"), proto_dir.join("generated")] {
            if candidate.is_dir() && roots.iter().all(|r| r.root != candidate) {
                roots.push(ExternalDepRoot {
                    module_path: format!("protoc:{}", candidate.display()),
                    version: String::new(),
                    root: candidate,
                    ecosystem: ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: Vec::new(),
                });
            }
        }
    }
    roots
}

fn collect_proto_dirs(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut found_proto = false;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(
                name,
                "node_modules" | ".git" | "target" | "build" | ".bearwisdom"
            ) {
                continue;
            }
            collect_proto_dirs(&path, out, depth + 1);
        } else if path.extension().and_then(|e| e.to_str()) == Some("proto") {
            found_proto = true;
        }
    }
    if found_proto && !out.iter().any(|p| p == dir) {
        out.push(dir.to_path_buf());
    }
}

fn walk_generated_tree(dir: &Path) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir(dir, &mut out, 0);
    out
}

fn walk_dir(dir: &Path, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth > 8 {
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
            if name.starts_with('.') {
                continue;
            }
            walk_dir(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let lang = if name.ends_with(".rs") {
                "rust"
            } else if name.ends_with(".go") {
                "go"
            } else if name.ends_with(".d.ts") || name.ends_with(".ts") {
                "typescript"
            } else if name.ends_with(".js") {
                "javascript"
            } else if name.ends_with(".java") {
                "java"
            } else if name.ends_with(".py") || name.ends_with(".pyi") {
                "python"
            } else {
                continue;
            };
            let display = path.to_string_lossy().replace('\\', "/");
            out.push(WalkedFile {
                relative_path: format!("ext:protoc:{display}"),
                absolute_path: path,
                language: lang,
            });
        }
    }
}
