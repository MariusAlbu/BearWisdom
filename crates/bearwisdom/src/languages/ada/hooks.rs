// =============================================================================
// languages/ada/hooks.rs — AdaHooks impl plus the concrete AdaResolver.
//
// AdaResolver handles the Ada-specific resolution shapes the generic ladder
// cannot express: bare-name across every imported package (with or use,
// distinguishing `ada_with_primitive` from `ada_use_clause`), own-package +
// ancestor-package visibility with rename-clause expansion, three alias-
// substitution paths (file-local rename, cross-file rename through use,
// ancestor-package rename), local-generic-instantiation dispatch with
// ancestor-prefix expansion, variable-type dispatch with multi-hop record
// field walk + generic instantiation chase + subtype-alias package-method
// fallback, qualified-name ci walk + chase-instantiation, fully-qualified
// variable-at-package-scope chains, fully-qualified variable field chains,
// partial qualification expansion, last-segment import shorthand, then the
// generic DefaultResolver ladder as the final fallback. Also a GNATCOLL.SQL.Exec
// / Execute_Query DB-query SQL-parser flow detector and build_file_context with
// both `with` and `use` producing wildcard imports.
//
// Case-insensitive same-file and dotted-qname resolution are NOT here: those
// fold via the profile's `name_normalization` spec in the generic ladder, which
// runs ahead of this residue resolver. Modular-type primitives (RM 13.7) are
// classified by `classify_external`.
// =============================================================================

use super::chain::{
    chase_instantiation, probe_dotted_qname, probe_package_of_type, walk_field_chain,
};
use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution,
    SymbolInfo, SymbolLookup, RESOLVED_CONFIDENCE,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile, SymbolKind};

#[cfg(test)]
pub(super) fn _test_probe_package_of_type(
    target: &str,
    edge_kind: EdgeKind,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    probe_package_of_type(target, edge_kind, lookup)
}

#[cfg(test)]
pub(super) fn _test_walk_field_chain(
    base_type: &str,
    segs: &[&str],
    edge_kind: EdgeKind,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    walk_field_chain(base_type, segs, edge_kind, lookup)
}

pub struct AdaResolver;

impl AdaResolver {
    pub(crate) fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        build_file_context_inner(file, project_ctx)
    }

    pub(crate) fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;


        let target_lower = target.to_lowercase();
        let simple = target.split('.').last().unwrap_or(target);
        let simple_lower = simple.to_lowercase();

        // Bare-name lookup against EVERY imported package (with or use).
        // `use Ada.Text_IO;` brings exports into bare scope (wildcard case).
        // But Ada also implicitly imports primitive operations of types
        // declared by `with`-only imports: `with Interfaces;` plus an
        // `Interfaces.Unsigned_16` value makes `Shift_Right(X, N)`
        // automatically callable bare because it's a primitive on the
        // modular type.
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
                            flow_emit: None,
                        });
                    }
                }
            }
        }

        // Own-package + ancestor-package visibility. A package body implicitly
        // sees all declarations from its own spec (indexed under the same
        // package qname) and from every ancestor package.
        if !target.contains('.') {
            if let Some(own_pkg) = &file_ctx.file_namespace {
                let parts: Vec<&str> = own_pkg.split('.').collect();
                for depth in (1..=parts.len()).rev() {
                    let ancestor = parts[..depth].join(".");
                    for member in lookup.members_of(&ancestor) {
                        if member.name.to_lowercase() == simple_lower
                            && predicates::kind_compatible(edge_kind, &member.kind)
                        {
                            return Some(Resolution {
                                target_symbol_id: member.id,
                                confidence: RESOLVED_CONFIDENCE,
                                strategy: "ada_parent_pkg_visibility",
                                resolved_yield_type: None,
                                flow_emit: None,
                            });
                        }
                        // Apply rename: `package Trace renames Simple_Logging;`
                        // emitted as member `Trace` with signature
                        // `renames Simple_Logging`. Rewrite and probe.
                        if let Some(sig) = &member.signature {
                            if let Some(rename_target) = sig.strip_prefix("renames ") {
                                if member.name.to_lowercase() == simple_lower {
                                    for sym in lookup.members_of(rename_target) {
                                        if sym.name.to_lowercase() == simple_lower
                                            && predicates::kind_compatible(edge_kind, &sym.kind)
                                        {
                                            return Some(Resolution {
                                                target_symbol_id: sym.id,
                                                confidence: RESOLVED_CONFIDENCE,
                                                strategy: "ada_parent_pkg_rename",
                                                resolved_yield_type: None,
                                                flow_emit: None,
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

        // Alias substitution — three paths:
        //  1. File-local rename: `package ASU renames Ada.Strings.Unbounded;`
        //     produces Imports ref target_name="ASU" + module="Ada.Strings.Unbounded".
        //  2. Cross-file rename visible via `use`: a `package ASU renames …`
        //     declared in a use'd package surfaces as a Namespace symbol with
        //     `signature = "renames …"`.
        //  3. Ancestor-package rename: child package inherits the rename
        //     declaration from its ancestor's spec via Ada parent-package
        //     visibility.
        if target.contains('.') {
            let leading = target.split('.').next().unwrap_or("");
            let leading_lower = leading.to_lowercase();
            let suffix = &target[leading.len()..];

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

            // Path 3: ancestor-package rename.
            if let Some(own_pkg) = &file_ctx.file_namespace {
                let parts: Vec<&str> = own_pkg.split('.').collect();
                for depth in (1..parts.len()).rev() {
                    let ancestor = parts[..depth].join(".");
                    for member in lookup.members_of(&ancestor) {
                        if member.name.to_lowercase() != leading_lower {
                            continue;
                        }
                        let Some(sig) = &member.signature else { continue };
                        let Some(rename_target) = sig.strip_prefix("renames ") else { continue };
                        let rewritten = format!("{rename_target}{suffix}");
                        if let Some(res) = probe_dotted_qname(&rewritten, edge_kind, lookup) {
                            return Some(Resolution {
                                strategy: "ada_ancestor_pkg_rename",
                                ..res
                            });
                        }
                    }
                }
            }
        }

        // Local-package instantiation dispatch.
        // `Sub_Cmd.Register(...)` where `Sub_Cmd` is a locally-declared
        // generic instantiation (Namespace symbol with
        // `signature = "instantiates CLIC.Subcommand.Instance"`).
        if target.contains('.') {
            let leading = target.split('.').next().unwrap_or("");
            let leading_lower = leading.to_lowercase();
            let suffix = &target[leading.len()..];

            // Ada namespace symbols may store the full dotted package name as
            // `name` (e.g. `name = "Alire.Containers"`) or just the leaf
            // segment. Match both forms.
            let ns_name_matches = |sym: &SymbolInfo| -> bool {
                let n = sym.name.to_lowercase();
                n == leading_lower
                    || n.ends_with(&format!(".{leading_lower}"))
            };
            let mut candidates: Vec<String> = Vec::new();
            for sym in lookup.in_file(&file_ctx.file_path) {
                if sym.kind == "namespace" && ns_name_matches(sym) {
                    if let Some(sig) = &sym.signature {
                        if let Some(gs) = sig.strip_prefix("instantiates ") {
                            candidates.push(gs.to_string());
                        }
                    }
                }
            }
            if let Some(own_pkg) = &file_ctx.file_namespace {
                let parts: Vec<&str> = own_pkg.split('.').collect();
                for depth in (1..=parts.len()).rev() {
                    let scope = parts[..depth].join(".");
                    for sym in lookup.members_of(&scope) {
                        if sym.kind == "namespace" && ns_name_matches(sym) {
                            if let Some(sig) = &sym.signature {
                                if let Some(gs) = sig.strip_prefix("instantiates ") {
                                    let gs = gs.to_string();
                                    if !candidates.contains(&gs) {
                                        candidates.push(gs);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            for generic_src in &candidates {
                let rewritten = format!("{generic_src}{suffix}");
                if let Some(res) = probe_dotted_qname(&rewritten, edge_kind, lookup) {
                    return Some(Resolution {
                        strategy: "ada_local_instantiation",
                        ..res
                    });
                }
                let method = suffix.trim_start_matches('.').split('.').next_back().unwrap_or("");
                let method_lower = method.to_lowercase();
                for member in lookup.members_of(generic_src) {
                    let member_leaf = member
                        .qualified_name
                        .rsplit_once('.')
                        .map(|(_, n)| n)
                        .unwrap_or(&member.qualified_name);
                    if member_leaf.to_lowercase() == method_lower
                        && predicates::kind_compatible(edge_kind, &member.kind)
                    {
                        return Some(Resolution {
                            target_symbol_id: member.id,
                            confidence: RESOLVED_CONFIDENCE,
                            strategy: "ada_local_instantiation",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }

                // The instantiation signature stores the generic name as-
                // written. Try the generic source as a direct name and try
                // prepending each ancestor prefix of the file's namespace.
                let generic_src_lower = generic_src.to_lowercase();
                let mut expanded_generics: Vec<String> = Vec::new();
                if !lookup.by_name(generic_src).is_empty() {
                    expanded_generics.push(generic_src.clone());
                }
                if let Some(own_pkg) = &file_ctx.file_namespace {
                    let parts: Vec<&str> = own_pkg.split('.').collect();
                    for depth in (1..=parts.len()).rev() {
                        let prefix = parts[..depth].join(".");
                        let candidate = format!("{prefix}.{generic_src}");
                        if !expanded_generics.iter().any(|e| e.to_lowercase() == candidate.to_lowercase()) {
                            expanded_generics.push(candidate);
                        }
                    }
                }
                for expanded_generic in &expanded_generics {
                    let expanded_lower = expanded_generic.to_lowercase();
                    for expanded_sym in lookup.by_name(expanded_generic) {
                        if expanded_sym.qualified_name.to_lowercase() == expanded_lower
                            && expanded_sym.kind == "namespace"
                        {
                            let expanded_rewritten = format!("{}{}", expanded_sym.qualified_name, suffix);
                            if let Some(res) = probe_dotted_qname(&expanded_rewritten, edge_kind, lookup) {
                                return Some(Resolution {
                                    strategy: "ada_local_instantiation",
                                    ..res
                                });
                            }
                            for member in lookup.members_of(&expanded_sym.qualified_name) {
                                let member_leaf = member
                                    .qualified_name
                                    .rsplit_once('.')
                                    .map(|(_, n)| n)
                                    .unwrap_or(&member.qualified_name);
                                if member_leaf.to_lowercase() == method_lower
                                    && predicates::kind_compatible(edge_kind, &member.kind)
                                {
                                    return Some(Resolution {
                                        target_symbol_id: member.id,
                                        confidence: RESOLVED_CONFIDENCE,
                                        strategy: "ada_local_instantiation",
                                        resolved_yield_type: None,
                                        flow_emit: None,
                                    });
                                }
                            }
                        }
                    }
                    let expanded_rewritten = format!("{expanded_generic}{suffix}");
                    if let Some(res) = probe_dotted_qname(&expanded_rewritten, edge_kind, lookup) {
                        return Some(Resolution {
                            strategy: "ada_local_instantiation",
                            ..res
                        });
                    }
                }
                let _ = generic_src_lower;
            }
        }

        // Variable-type dispatch.
        // `Result.Append(...)` where `Result` is a local variable typed `Vector`.
        // The extractor emits `object_declaration` / `parameter_specification` as a
        // Variable symbol with `signature = "type: T"`.
        if target.contains('.') {
            let leading = target.split('.').next().unwrap_or("");
            let leading_lower = leading.to_lowercase();
            let suffix = &target[leading.len()..];

            let mut var_types: Vec<String> = Vec::new();

            for sym in lookup.in_file(&file_ctx.file_path) {
                if sym.kind != "variable" || sym.name.to_lowercase() != leading_lower {
                    continue;
                }
                let Some(sig) = &sym.signature else { continue };
                let Some(ty) = sig.strip_prefix("type: ") else { continue };
                var_types.push(ty.to_string());
            }

            // Package-level variables brought into scope by wildcard imports.
            // Also collect the source package qname so we can probe it as a
            // type-package fallback for subtype-alias variables.
            let mut var_packages: Vec<String> = Vec::new();
            for import in &file_ctx.imports {
                if !import.is_wildcard {
                    continue;
                }
                let Some(module_path) = &import.module_path else { continue };
                for sym in lookup.members_of(module_path) {
                    if sym.kind != "variable" || sym.name.to_lowercase() != leading_lower {
                        continue;
                    }
                    let Some(sig) = &sym.signature else { continue };
                    let Some(ty) = sig.strip_prefix("type: ") else { continue };
                    var_types.push(ty.to_string());
                    var_packages.push(module_path.to_string());
                }
            }

            for ty in &var_types {
                let mut type_candidates: Vec<String> = Vec::new();
                type_candidates.push(ty.to_string());
                let ty_leaf = ty.split('.').next_back().unwrap_or(ty);
                for ty_sym in lookup.types_by_name(ty_leaf) {
                    type_candidates.push(ty_sym.qualified_name.clone());
                }

                for base_type in &type_candidates {
                    let segs: Vec<&str> = suffix.trim_start_matches('.').split('.').collect();
                    if segs.is_empty() {
                        continue;
                    }

                    // Multi-hop field walk for chains with intermediate segments.
                    if segs.len() > 1 {
                        if let Some(res) = walk_field_chain(base_type, &segs, edge_kind, lookup) {
                            return Some(res);
                        }
                    }

                    // Single-hop: probe type directly and at package level.
                    let method_suffix = format!(".{}", segs.join("."));
                    let rewritten = format!("{base_type}{method_suffix}");
                    if let Some(res) = probe_dotted_qname(&rewritten, edge_kind, lookup) {
                        return Some(res);
                    }
                    if let Some(res) = probe_package_of_type(&rewritten, edge_kind, lookup) {
                        return Some(res);
                    }

                    // Chase one level of generic instantiation.
                    if let Some(chained) = chase_instantiation(&rewritten, lookup) {
                        if let Some(res) = probe_dotted_qname(&chained, edge_kind, lookup) {
                            return Some(res);
                        }
                        if let Some(res) = probe_package_of_type(&chained, edge_kind, lookup) {
                            return Some(res);
                        }
                    }
                }

                // Chase instantiation on the bare-form suffix as fallback.
                let bare_rewritten = format!("{ty}{suffix}");
                if let Some(chained) = chase_instantiation(&bare_rewritten, lookup) {
                    if let Some(res) = probe_dotted_qname(&chained, edge_kind, lookup) {
                        return Some(res);
                    }
                    if let Some(res) = probe_package_of_type(&chained, edge_kind, lookup) {
                        return Some(res);
                    }
                }
            }

            // Subtype-alias fallback. `Green_LED : User_LED` where `User_LED`
            // is a subtype alias not independently indexed — primitive
            // operations for the underlying type live in the variable's
            // declaring package.
            if suffix.split('.').filter(|s| !s.is_empty()).count() == 1 {
                let method = suffix.trim_start_matches('.');
                let method_lower = method.to_lowercase();
                for pkg in &var_packages {
                    for sym in lookup.members_of(pkg) {
                        let sym_leaf = sym
                            .qualified_name
                            .rsplit_once('.')
                            .map(|(_, n)| n)
                            .unwrap_or(&sym.qualified_name);
                        if sym_leaf.to_lowercase() == method_lower
                            && predicates::kind_compatible(edge_kind, &sym.kind)
                        {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: RESOLVED_CONFIDENCE,
                                strategy: "ada_var_pkg_method",
                                resolved_yield_type: None,
                                flow_emit: None,
                            });
                        }
                    }
                }
            }
        }

        // Dotted target with leading segment matching a use'd package.
        if target.contains('.') {
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
                            confidence: RESOLVED_CONFIDENCE,
                            strategy: "ada_qualified_ci",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
            // Chase one level of generic instantiation.
            if let Some(chased) = chase_instantiation(target, lookup) {
                if let Some(res) = probe_dotted_qname(&chased, edge_kind, lookup) {
                    return Some(Resolution {
                        strategy: "ada_qualified_ci",
                        ..res
                    });
                }
            }
        }

        // Fully-qualified variable-at-package-scope chains. For a target like
        // `AAA.Strings.Empty_Vector.Append` where `AAA.Strings.Empty_Vector`
        // is a package-level variable, strip the variable name and probe the
        // owning package.
        if target.contains('.') {
            if let Some(res) = probe_package_of_type(target, edge_kind, lookup) {
                return Some(res);
            }
        }

        // Fully-qualified variable field chains.
        // `Pkg.Sub.Var.Field` where Var is at a deep qname (SVD-generated
        // peripheral instances). For each prefix, look up the variable
        // symbol, read its type from the signature, walk remaining segments.
        if target.contains('.') {
            let parts: Vec<&str> = target.split('.').collect();
            if parts.len() >= 3 {
                for var_end in (2..parts.len()).rev() {
                    let var_qname = parts[..var_end].join(".");
                    let remainder: Vec<&str> = parts[var_end..].to_vec();
                    if let Some(sym) = lookup.by_qualified_name(&var_qname) {
                        if sym.kind == "variable" {
                            if let Some(sig) = &sym.signature {
                                if let Some(ty) = sig.strip_prefix("type: ") {
                                    let ty_leaf =
                                        ty.split('.').next_back().unwrap_or(ty);
                                    let base_candidates: Vec<String> = {
                                        let mut v = vec![ty.to_string()];
                                        v.extend(
                                            lookup
                                                .types_by_name(ty_leaf)
                                                .iter()
                                                .map(|s| s.qualified_name.clone()),
                                        );
                                        v
                                    };
                                    for base in &base_candidates {
                                        if remainder.len() == 1 {
                                            let candidate = format!("{base}.{}", remainder[0]);
                                            if let Some(res) = probe_dotted_qname(
                                                &candidate,
                                                edge_kind,
                                                lookup,
                                            ) {
                                                return Some(Resolution {
                                                    strategy: "ada_qual_var_field_chain",
                                                    ..res
                                                });
                                            }
                                        } else if let Some(res) = walk_field_chain(
                                            base,
                                            &remainder
                                                .iter()
                                                .map(|s| *s)
                                                .collect::<Vec<_>>(),
                                            edge_kind,
                                            lookup,
                                        ) {
                                            return Some(Resolution {
                                                strategy: "ada_qual_var_field_chain",
                                                ..res
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

        // Partial qualification expansion. Ada child-package files may omit
        // the shared ancestor prefix in dotted calls.
        if target.contains('.') {
            if let Some(own_pkg) = &file_ctx.file_namespace {
                let parts: Vec<&str> = own_pkg.split('.').collect();
                for depth in (1..=parts.len()).rev() {
                    let prefix = parts[..depth].join(".");
                    let expanded = format!("{prefix}.{target}");
                    if let Some(res) = probe_dotted_qname(&expanded, edge_kind, lookup) {
                        return Some(Resolution {
                            strategy: "ada_partial_qualification",
                            ..res
                        });
                    }
                    if let Some(chased) = chase_instantiation(&expanded, lookup) {
                        if let Some(res) = probe_dotted_qname(&chased, edge_kind, lookup) {
                            return Some(Resolution {
                                strategy: "ada_partial_qualification",
                                ..res
                            });
                        }
                    }
                }
            }
        }

        // Last-segment import shorthand. `Text_IO.Put_Line` when
        // `with Ada.Text_IO;` is in scope.
        if target.contains('.') {
            let leading = target.split('.').next().unwrap_or("");
            let leading_lower = leading.to_lowercase();
            let suffix = &target[leading.len()..];
            for import in &file_ctx.imports {
                let Some(module_path) = &import.module_path else { continue };
                let last_seg = module_path.rsplit_once('.').map(|(_, s)| s).unwrap_or(module_path);
                if last_seg.to_lowercase() == leading_lower && module_path.to_lowercase() != leading_lower {
                    let rewritten = format!("{module_path}{suffix}");
                    if let Some(res) = probe_dotted_qname(&rewritten, edge_kind, lookup) {
                        return Some(Resolution {
                            strategy: "ada_last_seg_import",
                            ..res
                        });
                    }
                }
            }
        }

        let _ = target_lower;
        (DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: predicates::kind_compatible,
        })
        .resolve_all()
    }
}

// Given an Ada body path (`foo/bar.adb`), return the sibling spec path
// (`foo/bar.ads`). Returns None for any non-.adb file.
pub(crate) fn spec_for_body(file_path: &str) -> Option<String> {
    let normalized = file_path.replace('\\', "/");
    if normalized.ends_with(".adb") {
        let stem = &normalized[..normalized.len() - 4];
        Some(format!("{stem}.ads"))
    } else {
        None
    }
}

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;
    let r = &ref_ctx.extracted_ref;
    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }
    let target = r.target_name.as_str();
    // GNATCOLL.SQL.Exec / AdaSQL Execute_Query.
    if matches!(target, "Exec" | "Execute_Query" | "Execute" | "Query" | "Prepare") {
        let sql = r.call_args.iter().find_map(|a| match a {
            CallArg::StringLit(s) => Some(s.as_str()),
            _ => None,
        });
        if let Some(sql) = sql {
            let upper = sql.to_ascii_uppercase();
            let op = if upper.contains("INSERT INTO") {
                DbQueryOp::Insert
            } else if upper.contains("UPDATE ") {
                DbQueryOp::Update
            } else if upper.contains("DELETE FROM") {
                DbQueryOp::Delete
            } else if upper.contains(" FROM ") || upper.starts_with("SELECT") {
                DbQueryOp::Select
            } else {
                return Vec::new();
            };
            return vec![FlowEmission::DbQuery {
                entity_name: "ada.*".to_string(),
                operation: op,
            }];
        }
    }
    Vec::new()
}

pub(crate) fn build_file_context_inner(
    file: &ParsedFile,
    _project_ctx: Option<&ProjectContext>,
) -> FileContext {
    let mut imports = Vec::new();

    // Outermost package/namespace qname (e.g. `Alr.Commands.Run`). Ada
    // body/spec files declare exactly one top-level package.
    let file_namespace = file
        .symbols
        .iter()
        .find(|s| s.kind == SymbolKind::Namespace && s.parent_index.is_none())
        .map(|s| s.qualified_name.clone());

    for r in &file.refs {
        if r.kind != EdgeKind::Imports {
            continue;
        }
        // Both `with` and `use` clauses produce Imports edges.
        // package_renaming_declaration sets `module` to the renamed-target
        // package (for `package Trace renames Simple_Logging;` the ref
        // carries target_name="Trace" and module=Some("Simple_Logging")).
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

pub struct AdaHooks;

impl LanguageEngineHooks for AdaHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        let root = target.split('.').next().unwrap_or(target);

        if matches!(root, "Ada" | "System" | "Interfaces" | "GNAT" | "Standard") {
            return Some(root.to_string());
        }

        // Language-defined modular-type primitives (RM 13.7) are implicitly
        // visible bare wherever a modular type is in scope. They are operations
        // declared in `Interfaces`, so classify them there when no project
        // symbol shadows the name.
        if !target.contains('.')
            && matches!(
                root,
                "Shift_Left"
                    | "Shift_Right"
                    | "Shift_Right_Arithmetic"
                    | "Rotate_Left"
                    | "Rotate_Right"
            )
        {
            return Some("Interfaces".to_string());
        }

        if !target.contains('.')
            && matches!(
                root,
                "Long_Integer"
                    | "Long_Long_Integer"
                    | "Short_Integer"
                    | "Short_Short_Integer"
                    | "Integer_8"
                    | "Integer_16"
                    | "Integer_32"
                    | "Integer_64"
                    | "Unsigned_8"
                    | "Unsigned_16"
                    | "Unsigned_32"
                    | "Unsigned_64"
                    | "Long_Float"
                    | "Long_Long_Float"
                    | "Short_Float"
                    | "Duration"
                    | "Wide_Character"
                    | "Wide_Wide_Character"
                    | "Wide_String"
                    | "Wide_Wide_String"
            )
        {
            return Some("Standard".to_string());
        }
        None
    }

    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let _ = lookup;
        detect_flow_inner(file_ctx, ref_ctx)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(build_file_context_inner(file, project_ctx))
    }

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        AdaResolver.resolve(file_ctx, ref_ctx, lookup)
    }
}

pub static ADA_HOOKS: AdaHooks = AdaHooks;
