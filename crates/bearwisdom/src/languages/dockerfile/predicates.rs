// =============================================================================
// dockerfile/predicates.rs — Dockerfile builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls | EdgeKind::TypeRef => matches!(sym_kind, "class" | "variable"),
        _ => true,
    }
}

/// Dockerfile instructions are syntax keywords — never project symbols.
/// Returns true when the name is a Dockerfile instruction keyword.
pub(super) fn is_dockerfile_builtin(name: &str) -> bool {
    matches!(
        name.to_uppercase().as_str(),
        "FROM"
            | "RUN"
            | "CMD"
            | "ENTRYPOINT"
            | "COPY"
            | "ADD"
            | "ENV"
            | "ARG"
            | "EXPOSE"
            | "VOLUME"
            | "WORKDIR"
            | "USER"
            | "LABEL"
            | "STOPSIGNAL"
            | "HEALTHCHECK"
            | "SHELL"
            | "ONBUILD"
            | "MAINTAINER"
    )
}
