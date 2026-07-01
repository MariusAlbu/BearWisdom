// =============================================================================
// ecosystem/vendored_or_generated.rs — checked-in vendor/codegen classification
//
// Some projects commit third-party code or build/codegen output straight into
// the tree instead of `.gitignore`-ing it: a vendored `node_modules/`, a
// checked-in `dist/` bundle, generated protobuf/gRPC stubs, minified bundles.
// None of it is first-party source, but nothing on disk marks it external the
// way a real dependency root does.
//
// This module supplies the classification predicate consumed during origin
// assignment in `indexer/full.rs` and by the unresolved-ref classifier in
// `query/unresolved_classify.rs`.
// =============================================================================

/// The two checked-in-noise buckets this module recognizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VendorOrGeneratedKind {
    /// Third-party code brought into the tree (`node_modules/`, `vendor/`,
    /// `third_party/`).
    Vendor,
    /// Output produced by a build or codegen step (`dist/`, `build/`,
    /// `.next/`, protobuf/gRPC stubs, minified/bundled JS, `.designer.cs`).
    Generated,
}

/// Classifies `rel_path` (project-root-relative, either path separator) as
/// vendored third-party code, generated/build output, or neither.
///
/// Path-segment patterns match a whole segment exactly — `buildSrc/`,
/// `builder/`, and a file named `distributed.rs` do NOT match the `build`/
/// `dist` segment. Filename-suffix patterns match the trailing extension of
/// the last segment only.
pub fn classify(rel_path: &str) -> Option<VendorOrGeneratedKind> {
    let p = rel_path.replace('\\', "/");
    let segments: Vec<&str> = p.split('/').collect();

    for seg in &segments {
        if matches!(*seg, "node_modules" | "vendor" | "third_party" | "third-party") {
            return Some(VendorOrGeneratedKind::Vendor);
        }
        if matches!(
            *seg,
            "dist" | "build"
                | "out"
                | ".next"
                | ".nuxt"
                | ".svelte-kit"
                | ".output"
                | "generated"
                | "__generated__"
                | "obj"
                | "bin"
                | "target"
                | ".gradle"
                | ".idea"
                | ".vscode"
        ) {
            return Some(VendorOrGeneratedKind::Generated);
        }
    }

    let leaf = segments.last().copied().unwrap_or("");
    let generated_suffix = leaf.ends_with(".g.cs")
        || leaf.ends_with(".designer.cs")
        || leaf.ends_with(".generated.cs")
        || leaf.ends_with(".generated.ts")
        || leaf.ends_with(".gen.go")
        || leaf.ends_with(".pb.go")
        || leaf.ends_with("_pb2.py")
        || leaf.ends_with("_pb2_grpc.py")
        || leaf.ends_with(".min.js")
        || leaf.ends_with(".bundle.js");
    generated_suffix.then_some(VendorOrGeneratedKind::Generated)
}

#[cfg(test)]
#[path = "vendored_or_generated_tests.rs"]
mod tests;
