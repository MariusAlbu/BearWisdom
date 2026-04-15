// .NET — NuGet global packages cache + DLL metadata reader

use super::{ExternalDepRoot, ExternalSourceLocator};
use std::path::{Path, PathBuf};
use tracing::debug;

/// NuGet global cache → `parse_dotnet_externals`. .NET is the metadata-only
/// path: DLLs are parsed by dotscope and emitted as synthetic `ParsedFile`
/// entries, bypassing the walk-and-extract pipeline.
pub struct DotNetExternalsLocator;

impl ExternalSourceLocator for DotNetExternalsLocator {
    fn ecosystem(&self) -> &'static str { "dotnet" }

    fn parse_metadata_only(&self, project_root: &Path) -> Option<Vec<crate::types::ParsedFile>> {
        let parsed = parse_dotnet_externals(project_root);
        if parsed.is_empty() {
            None
        } else {
            Some(parsed)
        }
    }
}

/// A parsed .NET external source: a synthetic `ParsedFile` built from
/// a DLL's ECMA-335 metadata, ready to merge into the index.
///
/// Unlike Go/Python/TS/Java, .NET externals don't walk source files.
/// DLLs carry metadata but no source. `parse_dotnet_externals` uses
/// `dotscope` to enumerate types + methods directly and emits one
/// `ParsedFile` per DLL with one `ExtractedSymbol` per type/method.
///
/// The returned files have:
/// - `path`   : `ext:dotnet:{package_id}/{tfm}/{assembly_name}`
/// - `language`: `csharp` (so CLI search still matches by language filter)
/// - `symbols`: class/interface/struct/enum symbols from `types()`,
///              plus method symbols with `qualified_name = namespace.type.method`
pub fn parse_dotnet_externals(project_root: &Path) -> Vec<crate::types::ParsedFile> {
    use crate::indexer::manifest::nuget::parse_package_references_full;

    // Walk the project for .csproj / .fsproj / .vbproj and collect coords.
    let mut project_files: Vec<PathBuf> = Vec::new();
    collect_dotnet_project_files(project_root, &mut project_files, 0);
    if project_files.is_empty() {
        return Vec::new();
    }

    let mut coords: Vec<crate::indexer::manifest::nuget::NuGetCoord> = Vec::new();
    for p in &project_files {
        let Ok(content) = std::fs::read_to_string(p) else {
            continue;
        };
        coords.extend(parse_package_references_full(&content));
    }

    // Augment with transitive dependencies from `.deps.json`. The dotnet
    // SDK emits one per project under bin/{config}/{tfm}/{project}.deps.json
    // after `dotnet build`. It enumerates every assembly loaded at runtime,
    // including transitives that `.csproj` only declares indirectly
    // (`Microsoft.Extensions.Hosting` pulls in 30+ packages). This augments
    // the direct list without walking the whole NuGet cache.
    //
    // De-dup happens later at the dll_path level — reading the same package
    // declared as both a direct dep and a transitive is cheap because the
    // `seen` set in the main loop catches it.
    for p in &project_files {
        if let Some(proj_dir) = p.parent() {
            coords.extend(collect_transitive_coords_from_deps_json(proj_dir));
        }
    }

    if coords.is_empty() {
        return Vec::new();
    }

    let Some(nuget_root) = nuget_packages_root() else {
        debug!("No NuGet packages cache discovered; skipping .NET externals");
        return Vec::new();
    };

    debug!(
        "Probing NuGet cache {} for {} package references",
        nuget_root.display(),
        coords.len()
    );

    // Language tag from the project file type: VB and F# call sites
    // still see .NET metadata through the same DLL, but CLI language
    // filters and per-language stats should attribute the symbols to
    // the caller's source language.
    let lang_id = dominant_dotnet_language(&project_files);

    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for coord in coords {
        let Some(dll_path) = resolve_nuget_dll(&nuget_root, &coord) else {
            continue;
        };
        if !seen.insert(dll_path.clone()) {
            continue;
        }
        match parse_dotnet_dll(&dll_path, &coord.name, lang_id) {
            Ok(pf) => out.push(pf),
            Err(e) => debug!(
                "Failed to read .NET metadata from {}: {e}",
                dll_path.display()
            ),
        }
    }
    out
}

/// Collect transitive NuGet dependencies by reading `.deps.json` files
/// emitted under `{proj_dir}/bin/{config}/{tfm}/`. Each runtime library
/// listed with `"type": "package"` becomes a `NuGetCoord` so the main
/// externals pass can resolve its DLL in the global packages cache.
///
/// Returns an empty vector when no build output exists — that's the
/// expected state on a fresh checkout and the direct-dep pass in the
/// caller handles the common case fine. The transitive augmentation
/// only kicks in when the user has actually built their project at
/// least once.
///
/// Scans at most 16 deps.json files per project to avoid pathological
/// matrix TFM builds inflating the coord list. In the overwhelmingly
/// common single-TFM case this cap is irrelevant.
fn collect_transitive_coords_from_deps_json(
    proj_dir: &Path,
) -> Vec<crate::indexer::manifest::nuget::NuGetCoord> {
    let mut deps_json_files: Vec<PathBuf> = Vec::new();
    collect_deps_json(&proj_dir.join("bin"), &mut deps_json_files, 0);
    if deps_json_files.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for path in deps_json_files.iter().take(16) {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };
        // The `libraries` map is keyed by `{name}/{version}` and each
        // entry carries a `type` field. We want `type == "package"`
        // entries — local projects (`type == "project"`) and reference
        // assemblies (`type == "referenceassembly"`) aren't NuGet-cached.
        let Some(libs) = json.get("libraries").and_then(|v| v.as_object()) else {
            continue;
        };
        for (key, value) in libs {
            let ty = value
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("");
            if ty != "package" {
                continue;
            }
            let Some((name, version)) = key.rsplit_once('/') else {
                continue;
            };
            if !seen.insert(key.clone()) {
                continue;
            }
            out.push(crate::indexer::manifest::nuget::NuGetCoord {
                name: name.to_string(),
                version: Some(version.to_string()),
            });
        }
    }
    out
}

/// Walk a `bin/` tree collecting every `*.deps.json` file. Bounded
/// depth to avoid accidental traversal outside the build output. Skips
/// `obj/` and `runtimes/` to stay focused on the actual TFM outputs.
fn collect_deps_json(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 5 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(name, "obj" | "runtimes" | "ref") {
                        continue;
                    }
                }
                collect_deps_json(&path, out, depth + 1);
            } else if ft.is_file() {
                if path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(".deps.json"))
                {
                    out.push(path);
                }
            }
        }
    }
}

/// Determine the language tag to attribute external .NET symbols to.
/// Scans the project files found in the consumer tree and picks the
/// most common extension: `csharp` for `.csproj`, `fsharp` for `.fsproj`,
/// `vb` for `.vbproj`. If the project is a mix, C# wins — it's by far
/// the most common language and downstream search defaults to it.
fn dominant_dotnet_language(project_files: &[PathBuf]) -> &'static str {
    let mut cs = 0usize;
    let mut fs = 0usize;
    let mut vb = 0usize;
    for p in project_files {
        match p.extension().and_then(|e| e.to_str()) {
            Some("csproj") => cs += 1,
            Some("fsproj") => fs += 1,
            Some("vbproj") => vb += 1,
            _ => {}
        }
    }
    // C# is the default tiebreaker — it's the overwhelming majority on
    // NuGet and in the .NET ecosystem at large.
    if cs >= fs && cs >= vb {
        "csharp"
    } else if fs >= vb {
        "fsharp"
    } else {
        "vb"
    }
}

/// Locate the NuGet global packages folder in this order:
/// `BEARWISDOM_NUGET_PACKAGES` env override → `NUGET_PACKAGES` env →
/// `$HOME/.nuget/packages` (or `%USERPROFILE%\.nuget\packages` on Windows).
pub fn nuget_packages_root() -> Option<PathBuf> {
    for key in ["BEARWISDOM_NUGET_PACKAGES", "NUGET_PACKAGES"] {
        if let Some(raw) = std::env::var_os(key) {
            let p = PathBuf::from(raw);
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let candidate = PathBuf::from(home).join(".nuget").join("packages");
    if candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}

/// Resolve `{nuget_root}/{pkg}/{version}/lib/{tfm}/{pkg}.dll` for a coord.
///
/// The NuGet package folder is lowercase on disk. Inside, version dirs are
/// the concrete version strings; we prefer the caller's declared version
/// but fall back to the lexicographically largest when it's missing or
/// when the declared version isn't on disk.
///
/// Inside `lib/`, there may be multiple target frameworks. We prefer in
/// order: `net9.0`, `net8.0`, `net7.0`, `net6.0`, `netstandard2.1`,
/// `netstandard2.0` — newer frameworks tend to have more surface area.
/// If none of these are present, fall back to the lexicographically
/// largest subdirectory.
fn resolve_nuget_dll(
    nuget_root: &Path,
    coord: &crate::indexer::manifest::nuget::NuGetCoord,
) -> Option<PathBuf> {
    let pkg_dir = nuget_root.join(coord.name.to_lowercase());
    if !pkg_dir.is_dir() {
        return None;
    }

    let version = if let Some(v) = &coord.version {
        let concrete = pkg_dir.join(v);
        if concrete.is_dir() {
            v.clone()
        } else {
            largest_version_subdir(&pkg_dir)?
        }
    } else {
        largest_version_subdir(&pkg_dir)?
    };

    let version_dir = pkg_dir.join(&version);
    let lib_dir = version_dir.join("lib");
    if !lib_dir.is_dir() {
        return None;
    }

    let preferred_tfms = [
        "net9.0",
        "net8.0",
        "net7.0",
        "net6.0",
        "netstandard2.1",
        "netstandard2.0",
    ];
    let mut chosen_tfm: Option<PathBuf> = None;
    for tfm in preferred_tfms {
        let candidate = lib_dir.join(tfm);
        if candidate.is_dir() {
            chosen_tfm = Some(candidate);
            break;
        }
    }
    let tfm_dir = chosen_tfm.or_else(|| largest_subdir(&lib_dir))?;

    // The DLL filename matches the package name (case-insensitive). Scan
    // for a `.dll` that matches instead of guessing exact case.
    let entries = std::fs::read_dir(&tfm_dir).ok()?;
    let target_lower = coord.name.to_lowercase() + ".dll";
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name == target_lower {
            return Some(entry.path());
        }
    }
    None
}

/// Pick the lexicographically largest subdirectory name — a crude stand-in
/// for semver ordering that's good enough for finding any cached version.
fn largest_version_subdir(dir: &Path) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut versions: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            if e.file_type().ok()?.is_dir() {
                e.file_name().into_string().ok()
            } else {
                None
            }
        })
        .collect();
    versions.sort();
    versions.into_iter().next_back()
}

fn largest_subdir(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subs: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|e| {
            if e.file_type().ok()?.is_dir() {
                Some(e.path())
            } else {
                None
            }
        })
        .collect();
    subs.sort();
    subs.into_iter().next_back()
}

/// Parse a single .NET DLL and emit a synthetic `ParsedFile` with one
/// symbol per type (`Class` / `Interface` / `Struct` / `Enum`) and one
/// symbol per method. Signatures include the type's generic parameters
/// and the method's parameter/return types, with ECMA-335 placeholder
/// indices (`!0`, `!!0`) substituted back to the real parameter names
/// from the GenericParam metadata tables so the resolver's generic-param
/// classifier fires for C# externals the same way it does for TS after S6.
///
/// Per-type method iteration: methods are read via `type_def.methods`
/// (weak refs upgraded lazily) rather than the global `assembly.methods()`
/// + `declaring_type_fullname()` lookup. This gives direct attribution
/// without the per-method fullname formatting work that S7 paid.
///
/// Public surface only — types with non-public visibility and methods
/// with non-public visibility are skipped. Compiler-generated types
/// (`<>c`, `<PrivateImplementationDetails>`, `<Module>`) are filtered to
/// avoid polluting the index with noise no user code can reference.
///
/// The `lang_id` caller-chosen language tag is propagated onto the
/// synthetic `ParsedFile`; callers pick it based on whether the owning
/// project was a .csproj (`csharp`), .fsproj (`fsharp`), or .vbproj
/// (`vb`). The DLL itself is the same metadata format regardless — only
/// the display language differs.
fn parse_dotnet_dll(
    dll_path: &Path,
    package_name: &str,
    lang_id: &str,
) -> std::result::Result<crate::types::ParsedFile, String> {
    use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind};
    use dotscope::metadata::method::MethodAccessFlags;
    use dotscope::prelude::CilObject;

    let assembly = CilObject::from_path(dll_path).map_err(|e| e.to_string())?;

    let assembly_name = assembly
        .assembly()
        .map(|a| a.name.clone())
        .unwrap_or_else(|| package_name.to_string());

    let virtual_path = format!("ext:dotnet:{}/{}", package_name, assembly_name);
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();

    for type_def in assembly.types().all_types().iter() {
        let name = type_def.name.clone();
        let namespace = type_def.namespace.clone();

        // Skip compiler-generated types. These have names like `<>c`,
        // `<PrivateImplementationDetails>`, `<Module>` and inflate the
        // symbol table with noise no user code can reference.
        if name.starts_with('<') || name == "<Module>" {
            continue;
        }

        // Skip non-public types — public API surface only.
        // TypeAttributes.VisibilityMask = 0x07
        let visibility_mask = type_def.flags & 0x07;
        if visibility_mask != 1 && visibility_mask != 2 {
            // 1 = Public, 2 = NestedPublic; everything else is private/internal.
            continue;
        }

        // Interface flag = TypeAttributes.ClassSemanticsMask & 0x20
        let is_interface = type_def.flags & 0x20 != 0;
        let kind = if is_interface {
            SymbolKind::Interface
        } else {
            SymbolKind::Class
        };

        // Strip the ECMA-335 backtick-arity suffix (`Repository\`1` → `Repository`)
        // so user code that references `Repository<User>` resolves to the
        // right symbol. The arity is reflected in the generic_params vec.
        let display_name = strip_backtick_arity(&name);
        let qualified_name = if namespace.is_empty() {
            display_name.to_string()
        } else {
            format!("{namespace}.{display_name}")
        };

        // Build the real `<T, U>` suffix from the GenericParam table
        // rather than making up `<T1, T2, ...>` from the backtick count.
        let type_generic_names: Vec<String> = type_def
            .generic_params
            .iter()
            .map(|(_, gp)| gp.name.clone())
            .collect();
        let type_gp_suffix = format_generic_suffix(&type_generic_names);

        symbols.push(ExtractedSymbol {
            name: display_name.to_string(),
            qualified_name: qualified_name.clone(),
            kind,
            visibility: Some(crate::types::Visibility::Public),
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            signature: Some(format!(
                "{} {}{}",
                if is_interface { "interface" } else { "class" },
                display_name,
                type_gp_suffix
            )),
            doc_comment: None,
            scope_path: if namespace.is_empty() {
                None
            } else {
                Some(namespace.clone())
            },
            parent_index: None,
        });

        // Per-type method iteration: walk type_def.methods directly so we
        // get method-to-type attribution for free and avoid a second pass
        // over the global method map. `boxcar::Vec` yields `(usize, &T)`
        // tuples; we only care about the ref.
        for (_, method_ref) in type_def.methods.iter() {
            let Some(method) = method_ref.upgrade() else {
                continue;
            };

            // Skip compiler-generated accessors and lifecycle methods:
            // - `get_X` / `set_X` / `add_X` / `remove_X` (property/event accessors)
            // - `.ctor` / `.cctor` (constructors emit as Constructor symbols elsewhere)
            // - `<...>` anonymous/closure methods
            if method.name.starts_with('<') || method.name.starts_with('.') {
                continue;
            }
            // Public surface only.
            if method.flags_access != MethodAccessFlags::PUBLIC {
                continue;
            }

            let method_name = method.name.clone();
            let method_qname = format!("{qualified_name}.{method_name}");

            // Collect the method's own generic param names so we can
            // splice them into the signature and substitute `!!N`
            // placeholders back to real names.
            let method_generic_names: Vec<String> = method
                .generic_params
                .iter()
                .map(|(_, gp)| gp.name.clone())
                .collect();

            let signature = format_method_signature(
                &method_name,
                &method.signature,
                &type_generic_names,
                &method_generic_names,
                &assembly,
            );

            symbols.push(ExtractedSymbol {
                name: method_name,
                qualified_name: method_qname,
                kind: SymbolKind::Method,
                visibility: Some(crate::types::Visibility::Public),
                start_line: 0,
                end_line: 0,
                start_col: 0,
                end_col: 0,
                signature: Some(signature),
                doc_comment: None,
                scope_path: Some(qualified_name.clone()),
                parent_index: None,
            });
        }
    }

    let symbol_count = symbols.len();
    debug!(
        "Parsed {} external .NET symbols from {}",
        symbol_count,
        dll_path.display()
    );

    // Compute a content hash from the DLL bytes so incremental indexing
    // knows when to re-read. Use the file mtime + size as a cheap proxy
    // rather than hashing the whole DLL every time.
    let metadata = std::fs::metadata(dll_path).map_err(|e| e.to_string())?;
    let size = metadata.len();
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    let content_hash = format!("{:x}", size).to_string();

    Ok(crate::types::ParsedFile {
        path: virtual_path,
        language: lang_id.to_string(),
        content_hash,
        size,
        line_count: 0,
        mtime,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
    })
}

/// Strip the ECMA-335 backtick arity suffix from a type name.
///
/// `Repository\`1` → `Repository`, `Dictionary\`2` → `Dictionary`,
/// `Func\`4` → `Func`. Left-idempotent on names without a backtick.
/// This is the surface name users write in source — the arity is
/// reflected separately in the `generic_params` collection.
fn strip_backtick_arity(name: &str) -> &str {
    match name.find('`') {
        Some(idx) => &name[..idx],
        None => name,
    }
}

/// Format a list of generic parameter names as `<A, B, C>` or empty if
/// the list is empty. Kept as a helper so the type and method signature
/// builders stay readable.
fn format_generic_suffix(names: &[String]) -> String {
    if names.is_empty() {
        String::new()
    } else {
        format!("<{}>", names.join(", "))
    }
}

/// Format a method signature in a shape the resolver's generic-param
/// classifier and chain walker can parse. The classifier scans for
/// `<...>` at the top level of a signature string and splits on commas
/// to extract parameter names; the chain walker reads the return type
/// portion after the `:` separator.
///
/// Shape: `{method_name}<U, V>(Param1, Param2): ReturnType`
///
/// Parameter and return type strings get two post-processing passes:
/// 1. ECMA-335 placeholder substitution (`!N` → type param, `!!N` → method param)
/// 2. Metadata-token resolution: `class[00000042]` and `valuetype[00000042]`
///    → `Namespace.TypeName` via a `TypeRegistry` lookup.
///
/// Nested `GenericInst(class[…], args)` becomes `TypeName<T, U>` in one
/// pass — Display renders `class[…]<T, U>` and the token substitution
/// rewrites the leading `class[…]` to `TypeName` without touching the
/// already-valid generic argument list.
fn format_method_signature(
    method_name: &str,
    sig: &dotscope::metadata::signatures::SignatureMethod,
    type_generic_names: &[String],
    method_generic_names: &[String],
    assembly: &dotscope::prelude::CilObject,
) -> String {
    let gp_suffix = format_generic_suffix(method_generic_names);

    let mut params_str = String::from("(");
    for (i, p) in sig.params.iter().enumerate() {
        if i > 0 {
            params_str.push_str(", ");
        }
        let rendered = format!("{}", p);
        let substituted = substitute_generic_placeholders(
            &rendered,
            type_generic_names,
            method_generic_names,
        );
        params_str.push_str(&resolve_signature_tokens(&substituted, assembly));
    }
    params_str.push(')');

    let return_rendered = format!("{}", sig.return_type);
    let return_substituted = substitute_generic_placeholders(
        &return_rendered,
        type_generic_names,
        method_generic_names,
    );
    let return_str = resolve_signature_tokens(&return_substituted, assembly);

    format!("{method_name}{gp_suffix}{params_str}: {return_str}")
}

/// Replace ECMA-335 `class[HHHHHHHH]` / `valuetype[HHHHHHHH]` token
/// placeholders with their resolved `Namespace.TypeName`. Tries both
/// metadata-table sources:
///
/// - **TypeDef** (token high byte `0x02`): defined in the current
///   assembly, looked up via `assembly.types()` (a `TypeRegistry`).
/// - **TypeRef** (token high byte `0x01`): references to types in
///   other assemblies (`System.String`, `System.Threading.Tasks.Task`,
///   `Microsoft.Extensions.Logging.ILogger`, etc.), looked up via
///   `assembly.imports()`. Most nested type arguments in real .NET
///   signatures fall into this bucket — they reference types defined
///   in the BCL or other dependency assemblies.
///
/// Leaves unresolvable tokens as-is so the signature still renders and
/// the top-level `<...>` region stays parseable by downstream code.
/// `dotscope`'s `TypeSignature::Display` emits tokens as upper-case
/// 8-hex-digit values wrapped in square brackets; we scan for both
/// prefixes, parse the hex, select the right lookup via the token's
/// high byte, and splice the result back in.
fn resolve_signature_tokens(
    rendered: &str,
    assembly: &dotscope::prelude::CilObject,
) -> String {
    use dotscope::metadata::token::Token;

    let type_registry = assembly.types();
    let imports = assembly.imports().cil();

    let mut out = String::with_capacity(rendered.len());
    let bytes = rendered.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let remaining = &rendered[i..];
        let (prefix_len, skip_prefix) = if remaining.starts_with("class[") {
            (6, true)
        } else if remaining.starts_with("valuetype[") {
            (10, true)
        } else {
            (0, false)
        };
        if skip_prefix {
            // Scan for closing bracket.
            let after_prefix = &remaining[prefix_len..];
            if let Some(close_rel) = after_prefix.find(']') {
                let hex = &after_prefix[..close_rel];
                if let Ok(value) = u32::from_str_radix(hex, 16) {
                    let token = Token::new(value);
                    // High byte selects the metadata table:
                    //   0x02 = TypeDef  (current assembly)
                    //   0x01 = TypeRef  (external assemblies — BCL etc.)
                    //   0x1B = TypeSpec (generic instantiations, not handled here)
                    let table_byte = value >> 24;
                    let resolved: Option<String> = match table_byte {
                        0x02 => type_registry.get(&token).map(|ty| {
                            let name = strip_backtick_arity(&ty.name).to_string();
                            if ty.namespace.is_empty() {
                                name
                            } else {
                                format!("{}.{}", ty.namespace, name)
                            }
                        }),
                        0x01 => imports.get(token).map(|imp| {
                            let name = strip_backtick_arity(&imp.name).to_string();
                            if imp.namespace.is_empty() {
                                name
                            } else {
                                format!("{}.{}", imp.namespace, name)
                            }
                        }),
                        _ => None,
                    };
                    if let Some(full) = resolved {
                        out.push_str(&full);
                        i += prefix_len + close_rel + 1;
                        continue;
                    }
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Replace ECMA-335 generic parameter placeholders with their real names.
///
/// `!0` → first type generic parameter (e.g., `T`)
/// `!!0` → first method generic parameter (e.g., `U`)
///
/// Scans left-to-right, handling multi-digit indices (`!10`, `!!10`).
/// Unknown indices are left as-is so the signature still renders but
/// unrecognised generic params don't crash the formatter. Method-level
/// `!!N` must be checked BEFORE type-level `!N` because `!!` would
/// otherwise be consumed as two separate `!0` matches.
fn substitute_generic_placeholders(
    rendered: &str,
    type_gen: &[String],
    method_gen: &[String],
) -> String {
    let bytes = rendered.as_bytes();
    let mut out = String::with_capacity(rendered.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'!' {
            let is_method = i + 1 < bytes.len() && bytes[i + 1] == b'!';
            let num_start = if is_method { i + 2 } else { i + 1 };
            let mut num_end = num_start;
            while num_end < bytes.len() && bytes[num_end].is_ascii_digit() {
                num_end += 1;
            }
            if num_end > num_start {
                let idx: usize = rendered[num_start..num_end].parse().unwrap_or(usize::MAX);
                let target = if is_method { method_gen } else { type_gen };
                if let Some(name) = target.get(idx) {
                    out.push_str(name);
                    i = num_end;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn collect_dotnet_project_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 10 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(
                        name,
                        "bin" | "obj" | "node_modules" | ".git" | "target"
                            | "packages" | ".vs" | "TestResults" | "artifacts"
                    ) {
                        continue;
                    }
                }
                collect_dotnet_project_files(&path, out, depth + 1);
            } else if ft.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if matches!(ext, "csproj" | "fsproj" | "vbproj") {
                        out.push(path);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_backtick_arity_removes_generic_suffix() {
        assert_eq!(strip_backtick_arity("Repository`1"), "Repository");
        assert_eq!(strip_backtick_arity("Dictionary`2"), "Dictionary");
        assert_eq!(strip_backtick_arity("Func`4"), "Func");
        assert_eq!(strip_backtick_arity("List"), "List");
        assert_eq!(strip_backtick_arity(""), "");
    }

    #[test]
    fn format_generic_suffix_joins_names() {
        assert_eq!(format_generic_suffix(&[]), "");
        assert_eq!(
            format_generic_suffix(&["T".to_string()]),
            "<T>"
        );
        assert_eq!(
            format_generic_suffix(&["T".to_string(), "U".to_string()]),
            "<T, U>"
        );
    }

    #[test]
    fn substitute_placeholders_swaps_ecma335_syntax() {
        let type_gen = vec!["T".to_string()];
        let method_gen = vec!["U".to_string(), "V".to_string()];

        // Method-level placeholder.
        assert_eq!(
            substitute_generic_placeholders("!!0", &type_gen, &method_gen),
            "U"
        );
        assert_eq!(
            substitute_generic_placeholders("!!1", &type_gen, &method_gen),
            "V"
        );
        // Type-level placeholder.
        assert_eq!(
            substitute_generic_placeholders("!0", &type_gen, &method_gen),
            "T"
        );
        // Mixed inside a call-site signature.
        assert_eq!(
            substitute_generic_placeholders(
                "Func<!0, !!0, !!1>",
                &type_gen,
                &method_gen
            ),
            "Func<T, U, V>"
        );
        // Out-of-range index is left alone.
        assert_eq!(
            substitute_generic_placeholders("!!5", &type_gen, &method_gen),
            "!!5"
        );
    }

    #[test]
    fn substitute_placeholders_multi_digit_indices() {
        let method_gen: Vec<String> = (0..15).map(|i| format!("T{i}")).collect();
        assert_eq!(
            substitute_generic_placeholders("!!10", &[], &method_gen),
            "T10"
        );
        assert_eq!(
            substitute_generic_placeholders("!!14", &[], &method_gen),
            "T14"
        );
    }
}
