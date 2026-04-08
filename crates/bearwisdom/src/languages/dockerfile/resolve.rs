// =============================================================================
// languages/dockerfile/resolve.rs — Dockerfile resolution rules
//
// Dockerfile references:
//
//   FROM ubuntu:22.04          → base image reference (external)
//   FROM builder AS final      → multi-stage alias declaration
//   COPY --from=builder /src . → reference to a build stage named "builder"
//
// Resolution strategy:
//   1. `COPY --from=<stage>` references resolve against stage names defined by
//      `FROM ... AS <stage>` in the same file.
//   2. `FROM <image>` base images are external (Docker Hub / registry).
//
// External namespace: `"docker"` for base images from registries.
// =============================================================================

use crate::indexer::resolve::engine::{
    FileContext, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct DockerfileResolver;

impl LanguageResolver for DockerfileResolver {
    fn language_ids(&self) -> &[&str] {
        &["dockerfile"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        // No imports in Dockerfiles; stage names are just symbols in the same file.
        FileContext {
            file_path: file.path.clone(),
            language: "dockerfile".to_string(),
            imports: Vec::new(),
            file_namespace: None,
        }
    }

    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        // Imports (FROM base images) are always external.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // COPY --from=<stage> — resolve against stage symbols in the same file.
        // Stage names are emitted as symbols from `from_instruction` with an
        // alias (AS clause).
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name.eq_ignore_ascii_case(target) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "dockerfile_stage_ref",
                });
            }
        }

        None
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        // Base image references in FROM instructions are always external.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return Some("docker".to_string());
        }

        // Scratch is a special Docker pseudo-image.
        if ref_ctx.extracted_ref.target_name.eq_ignore_ascii_case("scratch") {
            return Some("docker".to_string());
        }

        None
    }
}
