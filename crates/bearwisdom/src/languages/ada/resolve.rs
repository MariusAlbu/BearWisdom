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
    SymbolInfo, SymbolLookup,
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

#[cfg(test)]
pub(super) fn _test_walk_field_chain(
    base_type: &str,
    segs: &[&str],
    edge_kind: EdgeKind,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    walk_field_chain(base_type, segs, edge_kind, lookup)
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

            // Path 3: ancestor-package rename. A child package `Alr.Commands.Run`
            // inherits all declarations from ancestor specs (`Alr`, `Alr.Commands`).
            // When an ancestor spec contains `package Trace renames Simple_Logging;`,
            // any child package can call `Trace.Detail` without an explicit `with` or
            // `use` — the name is visible purely through Ada's parent-package visibility.
            // The bare-name probe above handles single-segment targets; this path
            // handles dotted calls where the leading segment is such an ancestor rename.
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

        // Local-package instantiation dispatch: `Sub_Cmd.Register(...)` where
        // `Sub_Cmd` is a locally-declared generic instantiation (namespace with
        // `signature = "instantiates CLIC.Subcommand.Instance"`). Unlike the
        // variable-type path below, the leading segment is a Namespace symbol,
        // not a Variable. Probe `members_of(generic_source)` directly and via
        // `probe_dotted_qname` for the trailing method.
        //
        // Two candidate sources for the leading namespace:
        //   1. `in_file` — symbols defined in the current compilation unit.
        //   2. `members_of(file_namespace)` — members of the file's own package,
        //      which includes instantiations from the sibling `.ads` spec (merged
        //      at the index level under the same parent qname).
        if target.contains('.') {
            let leading = target.split('.').next().unwrap_or("");
            let leading_lower = leading.to_lowercase();
            let suffix = &target[leading.len()..];

            // Collect candidate namespace-instantiation symbols from:
            //   1. in_file — compilation unit's own namespace body/spec.
            //   2. members_of(file_namespace) — own package members (includes
            //      instantiations from the sibling .ads spec).
            //   3. members_of(ancestor) for each ancestor package — a child
            //      package body implicitly sees instantiations declared in any
            //      ancestor spec (Ada parent-package visibility rule).
            //
            // Name matching: Ada namespace symbols may store the full dotted
            // package name as `name` (e.g. `name = "Alire.Containers"` for a
            // top-level `package body Alire.Containers is` declaration) or
            // just the leaf segment (e.g. `name = "Version_Outcomes"` for a
            // nested instantiation). Match both forms.
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
                // depth==parts.len(): own package (members visible in body).
                // depth<parts.len(): ancestor packages (parent-pkg visibility).
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
                            confidence: 0.9,
                            strategy: "ada_local_instantiation",
                            resolved_yield_type: None,
                        });
                    }
                }

                // The instantiation signature stores the generic name as-written in
                // source — which may be a partial name when the generic is in scope
                // via a `with` clause. Ada stores namespace `name` = the full package
                // identifier (e.g. `Alire.Outcomes.Definite`), so look it up directly
                // and also try ancestor-prefix expansions of the file's own namespace.
                let generic_src_lower = generic_src.to_lowercase();
                let mut expanded_generics: Vec<String> = Vec::new();
                // Try the generic source as a direct name (Ada name = full dotted id).
                if !lookup.by_name(generic_src).is_empty() {
                    expanded_generics.push(generic_src.clone());
                }
                // Try prepending each ancestor prefix of the file's own namespace.
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
                    // by_name in Ada uses full dotted identifier as the `name` field.
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
                                        confidence: 0.88,
                                        strategy: "ada_local_instantiation",
                                        resolved_yield_type: None,
                                    });
                                }
                            }
                        }
                    }
                    // Also probe directly by qualified_name in case by_name misses it.
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
        //
        // Multi-segment chains (`This.Port.Mem_Read`): after resolving the
        // head variable's type, walk each intermediate segment as a record
        // field — look up `field_type_name(current_type.Segment)` and advance
        // the current type — until only the trailing method segment remains.
        // Depth is capped at 6 to avoid infinite cycles on malformed indexes.
        //
        // Variable scope searched:
        //   1. in_file — local subprogram variables and parameters.
        //   2. members_of(use'd packages) — package-level variables brought
        //      into bare scope by `use` clauses (e.g. `use STM32.Board;`
        //      makes the `Display` variable callable as `Display.Hidden_Buffer`).
        if target.contains('.') {
            let leading = target.split('.').next().unwrap_or("");
            let leading_lower = leading.to_lowercase();
            let suffix = &target[leading.len()..];

            // Collect variable symbols from both in_file and use'd-package members.
            // Store (type_string, symbol_id_unused) pairs; we only need the type.
            let mut var_types: Vec<String> = Vec::new();

            // Source 1: in_file variables.
            for sym in lookup.in_file(&file_ctx.file_path) {
                if sym.kind != "variable" || sym.name.to_lowercase() != leading_lower {
                    continue;
                }
                let Some(sig) = &sym.signature else { continue };
                let Some(ty) = sig.strip_prefix("type: ") else { continue };
                var_types.push(ty.to_string());
            }

            // Source 2: package-level variables brought into scope by wildcard imports.
            // Also collect the source package qname so we can probe it as a type
            // package fallback (for subtype-alias variables like `Green_LED : User_LED`
            // where `User_LED` is a subtype of the underlying type — the method
            // `Toggle` lives in the variable's declaring package, not the type).
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
                // Resolve initial variable type to a set of candidate qnames.
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

                    // Chase one level of generic instantiation and re-probe.
                    if let Some(chained) = chase_instantiation(&rewritten, lookup) {
                        if let Some(res) = probe_dotted_qname(&chained, edge_kind, lookup) {
                            return Some(res);
                        }
                        if let Some(res) = probe_package_of_type(&chained, edge_kind, lookup) {
                            return Some(res);
                        }
                    }
                }

                // Chase instantiation on the bare-form suffix as a fallback.
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

            // Fallback: when a variable from a use'd package has a single-segment
            // type name (e.g. `Green_LED : User_LED` where `User_LED` is a subtype
            // alias not independently indexed), Ada's primitive operations for the
            // underlying type often live in the variable's declaring package. Probe
            // the source package directly for the trailing method name.
            //
            // Example: `Green_LED.Toggle` — variable `STM32.Board.Green_LED` typed
            // `User_LED` (subtype of GPIO_Point) — `Toggle` lives in `STM32.Board`.
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
                                confidence: 0.82,
                                strategy: "ada_var_pkg_method",
                                resolved_yield_type: None,
                            });
                        }
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
            // Also chase one level of generic instantiation: a fully-qualified
            // target like `Alire.Containers.Crate_Name_Sets.To_Set` won't have
            // `Crate_Name_Sets.To_Set` directly, but `Crate_Name_Sets` is an
            // instantiation of a generic that does have `To_Set`.
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
        // is a package-level variable (not a local), the qualified-name walk
        // above won't find `Append` because `members_of("AAA.Strings.Empty_Vector")`
        // returns nothing (it's a variable, not a namespace). Strip the
        // second-to-last segment (the object/variable name) and probe the
        // owning package — the same transformation that `probe_package_of_type`
        // applies to type-qualified calls.
        if target.contains('.') {
            if let Some(res) = probe_package_of_type(target, edge_kind, lookup) {
                return Some(res);
            }
        }

        // Partial qualification expansion. Ada child-package files may omit the
        // shared ancestor prefix in dotted calls — a file in `Alire.Index.Search`
        // that does `with Alire.Utils.TTY` may call `Utils.TTY.Name` (dropping
        // the leading `Alire.`). Try prepending each ancestor prefix of the
        // file's own namespace and re-probing the expanded qualified name.
        // After the direct qname probe, also chase one level of generic
        // instantiation so that partially-qualified calls into instantiated
        // packages (e.g. `Containers.Crate_Name_Sets.To_Set` → expanded to
        // `Alire.Containers.Crate_Name_Sets.To_Set` → chased to
        // `Ada.Containers.Indefinite_Ordered_Sets.To_Set`) resolve.
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
                    // Chase one level of instantiation on the expanded form.
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

        // Last-segment import shorthand. Ada code may qualify a call using only
        // the last segment of a `with`-imported package: `Text_IO.Put_Line` when
        // `with Ada.Text_IO;` is in scope. When the target's leading segment
        // matches the last dot-segment of an imported module path, rewrite the
        // target using the full package name and probe again.
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
        engine::resolve_common("ada", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        let root = target.split('.').next().unwrap_or(target);

        // Ada standard library packages (root segment) — classify both Imports
        // and Calls edges so that unresolved `GNAT.OS_Lib.Is_Directory` calls
        // and similar are attributed to the stdlib namespace rather than left as
        // opaque unresolved refs.
        if matches!(root, "Ada" | "System" | "Interfaces" | "GNAT" | "Standard") {
            return Some(root.to_string());
        }

        // Bare names are classified by the engine's keywords() set
        // populated from ada/keywords.rs.
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

/// Walk a multi-segment field chain starting from a resolved type qname.
///
/// Given `base_type = "Drivers.Device"` and `segs = ["Port", "Mem_Read"]`,
/// resolves `Port` as a field of `Device`, obtains its type (e.g.,
/// `Drivers.Port_Type`), then probes `members_of` and the package-of-type
/// for `Mem_Read` against that type. Returns the first resolution found.
///
/// Depth is capped at 6 hops to guard against malformed or cyclic indexes.
/// Gives up (returns `None`) if any intermediate field's type is unknown.
fn walk_field_chain(
    base_type: &str,
    segs: &[&str],
    edge_kind: EdgeKind,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    const MAX_DEPTH: usize = 6;
    if segs.len() > MAX_DEPTH {
        return None;
    }
    // segs = [intermediate..., method]. Walk all but the last to resolve types.
    let intermediates = &segs[..segs.len() - 1];
    let method = segs[segs.len() - 1];
    let method_lower = method.to_lowercase();

    let mut current_type = base_type.to_string();
    for field_seg in intermediates {
        let field_lower = field_seg.to_lowercase();
        let field_qname = format!("{current_type}.{field_seg}");
        // Look up the field type by exact qname; fall back to a
        // case-insensitive scan of the current type's members.
        // Ada extractors store field type in `signature = "type: T"` rather
        // than emitting TypeRef edges, so also read the signature as a fallback
        // when field_type_name (which queries TypeRef edges) returns None.
        let next_type = lookup
            .field_type_name(&field_qname)
            .map(|s| s.to_string())
            .or_else(|| {
                lookup.members_of(&current_type).iter().find_map(|m| {
                    let leaf = m
                        .qualified_name
                        .rsplit_once('.')
                        .map(|(_, n)| n)
                        .unwrap_or(&m.qualified_name);
                    if leaf.to_lowercase() != field_lower {
                        return None;
                    }
                    // TypeRef-edge path.
                    lookup
                        .field_type_name(&m.qualified_name)
                        .map(|s| s.to_string())
                        // Signature-path fallback for Ada fields (no TypeRef edges).
                        .or_else(|| {
                            m.signature
                                .as_deref()
                                .and_then(|s| s.strip_prefix("type: "))
                                .map(|t| t.to_string())
                        })
                })
            });
        let Some(next_raw) = next_type else {
            return None; // Chain broken — give up.
        };
        // Expand bare field type to fully-qualified form when possible.
        let next_leaf = next_raw.split('.').next_back().unwrap_or(&next_raw);
        let expanded = lookup
            .types_by_name(next_leaf)
            .iter()
            .map(|s| s.qualified_name.clone())
            .next()
            .unwrap_or_else(|| next_raw.clone());
        current_type = expanded;
    }

    // current_type is the type reached after all intermediate hops.
    // Probe for the trailing method at type level, then at package level.
    let type_candidate = format!("{current_type}.{method}");
    if let Some(res) = probe_dotted_qname(&type_candidate, edge_kind, lookup) {
        return Some(res);
    }
    let parts: Vec<&str> = current_type.split('.').collect();
    if let Some(pkg_parts) = parts.split_last().map(|(_, rest)| rest) {
        if !pkg_parts.is_empty() {
            let pkg = pkg_parts.join(".");
            for sym in lookup.members_of(&pkg) {
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
                        confidence: 0.85,
                        strategy: "ada_field_chain",
                        resolved_yield_type: None,
                    });
                }
            }
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
