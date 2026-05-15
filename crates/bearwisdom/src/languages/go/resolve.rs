// =============================================================================
// indexer/resolve/rules/go/mod.rs — Go resolution rules
//
// Scope rules for Go:
//
//   1. Same-package resolution: all symbols declared in files with the same
//      package name are visible to each other without any import. This is the
//      dominant rule for intra-package calls.
//   2. Import resolution: `import "pkg/path"` makes exported symbols available
//      as `lastSegment.Symbol`. The ref target_name holds just the symbol name;
//      the Go extractor does NOT emit a module hint on call refs for selector
//      expressions — only the field identifier (method/function name) is stored.
//   3. Scope chain walk: for methods defined on a receiver type, walk the
//      qualified-name scope chain trying `{scope}.{target}`.
//   4. Fully qualified: dotted target_name resolved directly against the index.
//
// Go visibility:
//   Exported = first character uppercase → Public in our model.
//   Unexported = first character lowercase → Private.
//   Cross-package access requires the target to be exported (Public).
//
// Import format from the Go extractor (emit_import_ref):
//   target_name = last path segment (e.g., "gin" for "github.com/gin-gonic/gin")
//   module      = full import path    (e.g., "github.com/gin-gonic/gin")
//
// Call format from the Go extractor (extract_call_ref):
//   For `gin.Default()`:  target_name = "Default", module = None
//   For `fmt.Println()`:  target_name = "Println", module = None
//   For `localFunc()`:    target_name = "localFunc", module = None
//
// Key constraint: the extractor drops the package qualifier from call refs
// (it captures only the field_identifier). So "gin.Default()" becomes
// target_name = "Default" with no module. Disambiguation happens via imports.
// =============================================================================


use super::{predicates, type_checker::GoChecker};
use crate::type_checker::TypeChecker;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Go language resolver.
pub struct GoResolver;

impl LanguageResolver for GoResolver {
    fn language_ids(&self) -> &[&str] {
        &["go"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Derive the package name from symbols' scope_path or qualified_name prefix.
        // The Go extractor sets scope_path = Some(package_name) for top-level symbols,
        // and qualified_name = "package.SymbolName". We take the first segment.
        let file_namespace = extract_package_name(file);

        // Build import entries from EdgeKind::Imports refs.
        // The Go extractor emits:
        //   target_name = last path segment (the package alias by convention)
        //   module      = full import path
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let full_path = match &r.module {
                Some(m) => m.clone(),
                None => r.target_name.clone(),
            };

            // Detect alias, dot-import, and blank import by examining target_name
            // relative to the last segment of the full path.
            let last_segment = full_path.rsplit('/').next().unwrap_or(&full_path);

            // Blank import (`import _ "path"`) — side effects only, skip.
            if r.target_name == "_" {
                continue;
            }

            // Dot import (`import . "path"`) — all exported names enter scope directly.
            let is_dot_import = r.target_name == ".";

            // The alias used in source code: explicit alias overrides the last segment.
            let alias = if is_dot_import || r.target_name == last_segment {
                None
            } else {
                Some(r.target_name.clone())
            };

            imports.push(ImportEntry {
                imported_name: alias.clone().unwrap_or_else(|| last_segment.to_string()),
                module_path: Some(full_path),
                alias,
                // Dot imports bring all exported names into scope without qualification.
                // Regular imports require `pkg.Symbol` — not a wildcard in our model.
                is_wildcard: is_dot_import,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "go".to_string(),
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

        // Skip import refs — they declare scope, not symbol references.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Chain-aware resolution: if we have a structured MemberChain, walk it
        // step-by-step following field types.
        if let Some(chain_ref) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = GoChecker.resolve_chain(
                chain_ref, edge_kind, None, ref_ctx, lookup,
            ) {
                return Some(res);
            }

            // Package-qualified call: chain = ["pkg", "Func"].
            // Use the first segment to find the matching import, then resolve
            // the target as `{package_name}.{target}` with high confidence.
            if chain_ref.segments.len() >= 2 {
                let alias = &chain_ref.segments[0].name;
                if let Some(res) = self.resolve_via_import_alias(
                    file_ctx, alias, target, edge_kind, lookup,
                ) {
                    return Some(res);
                }
            }
        }

        // Step 1: Scope chain walk (innermost → outermost).
        // Handles methods calling sibling methods on the same receiver:
        //   scope_chain = ["main.Server", "main"]
        //   try "main.Server.Foo", "main.Foo"
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "go_scope_chain",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Step 2: Same-package resolution.
        // All symbols with the same package name are visible without import.
        // Try `{package}.{target}`.
        if let Some(pkg) = &file_ctx.file_namespace {
            let candidate = format!("{pkg}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "go_same_package",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }

            // Also check: direct children of the current package — top-level
            // functions / types / vars that can be called bare from the same
            // package. Uses members_of(pkg) so we scan the package's O(tens)
            // of direct children, not the O(all-symbols-named-target) pool
            // that by_name returns once externals are indexed.
            for sym in lookup.members_of(pkg) {
                if sym.name == *target
                    && self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "go_same_package_by_name",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Step 3: Import-based resolution.
        // For `gin.Default()`, the extractor emits target_name = "Default".
        // We need to find an import whose alias/last_segment maps to a package
        // that exports a symbol named `target`.
        for import in &file_ctx.imports {
            let Some(full_path) = &import.module_path else {
                continue;
            };

            // Dot import: all exported names from this package are directly visible.
            if import.is_wildcard {
                let last_seg = full_path.rsplit('/').next().unwrap_or(full_path.as_str());
                let candidate = format!("{last_seg}.{target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if self.is_visible(file_ctx, ref_ctx, sym)
                        && predicates::kind_compatible(edge_kind, &sym.kind)
                    {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "go_dot_import",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
                continue;
            }

            // The package alias used in source: explicit alias, otherwise last segment.
            let pkg_alias = import
                .alias
                .as_deref()
                .unwrap_or_else(|| full_path.rsplit('/').next().unwrap_or(full_path.as_str()));

            // The symbol index uses the Go package name (last segment of import path
            // by convention, unless the package declares a different name).
            // Try the conventional qualified name: `{last_segment}.{target}`.
            let last_seg = full_path.rsplit('/').next().unwrap_or(full_path.as_str());
            let candidate = format!("{last_seg}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "go_import",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }

            // If an explicit alias was used and differs from last_seg, also try alias.
            if pkg_alias != last_seg {
                let candidate = format!("{pkg_alias}.{target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if self.is_visible(file_ctx, ref_ctx, sym)
                        && predicates::kind_compatible(edge_kind, &sym.kind)
                    {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "go_import_alias",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
        }

        // Step 4: Fully qualified name (target contains dots, e.g., "pkg.Func").
        if target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "go_qualified_name",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Go bare-name fallback. 9th language using the
        // `<lang>_bare_name` template (PRs 31, 35-40, plus Lua).
        // Go's package-qualified calls (`pkg.Func()`) hit the
        // module path, but bare imports and lambda-style calls
        // benefit from index-wide name lookup. Gated by `.go`
        // file-extension and `kind_compatible`.
        //
        // Honors Go visibility: exported identifiers start with an
        // uppercase letter; unexported (lowercase-leading) names
        // can only resolve to symbols defined in the same package.
        // Skip lowercase-leading targets here unless the candidate
        // shares the source file's package directory — same logic
        // the deterministic paths apply.
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;
        if matches!(edge_kind, EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates)
            && ref_ctx.extracted_ref.module.is_none()
            && !target.contains('.')
        {
            let is_exported = target.chars().next().is_some_and(|c| c.is_uppercase());
            let source_dir = ref_ctx
                .extracted_ref
                .module
                .as_deref()
                .unwrap_or("")
                .to_string();
            let source_pkg_dir = file_ctx
                .file_path
                .rfind('/')
                .map(|i| &file_ctx.file_path[..i])
                .unwrap_or("");
            let _ = source_dir;
            for sym in lookup.by_name(target) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                let path = &sym.file_path;
                let is_go = path.ends_with(".go") || path.starts_with("ext:go:");
                if !is_go {
                    continue;
                }
                if !is_exported {
                    let target_pkg_dir = path
                        .rfind('/')
                        .map(|i| &path[..i])
                        .unwrap_or("");
                    if target_pkg_dir != source_pkg_dir {
                        continue;
                    }
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.80,
                    strategy: "go_bare_name",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        // Could not resolve deterministically — fall back to heuristic.
        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Import refs (e.g., `import "fmt"`, `import "mymodule/pkg"`).
        // These are namespace declarations, not symbol references — they don't
        // map to a single target symbol. Classify them all with their module path
        // so they move out of unresolved_refs (we know what they are).
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let import_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            return Some(import_path.to_string());
        }

        // Go built-in functions and types — always external (runtime/stdlib).
        if predicates::is_go_builtin(target) {
            return Some("builtin".to_string());
        }

        // Go composite literal types: []string, map[string]int, []*Foo, etc.
        if predicates::is_go_composite_type(target) {
            return Some("builtin".to_string());
        }

        // For non-import refs, "external namespace" means the import path of
        // the package this ref likely comes from. Only exported (capitalized)
        // names can come from external packages.
        let is_exported = target.chars().next().is_some_and(|c| c.is_uppercase());
        if !is_exported {
            return None;
        }

        let mut best: Option<&str> = None;

        for import in &file_ctx.imports {
            let Some(full_path) = &import.module_path else {
                continue;
            };

            // Manifest-driven: check go.mod external dependencies first.
            // go.mod external deps are full module paths (e.g., "github.com/gin-gonic/gin").
            // is_external_go_import already uses go_module_path from the manifest,
            // so this explicit check adds direct manifest validation as the first pass.
            let external = if let Some(ctx) = project_ctx {
                is_manifest_go_external(ctx, full_path)
            } else {
                predicates::is_external_go_import_fallback(full_path)
            };

            if external {
                // Prefer longer (more specific) paths.
                if best.is_none() || full_path.len() > best.unwrap().len() {
                    best = Some(full_path.as_str());
                }
            }
        }

        best.map(|s| s.to_string())
    }

    fn is_visible(
        &self,
        file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        target: &SymbolInfo,
    ) -> bool {
        let vis = target.visibility.as_deref().unwrap_or("public");

        // Private (unexported) symbols are only visible within the same package,
        // which in Go means files in the same directory with the same package name.
        // We approximate with same file_path prefix (same directory).
        if vis == "private" {
            // Same file is always fine.
            if &*target.file_path == file_ctx.file_path {
                return true;
            }
            // Same package: compare directories.
            let target_dir = predicates::parent_dir(&target.file_path);
            let source_dir = predicates::parent_dir(&file_ctx.file_path);
            return target_dir == source_dir;
        }

        // Public (exported) symbols are always visible from anywhere.
        true
    }

    fn detect_flow_emission(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let r = &ref_ctx.extracted_ref;
        if r.kind != EdgeKind::Calls {
            return Vec::new();
        }
        let Some(chain) = r.chain.as_ref() else { return Vec::new(); };
        if let Some(emission) = detect_go_http_chain_emission(chain, &r.call_args) {
            return vec![emission];
        }
        if let Some(emission) = detect_go_db_query_emission(chain, &r.call_args) {
            return vec![emission];
        }
        if let Some(emission) = detect_go_grpc_chain_emission(chain, file_ctx) {
            return vec![emission];
        }
        if let Some(emission) = detect_go_mailer_emission(chain, &r.call_args) {
            return vec![emission];
        }
        if let Some(emission) = detect_go_bgjob_emission(chain) {
            return vec![emission];
        }
        if let Some(emission) = detect_go_mq_emission(chain, &r.call_args) {
            return vec![emission];
        }
        if let Some(emission) = detect_go_redis_config_lookup(chain, &r.call_args) {
            return vec![emission];
        }
        if let Some(emission) = detect_go_uds_emission(chain, &r.call_args) {
            return vec![emission];
        }
        if let Some(emission) = detect_go_gorilla_ws_consumer(chain) {
            return vec![emission];
        }
        Vec::new()
    }

    fn detect_flow_emission_with_lookup(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let direct = self.detect_flow_emission(file_ctx, ref_ctx);
        if !direct.is_empty() {
            return direct;
        }
        // Let-binding propagation: `client := userpb.NewUserServiceClient(conn)`
        // followed by `client.GetUser(...)`. The variable's recorded type
        // (from the Go extractor's TypeRef on the Variable symbol) is
        // consulted via `lookup.field_type_name`.
        let r = &ref_ctx.extracted_ref;
        let Some(chain) = r.chain.as_ref() else { return Vec::new() };
        let Some(root_seg) = chain.segments.first() else { return Vec::new() };
        if !matches!(root_seg.kind, crate::types::SegmentKind::Identifier) {
            return Vec::new();
        }
        let var_qname = match ref_ctx.source_symbol.scope_path.as_deref() {
            Some(scope) => format!("{}.{}", scope, root_seg.name),
            None => root_seg.name.clone(),
        };
        let type_name = match lookup.field_type_name(&var_qname) {
            Some(t) => t.to_string(),
            None => return Vec::new(),
        };
        if !type_name.ends_with("Client") {
            return Vec::new();
        }
        let mut new_segments = vec![
            crate::types::ChainSegment {
                name: type_name,
                node_kind: "rewritten_var".to_string(),
                kind: crate::types::SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            },
        ];
        new_segments.extend(chain.segments.iter().skip(1).cloned());
        let rewritten = crate::types::MemberChain { segments: new_segments };
        if let Some(em) = detect_go_grpc_chain_emission(&rewritten, file_ctx) {
            return vec![em];
        }
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// HTTP Producer detection — net/http + resty
// ---------------------------------------------------------------------------

/// Recognise Go HTTP-client call shapes and emit Producer
/// `NamedChannel { kind: HttpCall, .. }` keyed on the first string-literal arg.
///
/// Shapes handled:
/// - `http.Get(url)` → Get; `http.Post(url, ct, body)` → Post;
///   `http.PostForm(url, ...)` → Post; `http.Head(url)` → Head;
///   `http.NewRequest("METHOD", url, body)` → method from first arg.
/// - resty: `client.R().Get(url)` / `.Post(url)` / etc. — verb leaf,
///   URL as first string arg.
pub(crate) fn detect_go_http_chain_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;

    let segs = &chain.segments;
    if segs.is_empty() {
        return None;
    }
    let root = segs.first()?.name.as_str();
    let leaf = segs.last()?.name.as_str();

    // `http.NewRequest("GET", "/x", body)` — method is the first string arg,
    // URL the second. Emit Producer with the parsed verb + URL.
    if root == "http" && leaf == "NewRequest" {
        let (method_arg, url_arg) = match (call_args.first(), call_args.get(1)) {
            (Some(CallArg::StringLit(m)), Some(CallArg::StringLit(u))) => (m.clone(), u.clone()),
            _ => return None,
        };
        if url_arg.is_empty() {
            return None;
        }
        let method = HttpMethod::from_method_name(method_arg.as_str());
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::HttpCall,
            name: crate::connectors::url_pattern::normalize(&url_arg),
            role: ChannelRole::Producer,
            method: Some(method),
        streaming: None,
        });
    }

    // `http.Get(url)` / `http.Post(url, ...)` / etc.
    if root == "http" && segs.len() == 2 {
        let method = parse_go_http_pkg_verb(leaf)?;
        let url = match call_args.first()? {
            CallArg::StringLit(s) => s.clone(),
            _ => return None,
        };
        if url.is_empty() {
            return None;
        }
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::HttpCall,
            name: crate::connectors::url_pattern::normalize(&url),
            role: ChannelRole::Producer,
            method: Some(method),
        streaming: None,
        });
    }

    // resty `client.R().Get("/x")` — chain has `R` then a verb leaf.
    if segs.iter().any(|s| s.name == "R") {
        let method = parse_resty_verb(leaf)?;
        let url = match call_args.first()? {
            CallArg::StringLit(s) => s.clone(),
            _ => return None,
        };
        if url.is_empty() {
            return None;
        }
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::HttpCall,
            name: crate::connectors::url_pattern::normalize(&url),
            role: ChannelRole::Producer,
            method: Some(method),
        streaming: None,
        });
    }

    None
}

fn parse_go_http_pkg_verb(name: &str) -> Option<crate::indexer::resolve::flow_emit::HttpMethod> {
    use crate::indexer::resolve::flow_emit::HttpMethod;
    Some(match name {
        "Get" => HttpMethod::Get,
        "Post" | "PostForm" => HttpMethod::Post,
        "Head" => HttpMethod::Head,
        _ => return None,
    })
}

fn parse_resty_verb(name: &str) -> Option<crate::indexer::resolve::flow_emit::HttpMethod> {
    use crate::indexer::resolve::flow_emit::HttpMethod;
    Some(match name {
        "Get" => HttpMethod::Get,
        "Post" => HttpMethod::Post,
        "Put" => HttpMethod::Put,
        "Patch" => HttpMethod::Patch,
        "Delete" => HttpMethod::Delete,
        "Head" => HttpMethod::Head,
        "Options" => HttpMethod::Options,
        "Execute" => HttpMethod::Any,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// DbQuery — database/sql + gorm
// ---------------------------------------------------------------------------

/// Recognise database/sql and gorm call shapes and emit Producer DbQuery
/// keyed on the entity/table name parsed from the call.
///
/// Shapes handled:
/// - `db.Query("SELECT ... FROM table")` / `db.QueryRow(...)` /
///   `db.Exec(...)` — entity parsed from SQL `FROM`/`UPDATE`/
///   `INSERT INTO`/`DELETE FROM` clause.
/// - gorm `db.First(&user)` / `db.Find(&users)` / `db.Save(&user)` /
///   `db.Create(&user)` / `db.Delete(&user)` — entity from the
///   `&Type{}` composite literal type or the pointer's struct type.
pub(crate) fn detect_go_db_query_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;

    let leaf = chain.segments.last()?.name.as_str();

    // database/sql — SQL text first arg.
    let sql_op = match leaf {
        "Query" | "QueryRow" | "QueryContext" | "QueryRowContext" => Some(DbQueryOp::Select),
        "Exec" | "ExecContext" => Some(DbQueryOp::Other),
        _ => None,
    };
    if let Some(default_op) = sql_op {
        if let Some(CallArg::StringLit(sql)) = call_args.first() {
            if let Some((entity, op)) = parse_sql_entity(sql, default_op) {
                return Some(FlowEmission::DbQuery {
                    entity_name: format!("go.{}", entity),
                    operation: op,
                });
            }
        }
        return None;
    }

    // gorm — entity is the first non-empty Ident arg (composite-literal
    // address-of or pointer variable).
    let gorm_op = match leaf {
        "First" | "Find" | "Take" | "Last" | "Where" | "Preload" | "Joins" | "Select"
        | "Distinct" | "Pluck" | "Count" | "Scan" | "Scopes" => Some(DbQueryOp::Select),
        "Create" | "CreateInBatches" => Some(DbQueryOp::Insert),
        "Save" | "Update" | "Updates" | "UpdateColumn" | "UpdateColumns" => Some(DbQueryOp::Update),
        "Delete" => Some(DbQueryOp::Delete),
        "FirstOrCreate" | "Upsert" => Some(DbQueryOp::Upsert),
        _ => None,
    };
    if let Some(op) = gorm_op {
        let entity = call_args.iter().find_map(|a| match a {
            CallArg::Ident(name) if is_pascal_case_first_go(name) => Some(name.clone()),
            _ => None,
        })?;
        return Some(FlowEmission::DbQuery {
            entity_name: format!("go.{}", strip_pointer_prefix(&entity)),
            operation: op,
        });
    }

    None
}

fn is_pascal_case_first_go(name: &str) -> bool {
    name.trim_start_matches('*')
        .trim_start_matches('&')
        .chars()
        .next()
        .map_or(false, |c| c.is_ascii_uppercase())
}

fn strip_pointer_prefix(name: &str) -> String {
    name.trim_start_matches('*')
        .trim_start_matches('&')
        .trim_start_matches("[]")
        .to_string()
}

/// Parse an SQL string to identify the entity (table) name and operation.
/// Handles the common SELECT / UPDATE / INSERT INTO / DELETE FROM shapes.
fn parse_sql_entity(
    sql: &str,
    fallback_op: crate::indexer::resolve::flow_emit::DbQueryOp,
) -> Option<(String, crate::indexer::resolve::flow_emit::DbQueryOp)> {
    use crate::indexer::resolve::flow_emit::DbQueryOp;
    let upper = sql.trim().to_ascii_uppercase();
    // Locate the verb + entity-token slot for each common operation.
    let (op, marker, after_marker): (DbQueryOp, &str, usize) =
        if let Some(i) = upper.find("UPDATE ") {
            (DbQueryOp::Update, " UPDATE ", i + 7)
        } else if let Some(i) = upper.find("INSERT INTO ") {
            (DbQueryOp::Insert, " INSERT INTO ", i + 12)
        } else if let Some(i) = upper.find("DELETE FROM ") {
            (DbQueryOp::Delete, " DELETE FROM ", i + 12)
        } else if let Some(i) = upper.find(" FROM ") {
            (DbQueryOp::Select, " FROM ", i + 6)
        } else if upper.starts_with("FROM ") {
            (DbQueryOp::Select, "FROM ", 5)
        } else {
            return None;
        };
    let _ = marker;
    let entity_slice = sql.get(after_marker..)?;
    let entity = entity_slice
        .split_whitespace()
        .next()?
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
        .to_string();
    if entity.is_empty() {
        return None;
    }
    let final_op = match op {
        DbQueryOp::Select => fallback_op_or(op, fallback_op),
        _ => op,
    };
    Some((entity, final_op))
}

fn fallback_op_or(
    primary: crate::indexer::resolve::flow_emit::DbQueryOp,
    fallback: crate::indexer::resolve::flow_emit::DbQueryOp,
) -> crate::indexer::resolve::flow_emit::DbQueryOp {
    use crate::indexer::resolve::flow_emit::DbQueryOp;
    // For `Exec` calls the SQL might contain a SELECT (unusual), but the
    // Go-side intent is a write operation. Prefer the SQL-parsed op for
    // explicit verbs (UPDATE/INSERT/DELETE) and fall back to the call
    // shape's hint for SELECTs (which would only land here for Exec).
    match (primary, fallback) {
        (DbQueryOp::Select, DbQueryOp::Other) => DbQueryOp::Select,
        (DbQueryOp::Select, fb) => fb,
        (p, _) => p,
    }
}

// ---------------------------------------------------------------------------
// gRPC Producer — `client.<Service>.<Method>(ctx, req)` chains
// ---------------------------------------------------------------------------

/// Recognise gRPC client chains and emit Producer `NamedChannel { kind: RpcCall, .. }`
/// keyed on `<service>/<method>` (canonical lowercase form). Fires only
/// when the file imports a gRPC-generated package — by convention the
/// import path ends in `pb`/`grpc` or contains `proto`.
pub(crate) fn detect_go_grpc_chain_emission(
    chain: &crate::types::MemberChain,
    file_ctx: &FileContext,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };

    // Require the file to import a generated proto / pb package — the
    // `google.golang.org/grpc` package alone is gRPC plumbing (Dial,
    // ServerOption, etc.), not service calls.
    if !file_imports_proto_pkg(file_ctx) {
        return None;
    }
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs.first()?.name.as_str();
    let leaf = segs.last()?.name.as_str();

    // Drop chains rooted at known stdlib / grpc-setup packages — these are
    // never service calls (fmt.Println, grpc.WithInsecure, context.Background,
    // etc.).
    if is_go_stdlib_or_setup_root(root) {
        return None;
    }
    // Drop lifecycle / factory / generic-leaf names that aren't gRPC
    // service methods. The `New*` factories are detected by prefix —
    // they construct the client, they don't call the service.
    if is_go_grpc_non_method_leaf(leaf) || leaf.starts_with("New") {
        return None;
    }
    // gRPC service method names are conventionally PascalCase with a
    // verb prefix (`Get`, `List`, `Create`, `Update`, `Delete`, `Stream`,
    // `Watch`, `Subscribe`, `Publish`, `Send`). Filtering on these prefixes
    // keeps simple-PascalCase struct-field accesses (`resp.Name`) out of
    // the rpc_call stream.
    if !looks_like_grpc_method_name(leaf) {
        return None;
    }
    // For 2-segment chains the root must look like a gRPC client binding —
    // either a `Client`-suffixed type identifier or a camelCase local
    // variable (`client`, `userClient`). Rejects single-segment-rooted
    // package calls.
    if segs.len() == 2
        && !root.ends_with("Client")
        && !is_camel_case_local(root)
    {
        return None;
    }

    let service = if segs.len() >= 3 {
        segs[segs.len() - 2].name.as_str()
    } else {
        segs[0].name.as_str()
    };
    let service_norm = strip_go_service_suffix(service);
    if service_norm.is_empty() {
        return None;
    }
    use crate::indexer::resolve::flow_emit::StreamKind;
    let streaming = Some(StreamKind::from_method_name(leaf));
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::RpcCall,
        name: format!(
            "{}/{}",
            service_norm.to_ascii_lowercase(),
            leaf.to_ascii_lowercase()
        ),
        role: ChannelRole::Producer,
        method: Some(HttpMethod::Any),
        streaming,
    })
}

fn file_imports_proto_pkg(file_ctx: &FileContext) -> bool {
    file_ctx.imports.iter().any(|imp| {
        let m = imp
            .module_path
            .as_deref()
            .unwrap_or(imp.imported_name.as_str());
        // Only count generated-proto packages. `google.golang.org/grpc` is
        // intentionally excluded — it's the plumbing, not the service.
        let last_seg = m.rsplit('/').next().unwrap_or(m);
        m.contains("/proto/")
            || m.contains("/pb/")
            || m.contains("/protobuf/")
            || last_seg.ends_with("pb")
            || last_seg.ends_with("_pb")
            || last_seg.ends_with("Pb")
    })
}

fn is_go_stdlib_or_setup_root(name: &str) -> bool {
    matches!(
        name,
        "fmt"
            | "os"
            | "io"
            | "log"
            | "errors"
            | "context"
            | "time"
            | "sync"
            | "strings"
            | "strconv"
            | "bytes"
            | "bufio"
            | "json"
            | "yaml"
            | "http"
            | "net"
            | "url"
            | "grpc"
            | "metadata"
            | "codes"
            | "status"
            | "credentials"
            | "reflection"
            | "health"
    )
}

/// Reject leaves that don't look like gRPC RPC method names. Matches the
/// common verb prefixes for service methods (`Get`, `List`, `Create`,
/// `Update`, `Delete`, `Stream`, `Watch`, `Subscribe`, `Send`, `Publish`,
/// `Search`, `Find`, `Query`, `Mutate`, `Push`, `Pull`, `Insert`, `Remove`,
/// `Add`, `Set`, `Fetch`).
fn looks_like_grpc_method_name(name: &str) -> bool {
    if name.len() < 3 {
        return false;
    }
    let prefixes = [
        "Get", "List", "Create", "Update", "Delete", "Stream", "Watch",
        "Subscribe", "Send", "Publish", "Search", "Find", "Query", "Mutate",
        "Push", "Pull", "Insert", "Remove", "Add", "Set", "Fetch", "Run",
        "Exec", "Process", "Apply", "Validate", "Authenticate", "Authorize",
        "Sync", "Replicate", "Snapshot",
    ];
    prefixes.iter().any(|p| name.starts_with(p))
}

fn is_camel_case_local(name: &str) -> bool {
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        // Single-letter lowercase identifier (`c`, `s`) — accept.
        (Some(c0), None) => c0.is_ascii_lowercase(),
        // Multi-char camelCase: lowercase first char + at least one more letter.
        (Some(c0), Some(_)) => c0.is_ascii_lowercase() && name.chars().any(|c| c.is_alphanumeric()),
        _ => false,
    }
}

fn strip_go_service_suffix(name: &str) -> String {
    for suffix in ["ServiceClient", "Client", "Service"] {
        if let Some(stripped) = name.strip_suffix(suffix) {
            if !stripped.is_empty() {
                return stripped.to_string();
            }
        }
    }
    name.to_string()
}

fn is_go_grpc_non_method_leaf(name: &str) -> bool {
    matches!(
        name,
        "Close" | "Invoke" | "NewClient" | "Dial" | "DialContext"
    )
}

// ---------------------------------------------------------------------------
// Mailer Producer — gomail + stdlib smtp
// ---------------------------------------------------------------------------

/// `gomail.NewDialer(...).DialAndSend(msg)` and `smtp.SendMail(...)`.
pub(crate) fn detect_go_mailer_emission(
    chain: &crate::types::MemberChain,
    _call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.is_empty() {
        return None;
    }
    let root = segs.first()?.name.as_str();
    let leaf = segs.last()?.name.as_str();

    // stdlib `smtp.SendMail`.
    if root == "smtp" && leaf == "SendMail" {
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::Mailer,
            name: "go.smtp".to_string(),
            role: ChannelRole::Producer,
            method: None,
        streaming: None,
        });
    }

    // gomail dialer-style chain. The chain root carries the dialer var (often
    // `d` / `dialer` / `gomail`). Match on the leaf alone — `DialAndSend`,
    // `DialAndSendMessage`, and the v2 `Send` are unambiguous for the lib.
    if matches!(leaf, "DialAndSend" | "DialAndSendContext") {
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::Mailer,
            name: "go.gomail".to_string(),
            role: ChannelRole::Producer,
            method: None,
        streaming: None,
        });
    }
    None
}

// ---------------------------------------------------------------------------
// BgJob Producer — asynq + machinery
// ---------------------------------------------------------------------------

/// `client.Enqueue(task)` from `hibiken/asynq`, `server.SendTask(...)` from
/// `RichardKnop/machinery`. Leaf-name only; the root is usually a local
/// `client` / `srv` / `enqueuer` ident.
pub(crate) fn detect_go_bgjob_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let leaf = segs.last()?.name.as_str();
    let kind_name = match leaf {
        "Enqueue" | "EnqueueContext" | "EnqueueIn" => "asynq",
        "SendTask" | "SendTaskWithContext" => "machinery",
        _ => return None,
    };
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::BgJob,
        name: format!("go.{}", kind_name),
        role: ChannelRole::Producer,
        method: None,
    streaming: None,
    })
}

// ---------------------------------------------------------------------------
// MessageQueue Producer — Kafka (sarama, kafka-go) + NATS
// ---------------------------------------------------------------------------

/// Recognise Kafka (sarama `SendMessage`, kafka-go `WriteMessages`) and NATS
/// (`nc.Publish(subject, data)`). Emits `NamedChannel { kind: MessageQueue, .. }`.
pub(crate) fn detect_go_mq_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use crate::types::CallArg;

    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let leaf = segs.last()?.name.as_str();

    // sarama / kafka-go.
    if matches!(leaf, "SendMessage" | "SendMessages" | "WriteMessages") {
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::MessageQueue,
            name: "go.kafka".to_string(),
            role: ChannelRole::Producer,
            method: None,
        streaming: None,
        });
    }
    // NATS — `nc.Publish(subject, data)`. The subject is the first string lit.
    if leaf == "Publish" {
        let subject = call_args.iter().find_map(|a| match a {
            CallArg::StringLit(s) if !s.is_empty() => Some(s.clone()),
            _ => None,
        })?;
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::MessageQueue,
            name: subject,
            role: ChannelRole::Producer,
            method: None,
        streaming: None,
        });
    }
    None
}

// ---------------------------------------------------------------------------
// Redis ConfigLookup — go-redis Get
// ---------------------------------------------------------------------------

/// `rdb.Get(ctx, "key")` / `rdb.Get(ctx, "key").Result()` from `go-redis`.
/// Emits `ConfigLookup { key: "redis:KEY" }` so cache lookups cluster with
/// config-key reads.
pub(crate) fn detect_go_redis_config_lookup(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use crate::types::CallArg;

    let segs = &chain.segments;
    let leaf = segs.last()?.name.as_str();
    if leaf != "Get" && leaf != "GetEx" {
        return None;
    }
    // go-redis convention: first arg is `ctx`, second is the key string.
    let key = match (call_args.first(), call_args.get(1)) {
        (Some(CallArg::Ident(c)), Some(CallArg::StringLit(k)))
            if matches!(c.as_str(), "ctx" | "context" | "c" | "rctx") =>
        {
            k.clone()
        }
        _ => return None,
    };
    if key.is_empty() {
        return None;
    }
    Some(FlowEmission::ConfigLookup {
        key: format!("redis:{}", key),
    })
}

// ---------------------------------------------------------------------------
// IPC — Unix domain socket Listen / Dial
// ---------------------------------------------------------------------------

/// gorilla/websocket `upgrader.Upgrade(w, r, nil)` and
/// nhooyr/websocket `websocket.Accept(w, r, opts)`. Single-ended Consumer.
pub(crate) fn detect_go_gorilla_ws_consumer(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    let matches_gorilla = matches!(root, "upgrader" | "Upgrader") && leaf == "Upgrade";
    let matches_nhooyr = root == "websocket" && leaf == "Accept";
    if !matches_gorilla && !matches_nhooyr {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::WebSocket,
        name: if matches_gorilla {
            "go.gorilla.ws"
        } else {
            "go.nhooyr.ws"
        }
        .to_string(),
        role: ChannelRole::Consumer,
        method: None,
    streaming: None,
    })
}

/// `net.Listen("unix", path)` → IpcCall Consumer keyed on the path.
/// `net.Dial("unix", path)` → IpcCall Producer keyed on the path.
pub(crate) fn detect_go_uds_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use crate::types::CallArg;

    let segs = &chain.segments;
    if segs.len() != 2 {
        return None;
    }
    let root = segs.first()?.name.as_str();
    let leaf = segs.last()?.name.as_str();
    if root != "net" {
        return None;
    }
    let role = match leaf {
        "Listen" | "ListenUnix" => ChannelRole::Consumer,
        "Dial" | "DialUnix" => ChannelRole::Producer,
        _ => return None,
    };
    let (network_arg, path_arg) = match (call_args.first(), call_args.get(1)) {
        (Some(CallArg::StringLit(n)), Some(CallArg::StringLit(p))) => (n.as_str(), p.as_str()),
        _ => return None,
    };
    if !network_arg.starts_with("unix") {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::IpcCall,
        name: path_arg.to_string(),
        role,
        method: None,
    streaming: None,
    })
}

impl GoResolver {
    /// Resolve a package-qualified call using the chain's first segment as the
    /// import alias. For `gin.Default()` with chain `["gin", "Default"]`, find
    /// the import whose alias is "gin", derive the package name from its path,
    /// and look up `{package_name}.{target}`.
    fn resolve_via_import_alias(
        &self,
        file_ctx: &FileContext,
        alias: &str,
        target: &str,
        edge_kind: EdgeKind,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        for import in &file_ctx.imports {
            let Some(full_path) = &import.module_path else {
                continue;
            };

            // Match the alias: explicit alias if set, otherwise last path segment.
            let import_alias = import
                .alias
                .as_deref()
                .unwrap_or_else(|| full_path.rsplit('/').next().unwrap_or(full_path.as_str()));

            if import_alias != alias {
                continue;
            }

            // Found the matching import. The Go package name is conventionally
            // the last segment of the import path.
            let pkg_name = full_path.rsplit('/').next().unwrap_or(full_path.as_str());
            let candidate = format!("{pkg_name}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "go_chain_import",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }

            // Also try alias-based QN if alias differs from pkg_name.
            if alias != pkg_name {
                let candidate = format!("{alias}.{target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "go_chain_import",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }

            // Matched the import but couldn't find the symbol — don't try other imports.
            break;
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Private helpers (file-local)
// ---------------------------------------------------------------------------

/// Extract the Go package name from a parsed file.
///
/// Strategy: look at the scope_path of the first non-import symbol.
/// The extractor sets scope_path = Some(package_name) for all top-level symbols,
/// and qualified_name = "package.Name" for all symbols. We take the first segment
/// of any symbol's qualified_name.
fn extract_package_name(file: &ParsedFile) -> Option<String> {
    for sym in &file.symbols {
        // The qualified_name is "pkg.Name" — take everything before the first dot.
        if let Some(dot) = sym.qualified_name.find('.') {
            let pkg = &sym.qualified_name[..dot];
            if !pkg.is_empty() {
                return Some(pkg.to_string());
            }
        }
        // If no dot (bare name with empty prefix), fall back to scope_path.
        if let Some(ref sp) = sym.scope_path {
            if !sp.is_empty() {
                return Some(sp.split('.').next().unwrap_or(sp.as_str()).to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Check whether a Go import path is external to the project, using the go.mod manifest.
///
/// Returns `false` when the path matches (or is a sub-package of) the project's own
/// module path. Falls back to the dot-in-host heuristic when no GoMod manifest is
/// available.
pub(crate) fn is_manifest_go_external(ctx: &ProjectContext, import_path: &str) -> bool {
    let module_path: Option<&str> = ctx
        .manifest(ManifestKind::GoMod)
        .and_then(|m| m.module_path.as_deref());

    if let Some(module_path) = module_path {
        if import_path == module_path {
            return false;
        }
        if import_path.starts_with(module_path)
            && import_path.len() > module_path.len()
            && import_path.as_bytes()[module_path.len()] == b'/'
        {
            return false;
        }
        return true;
    }
    // No module path available — heuristic: dot in first segment = third-party host.
    let first_segment = import_path.split('/').next().unwrap_or(import_path);
    first_segment.contains('.')
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

