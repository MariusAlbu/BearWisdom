// =============================================================================
// ada/resolve.rs — Ada resolution rules
//
// Scope rules for Ada:
//
//   1. Scope chain walk: innermost subprogram/block → package → library.
//   2. Same-file resolution: all declarations in the same compilation unit.
//   3. Import-based resolution:
//        `with Package_Name;` → makes Package_Name visible (dot-qualified)
//        `use Package_Name;`  → brings all exported names into direct scope
//   4. Spec-to-body context inheritance: a `.adb` body inherits all context
//        clauses declared in its sibling `.ads` spec. The resolver driver
//        merges the spec's imports into the body's FileContext before any
//        reference is resolved.
//
// The extractor emits EdgeKind::Imports with:
//   target_name = package name (both `with` and `use` clauses)
//   module      = None (Ada imports are always the package name itself)
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile, SymbolKind};

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;

#[cfg(test)]
pub(super) fn _test_probe_package_of_type(
    target: &str,
    edge_kind: EdgeKind,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    probe_package_of_type(target, edge_kind, lookup)
}

/// Ada language resolver.
pub struct AdaResolver;

impl LanguageResolver for AdaResolver {
    fn language_ids(&self) -> &[&str] {
        &["ada"]
    }

    /// For an Ada body file (`.adb`), return the sibling spec file (`.ads`)
    /// so the resolve driver can merge the spec's context clauses — `with` /
    /// `use` clauses and package renames — into the body's FileContext before
    /// any reference is resolved. Ada's visibility rule requires this: a body
    /// inherits every context clause declared in its specification.
    fn companion_file_for_imports(&self, file_path: &str) -> Option<String> {
        spec_for_body(file_path)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Capture the outermost package/namespace qname (e.g. `Alr.Commands.Run`).
        // Ada body/spec files declare exactly one top-level package; its qualified
        // name is the compilation unit's identifier in dot notation.
        let file_namespace = file
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Namespace && s.parent_index.is_none())
            .map(|s| s.qualified_name.clone());

        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            // Both `with` and `use` clauses produce Imports edges. The
            // package_renaming_declaration handler in the extractor sets
            // `module` to the renamed-target package (e.g. for
            // `package Trace renames Simple_Logging;` the ref carries
            // target_name="Trace" and module=Some("Simple_Logging"));
            // when present, that's the actual module to look up.
            let module_path = r
                .module
                .clone()
                .unwrap_or_else(|| r.target_name.clone());
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module_path),
                alias: None,
                is_wildcard: true, // Ada `use` makes all names visible
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "ada".to_string(),
            imports,
            file_namespace,
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

        if edge_kind == EdgeKind::Imports {
            return None;
        }

        let target_lower = target.to_lowercase();
        let simple = target.split('.').last().unwrap_or(target);
        let simple_lower = simple.to_lowercase();

        // Ada identifiers are case-insensitive; check same-file with case folding
        // before delegating to the common resolver (which uses exact matching).
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name.to_lowercase() == simple_lower
                && predicates::kind_compatible(edge_kind, &sym.kind)
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "ada_same_file_ci",
                    resolved_yield_type: None,
                });
            }
        }

        // Ada language-defined primitives on modular types (`Shift_Right`,
        // `Shift_Left`, `Rotate_Left`, `Rotate_Right`,
        // `Shift_Right_Arithmetic`) are implicitly visible wherever a
        // modular type is in scope — strict resolution would require
        // tracking what types are reachable through imports. We approximate
        // by taking any `Interfaces.<name>` match when the bare target is
        // one of these well-known operators. The list is fixed by Ada RM
        // 13.7, not a library API surface.
        if !target.contains('.') && is_ada_modular_primitive(simple) {
            for sym in lookup.by_name(simple) {
                if sym.qualified_name.starts_with("Interfaces.")
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.85,
                        strategy: "ada_modular_primitive",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Bare-name lookup against EVERY imported package (with or use).
        //
        // `use Ada.Text_IO;` brings the package's exports into bare scope —
        // the wildcard case. But Ada also implicitly imports primitive
        // operations of types declared by `with`-only imports: when a file
        // does `with Interfaces;` and uses an `Interfaces.Unsigned_16`
        // value, `Shift_Right(X, N)` is automatically callable bare because
        // it's a primitive on the modular type. The compiler resolves these
        // via type-driven rules; we approximate by checking every imported
        // package for a member whose name matches.
        //
        // This bypasses the engine's `file_stem_matches` heuristic, which
        // breaks for GNAT's krunched filenames (`a-textio.ads` vs module
        // last-segment `text_io`), and unlocks `with`-only primitives.
        if !target.contains('.') {
            for import in &file_ctx.imports {
                let Some(module_path) = &import.module_path else {
                    continue;
                };
                for sym in lookup.members_of(module_path) {
                    if sym.name.to_lowercase() == simple_lower
                        && predicates::kind_compatible(edge_kind, &sym.kind)
                    {
                        let strategy = if import.is_wildcard {
                            "ada_use_clause"
                        } else {
                            "ada_with_primitive"
                        };
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: if import.is_wildcard { 0.95 } else { 0.85 },
                            strategy,
                            resolved_yield_type: None,
                        });
                    }
                }
            }
        }

        // Parent-package implicit visibility. A child package body (`Alr.Commands.Run`)
        // implicitly sees all declarations from every ancestor package (`Alr.Commands`,
        // `Alr`). Walk the ancestor prefixes of the file's own package qname and
        // check `members_of(ancestor)` for a bare-name match, applying any rename
        // substitution encoded in a member's signature (`renames <target>`).
        if !target.contains('.') {
            if let Some(own_pkg) = &file_ctx.file_namespace {
                let parts: Vec<&str> = own_pkg.split('.').collect();
                // Walk from immediate parent up to the root (skip the full qname
                // itself — that's same-file scope already handled above).
                for depth in (1..parts.len()).rev() {
                    let ancestor = parts[..depth].join(".");
                    for member in lookup.members_of(&ancestor) {
                        if member.name.to_lowercase() == simple_lower
                            && predicates::kind_compatible(edge_kind, &member.kind)
                        {
                            return Some(Resolution {
                                target_symbol_id: member.id,
                                confidence: 0.9,
                                strategy: "ada_parent_pkg_visibility",
                                resolved_yield_type: None,
                            });
                        }
                        // Apply rename: `package Trace renames Simple_Logging;`
                        // emitted as member `Trace` with signature `renames Simple_Logging`.
                        // If the bare target matches the renamed alias, rewrite and probe.
                        if let Some(sig) = &member.signature {
                            if let Some(rename_target) = sig.strip_prefix("renames ") {
                                if member.name.to_lowercase() == simple_lower {
                                    for sym in lookup.members_of(rename_target) {
                                        if sym.name.to_lowercase() == simple_lower
                                            && predicates::kind_compatible(edge_kind, &sym.kind)
                                        {
                                            return Some(Resolution {
                                                target_symbol_id: sym.id,
                                                confidence: 0.88,
                                                strategy: "ada_parent_pkg_rename",
                                                resolved_yield_type: None,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Alias substitution. Two paths feed into the same lookup machinery:
        //
        //  1. File-local alias: `package ASU renames Ada.Strings.Unbounded;`
        //     declared in this file produces an Imports ref with
        //     target_name="ASU" + module=Some("Ada.Strings.Unbounded"). When
        //     a call writes `ASU.To_String(...)`, replace the leading "ASU"
        //     with the rename target and probe the canonical qname.
        //
        //  2. Cross-file alias visible via `use`: `package SP.Strings is
        //     package ASU renames Ada.Strings.Unbounded; end SP.Strings;`
        //     emits a Namespace symbol `SP.Strings.ASU` with signature
        //     `"renames Ada.Strings.Unbounded"`. Files that `use SP.Strings;`
        //     can write `ASU.To_String` bare; the leading ASU is found via
        //     `members_of("SP.Strings")` whose signature reveals the rename
        //     target. Substitute and retry.
        if target.contains('.') {
            let leading = target.split('.').next().unwrap_or("");
            let leading_lower = leading.to_lowercase();
            let suffix = &target[leading.len()..]; // includes leading dot

            // Path 1: file-local rename (Imports edge).
            for import in &file_ctx.imports {
                if import.imported_name != leading {
                    continue;
                }
                let Some(module_path) = &import.module_path else {
                    continue;
                };
                if module_path == leading {
                    continue;
                }
                let rewritten = format!("{module_path}{suffix}");
                if let Some(res) = probe_dotted_qname(&rewritten, edge_kind, lookup) {
                    return Some(res);
                }
            }

            // Path 2: cross-file rename visible through a use'd package.
            for import in &file_ctx.imports {
                if !import.is_wildcard {
                    continue;
                }
                let Some(module_path) = &import.module_path else {
                    continue;
                };
                for member in lookup.members_of(module_path) {
                    if member.name.to_lowercase() != leading_lower {
                        continue;
                    }
                    let Some(sig) = &member.signature else { continue };
                    let Some(rename_target) = sig.strip_prefix("renames ") else { continue };
                    let rewritten = format!("{rename_target}{suffix}");
                    if let Some(res) = probe_dotted_qname(&rewritten, edge_kind, lookup) {
                        return Some(res);
                    }
                }
            }
        }

        // Variable-type dispatch: `Result.Append(...)` where `Result` is a
        // local variable typed `Vector`. The extractor emits each
        // `object_declaration` / `parameter_specification` as a Variable
        // symbol with `signature = "type: T"`. Look up the leading segment
        // in the file's variables (case-insensitively), parse the encoded
        // type, and retry the lookup with the type's qualified name.
        //
        // Probe order per resolved type qname (`T`):
        //   a. `members_of(T)` — direct type-level lookup (Ada record member).
        //   b. `members_of(package_of(T))` — package-level lookup; Ada methods
        //      live at package scope, not nested under the type qname
        //      (e.g., `Ada.Containers.Vectors.Append`, not `.Vector.Append`).
        //   c. Chase through one level of generic instantiation, then repeat
        //      probes (a) and (b) on the generic's qname.
        if target.contains('.') {
            let leading = target.split('.').next().unwrap_or("");
            let leading_lower = leading.to_lowercase();
            let suffix = &target[leading.len()..];
            for sym in lookup.in_file(&file_ctx.file_path) {
                if sym.kind != "variable" {
                    continue;
                }
                if sym.name.to_lowercase() != leading_lower {
                    continue;
                }
                let Some(sig) = &sym.signature else { continue };
                let Some(ty) = sig.strip_prefix("type: ") else { continue };

                // 1. Bare-type retry — type-level and package-level.
                let rewritten = format!("{ty}{suffix}");
                if let Some(res) = probe_dotted_qname(&rewritten, edge_kind, lookup) {
                    return Some(res);
                }
                if let Some(res) = probe_package_of_type(&rewritten, edge_kind, lookup) {
                    return Some(res);
                }

                // 2. Resolve bare type to its fully-qualified form.
                let ty_leaf = ty.split('.').next_back().unwrap_or(ty);
                for ty_sym in lookup.types_by_name(ty_leaf) {
                    let qualified = format!("{}{suffix}", ty_sym.qualified_name);
                    if let Some(res) = probe_dotted_qname(&qualified, edge_kind, lookup) {
                        return Some(res);
                    }
                    if let Some(res) = probe_package_of_type(&qualified, edge_kind, lookup) {
                        return Some(res);
                    }
                    // Chase through instantiations on the qualified form and
                    // probe both type-level and package-level on the result.
                    if let Some(chained) = chase_instantiation(&qualified, lookup) {
                        if let Some(res) = probe_dotted_qname(&chained, edge_kind, lookup) {
                            return Some(res);
                        }
                        if let Some(res) = probe_package_of_type(&chained, edge_kind, lookup) {
                            return Some(res);
                        }
                    }
                }

                // 3. Chase one level of generic instantiation on the bare form,
                // then probe type-level and package-level on the rewritten qname.
                if let Some(chained) = chase_instantiation(&rewritten, lookup) {
                    if let Some(res) = probe_dotted_qname(&chained, edge_kind, lookup) {
                        return Some(res);
                    }
                    if let Some(res) = probe_package_of_type(&chained, edge_kind, lookup) {
                        return Some(res);
                    }
                }
            }
        }

        // Dotted target with leading segment matching a use'd package: try
        // the full qname, then walk back through dotted segments. Handles
        // `Ada.Text_IO.Put_Line` written explicitly even when `use Ada;` is
        // active.
        if target.contains('.') {
            // Try direct qname lookup case-insensitively by walking
            // members_of for each successively-shorter parent prefix.
            let parts: Vec<&str> = target.split('.').collect();
            for split in (1..parts.len()).rev() {
                let parent = parts[..split].join(".");
                let leaf = parts[split..].join(".");
                let leaf_lower = leaf.to_lowercase();
                for sym in lookup.members_of(&parent) {
                    if sym.qualified_name
                        .rsplit_once('.')
                        .map(|(_, n)| n)
                        .unwrap_or(&sym.qualified_name)
                        .to_lowercase()
                        == leaf_lower
                        && predicates::kind_compatible(edge_kind, &sym.kind)
                    {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.95,
                            strategy: "ada_qualified_ci",
                            resolved_yield_type: None,
                        });
                    }
                }
            }
        }

        let _ = target_lower;
        engine::resolve_common("ada", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Ada standard library imports are classified by their top-level package name.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let root = target.split('.').next().unwrap_or(target);
            if matches!(root, "Ada" | "System" | "Interfaces" | "GNAT" | "Standard") {
                return Some(root.to_string());
            }
            // Non-stdlib imports: fall through to common handler.
        }

        // Bare names are classified by the engine's keywords() set
        // populated from ada/keywords.rs. The Ada.* / System.* / GNAT.*
        // import-classification above handles the namespace cases.
        let _ = (file_ctx, ref_ctx, project_ctx);
        None
    }
}

/// True iff the name is one of Ada's language-defined modular-type
/// primitives (RM 13.7). These are implicitly visible wherever a
/// modular type is in scope, and BW resolves them generously to
/// `Interfaces.<name>` when an `Interfaces` symbol of that name exists.
fn is_ada_modular_primitive(name: &str) -> bool {
    matches!(
        name,
        "Shift_Right"
            | "Shift_Left"
            | "Rotate_Right"
            | "Rotate_Left"
            | "Shift_Right_Arithmetic"
    )
}

/// Walk a dotted qname looking for any prefix that corresponds to a
/// generic-instantiation symbol (`signature = "instantiates X"`). When
/// found, replace that prefix with the generic's qname so the suffix
/// can resolve against the generic's exported members.
///
/// Example: `String_Vectors.Vector.Append`
///   * `String_Vectors` is a Namespace symbol with
///     `signature = "instantiates Ada.Containers.Vectors"`
///   * Returns `"Ada.Containers.Vectors.Vector.Append"`.
fn chase_instantiation(target: &str, lookup: &dyn SymbolLookup) -> Option<String> {
    let parts: Vec<&str> = target.split('.').collect();
    for split in 1..=parts.len() {
        let prefix = parts[..split].join(".");
        let suffix = if split == parts.len() {
            String::new()
        } else {
            format!(".{}", parts[split..].join("."))
        };
        if let Some(sym) = lookup.by_qualified_name(&prefix) {
            if let Some(sig) = &sym.signature {
                if let Some(generic) = sig.strip_prefix("instantiates ") {
                    return Some(format!("{generic}{suffix}"));
                }
            }
        }
        // Also try by simple name if the qname lookup fails — covers
        // bare-name instantiations like `package Foo is new Bar(...)`.
        if split == 1 {
            for sym in lookup.by_name(&prefix) {
                if let Some(sig) = &sym.signature {
                    if let Some(generic) = sig.strip_prefix("instantiates ") {
                        return Some(format!("{generic}{suffix}"));
                    }
                }
            }
        }
    }
    None
}

/// Given an Ada body path (`foo/bar.adb`), return the sibling spec path
/// (`foo/bar.ads`). Returns `None` for any file that isn't a `.adb` body.
///
/// The caller is responsible for checking whether the returned path exists in
/// the index; if it isn't present (e.g. spec is external-only or not yet
/// indexed), the driver silently skips the merge.
pub(crate) fn spec_for_body(file_path: &str) -> Option<String> {
    let normalized = file_path.replace('\\', "/");
    if normalized.ends_with(".adb") {
        let stem = &normalized[..normalized.len() - 4];
        Some(format!("{stem}.ads"))
    } else {
        None
    }
}

/// Probe the *package* that owns a type when a member call couldn't be found
/// under the type's own qname.
///
/// Ada subprograms for a type live at package scope, not nested under the
/// type's qname in the index. Given `Pkg.A.B.Type.Method`, the symbol is
/// most likely `Pkg.A.B.Method` — i.e., stripping the penultimate segment
/// (the type name) and probing `members_of("Pkg.A.B")`.
///
/// Returns `None` when the target has fewer than three segments (no package
/// component above the type) or when no match is found.
fn probe_package_of_type(
    target: &str,
    edge_kind: EdgeKind,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let parts: Vec<&str> = target.split('.').collect();
    // Need at least: package + type + method (3 segments).
    if parts.len() < 3 {
        return None;
    }
    let method = *parts.last().unwrap();
    let method_lower = method.to_lowercase();
    // Drop the type segment (second-to-last); everything before it is the package.
    let pkg = parts[..parts.len() - 2].join(".");
    for sym in lookup.members_of(&pkg) {
        if sym
            .qualified_name
            .rsplit_once('.')
            .map(|(_, n)| n)
            .unwrap_or(&sym.qualified_name)
            .to_lowercase()
            == method_lower
            && predicates::kind_compatible(edge_kind, &sym.kind)
        {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.88,
                strategy: "ada_pkg_of_type",
                resolved_yield_type: None,
            });
        }
    }
    None
}

/// Walk a dotted target back through its parents, probing
/// `members_of(parent)` for a leaf whose name matches case-insensitively
/// and whose kind is compatible with the edge. Returns the first hit.
/// Used by both file-local and cross-file alias substitution.
fn probe_dotted_qname(
    target: &str,
    edge_kind: EdgeKind,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let parts: Vec<&str> = target.split('.').collect();
    for split in (1..parts.len()).rev() {
        let parent = parts[..split].join(".");
        let leaf = parts[split..].join(".");
        let leaf_lower = leaf.to_lowercase();
        for sym in lookup.members_of(&parent) {
            let sym_leaf = sym
                .qualified_name
                .rsplit_once('.')
                .map(|(_, n)| n)
                .unwrap_or(&sym.qualified_name)
                .to_lowercase();
            if sym_leaf == leaf_lower
                && predicates::kind_compatible(edge_kind, &sym.kind)
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.92,
                    strategy: "ada_alias_substitution",
                    resolved_yield_type: None,
                });
            }
        }
    }
    None
}
