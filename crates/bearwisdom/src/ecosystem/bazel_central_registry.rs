// =============================================================================
// ecosystem/bazel_central_registry.rs — Bazel / BCR ecosystem
//
// Covers both bzlmod (MODULE.bazel) and legacy WORKSPACE-based projects.
// Discovers external dep roots from the Bazel output-base external/ directory
// and the project-local bazel-<name>/external/ symlink, then walks .bzl,
// BUILD, and BUILD.bazel files for indexing.
//
// Synthetic symbols are emitted for the Bazel native built-in rules (cc_*,
// java_*, py_*, genrule, …) which are implemented in Java and have no .bzl
// source on disk, using the virtual path `ext:bazel-builtins:rules.bzl`.
//
// Round 3: also emits the analysistest `env` assertion API at
// `ext:bazel-builtins:env.bzl` so chain walkers produce real resolved edges
// for env.expect.that_str / that_collection / that_int / that_bool patterns.
//
// Activation: Any([ManifestMatch, LanguagePresent("starlark")]).
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

pub const ID: EcosystemId = EcosystemId::new("bazel-central-registry");
const LEGACY_ECOSYSTEM_TAG: &str = "bazel-central-registry";
const LANGUAGES: &[&str] = &["starlark"];

// ---------------------------------------------------------------------------
// Manifest specs
// ---------------------------------------------------------------------------

const MANIFESTS: &[ManifestSpec] = &[
    ManifestSpec {
        glob: "**/MODULE.bazel",
        parse: parse_module_bazel,
    },
    ManifestSpec {
        glob: "**/WORKSPACE{,.bazel}",
        parse: parse_workspace,
    },
];

fn parse_module_bazel(path: &Path) -> std::io::Result<crate::ecosystem::manifest::ManifestData> {
    use crate::ecosystem::manifest::ManifestData;
    let content = std::fs::read_to_string(path)?;
    let deps = extract_bzlmod_deps(&content);
    let mut data = ManifestData::default();
    data.dependencies = deps.into_iter().collect();
    Ok(data)
}

fn parse_workspace(path: &Path) -> std::io::Result<crate::ecosystem::manifest::ManifestData> {
    use crate::ecosystem::manifest::ManifestData;
    let content = std::fs::read_to_string(path)?;
    let deps = extract_workspace_deps(&content);
    let mut data = ManifestData::default();
    data.dependencies = deps.into_iter().collect();
    Ok(data)
}

// ---------------------------------------------------------------------------
// Ecosystem impl
// ---------------------------------------------------------------------------

pub struct BazelCentralRegistryEcosystem;

impl Ecosystem for BazelCentralRegistryEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("starlark"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_bazel_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_bazel_root(dep)
    }

    /// Emit synthetic `ParsedFile` entries for Bazel built-in rules, the
    /// `ctx` / `repository_ctx` API, and the `env` / analysistest assertion
    /// chain API. Returned unconditionally so the resolver can close native
    /// rule refs like `cc_library(...)`, ctx-chain refs, and env-chain refs.
    fn parse_metadata_only(&self, _dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        Some(vec![synth_builtin_rules(), synth_ctx_api(), synth_env_api()])
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
}

impl ExternalSourceLocator for BazelCentralRegistryEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_bazel_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_bazel_root(dep)
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        Some(vec![synth_builtin_rules(), synth_ctx_api(), synth_env_api()])
    }
}

/// Process-wide shared instance.
pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<BazelCentralRegistryEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(BazelCentralRegistryEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Locate Bazel external dependency roots for a project.
///
/// Two paths are probed in order:
///   1. `<project>/bazel-<dirname>/external/` — project-local output-base
///      symlink that Bazel creates after any build/query.
///   2. `~/.cache/bazel/_bazel_<user>/<hash>/external/` (Linux) or
///      `%USERPROFILE%/_bazel_<user>/<hash>/external/` (Windows) — the
///      real on-disk output-base cache.
///
/// Each subdirectory under `external/` is one dependency. We emit an
/// `ExternalDepRoot` per subdirectory whose name was declared in the
/// project manifests (or all of them if we can't read the manifest).
pub fn discover_bazel_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let declared = declared_dep_names(project_root);

    let mut externals_dirs: Vec<PathBuf> = Vec::new();

    // 1. Project-local bazel-<name>/external/ symlink.
    if let Some(dir_name) = project_root.file_name().and_then(|n| n.to_str()) {
        let local_link = project_root.join(format!("bazel-{dir_name}")).join("external");
        if local_link.is_dir() {
            externals_dirs.push(local_link);
        }
        // Generic `bazel-bin` adjacent fallback — some projects use bazel-<project>.
        let plain_link = project_root.join("bazel-out").parent()
            .map(|p| p.join("external"))
            .filter(|p| p.is_dir());
        if let Some(p) = plain_link { externals_dirs.push(p); }
    }

    // 2. Global output-base cache.
    for cache_ext in find_output_base_externals() {
        externals_dirs.push(cache_ext);
    }

    if externals_dirs.is_empty() {
        debug!("BazelBCR: no external/ directories found for {}", project_root.display());
        return Vec::new();
    }

    let mut roots: Vec<ExternalDepRoot> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for ext_dir in &externals_dirs {
        let Ok(entries) = std::fs::read_dir(ext_dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() { continue; }
            let Some(dep_name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            // Skip Bazel-internal repos prefixed with `_`, `bazel_tools`, etc.
            if dep_name.starts_with('_') || dep_name == "bazel_tools" || dep_name == "local_config_cc" {
                continue;
            }
            if seen.contains(dep_name) { continue; }
            // If we parsed manifests, only include declared deps; otherwise include all.
            if !declared.is_empty() && !declared.contains(dep_name) { continue; }
            seen.insert(dep_name.to_string());
            roots.push(ExternalDepRoot {
                module_path: dep_name.to_string(),
                version: String::new(),
                root: path,
                ecosystem: LEGACY_ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: Vec::new(),
            });
        }
    }

    debug!("BazelBCR: {} external dep roots", roots.len());
    roots
}

/// Parse all manifests under `project_root` and collect the union of declared
/// dependency names (both bzlmod and WORKSPACE formats).
fn declared_dep_names(project_root: &Path) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();

    // MODULE.bazel
    let module_path = project_root.join("MODULE.bazel");
    if let Ok(content) = std::fs::read_to_string(&module_path) {
        for n in extract_bzlmod_deps(&content) { names.insert(n); }
    }

    // WORKSPACE / WORKSPACE.bazel
    for candidate in ["WORKSPACE", "WORKSPACE.bazel"] {
        let ws_path = project_root.join(candidate);
        if let Ok(content) = std::fs::read_to_string(&ws_path) {
            for n in extract_workspace_deps(&content) { names.insert(n); }
            break;
        }
    }

    names
}

/// Find `external/` directories inside the Bazel output-base cache.
///
/// Layout: `~/.cache/bazel/_bazel_<user>/<hash>/external/` (Linux/macOS)
///          `%USERPROFILE%/_bazel_<user>/<hash>/external/` (Windows)
fn find_output_base_externals() -> Vec<PathBuf> {
    let mut found = Vec::new();
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    } else {
        // ~/.cache/bazel on Linux; ~/Library/Caches/bazel on macOS is also common.
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache").join("bazel"))
    };
    let Some(cache_root) = home else { return found };

    // On Linux/macOS: $HOME/.cache/bazel/_bazel_<user>/
    // The actual directory is $HOME/.cache/bazel/_bazel_<user>/<hash>/external/
    // We walk two levels deep to find any hash directory.
    let bazel_dir = if cfg!(windows) { cache_root.join(".bazel") } else { cache_root };
    if !bazel_dir.is_dir() { return found; }

    let Ok(user_dirs) = std::fs::read_dir(&bazel_dir) else { return found };
    for user_entry in user_dirs.flatten() {
        let user_path = user_entry.path();
        if !user_path.is_dir() { continue; }
        let Ok(hash_dirs) = std::fs::read_dir(&user_path) else { continue };
        for hash_entry in hash_dirs.flatten() {
            let ext = hash_entry.path().join("external");
            if ext.is_dir() {
                found.push(ext);
            }
        }
    }
    found
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

const MAX_WALK_DEPTH: u32 = 8;

pub fn walk_bazel_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_dir_bounded(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, ".git" | "node_modules" | "__pycache__") || name.starts_with('.') {
                    continue;
                }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if ft.is_file() {
            if !is_bazel_source_file(&path) { continue; }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:bazel:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "starlark",
            });
        }
    }
}

fn is_bazel_source_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return false };
    matches!(name, "BUILD" | "BUILD.bazel") || name.ends_with(".bzl")
}

// ---------------------------------------------------------------------------
// Manifest parsing — line-regex MVP
// ---------------------------------------------------------------------------

/// Extract `bazel_dep(name = "...", ...)` entries from a MODULE.bazel file.
pub fn extract_bzlmod_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("bazel_dep(") { continue; }
        if let Some(name) = extract_kwarg(trimmed, "name") {
            if !name.is_empty() { deps.push(name); }
        }
    }
    deps
}

/// Extract `http_archive(name = "...")` and `git_repository(name = "...")`
/// entries from a legacy WORKSPACE file.
pub fn extract_workspace_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_block = false;
    let mut block_buf = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if !in_block {
            if trimmed.starts_with("http_archive(")
                || trimmed.starts_with("git_repository(")
                || trimmed.starts_with("new_git_repository(")
                || trimmed.starts_with("http_file(")
            {
                in_block = true;
                block_buf.clear();
                block_buf.push_str(trimmed);
                block_buf.push('\n');
                if trimmed.ends_with(')') {
                    if let Some(name) = extract_kwarg(&block_buf, "name") {
                        if !name.is_empty() { deps.push(name); }
                    }
                    in_block = false;
                }
            }
        } else {
            block_buf.push_str(trimmed);
            block_buf.push('\n');
            if trimmed == ")" || trimmed.ends_with(')') {
                if let Some(name) = extract_kwarg(&block_buf, "name") {
                    if !name.is_empty() { deps.push(name); }
                }
                in_block = false;
            }
        }
    }
    deps
}

/// Extract `key = "value"` from a Starlark-ish single-line or buffered block.
fn extract_kwarg(text: &str, key: &str) -> Option<String> {
    // Only match the right key= form.
    let search = format!("{key} = \"");
    let start = text.find(&search)?;
    let rest = &text[start + search.len()..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

// ---------------------------------------------------------------------------
// Synthetic built-in rules + Bazel ctx API
// ---------------------------------------------------------------------------

/// Dotted members of the Bazel rule-context object `ctx`. Each entry is the
/// qualified name under `ctx.*` (e.g., `"actions.run"` → symbol qname
/// `"ctx.actions.run"`). Emitted into `ext:bazel-builtins:ctx.bzl` so that
/// chain walkers (if ever wired for Starlark) can look up field types.
///
/// Members are grouped by sub-object: `actions.*`, `label.*`, top-level
/// attributes (`attr`, `file`, `files`, `outputs`, `executable`, etc.).
const CTX_MEMBERS: &[(&str, &str, &str)] = &[
    // (short_name, qualified_name, signature)
    ("actions", "ctx.actions", "ctx.actions"),
    ("actions.run", "ctx.actions.run", "ctx.actions.run(outputs, inputs, executable, arguments=[], **kwargs)"),
    ("actions.run_shell", "ctx.actions.run_shell", "ctx.actions.run_shell(outputs, inputs=[], command, **kwargs)"),
    ("actions.declare_file", "ctx.actions.declare_file", "ctx.actions.declare_file(filename, sibling=None) -> File"),
    ("actions.declare_directory", "ctx.actions.declare_directory", "ctx.actions.declare_directory(name, sibling=None) -> File"),
    ("actions.write", "ctx.actions.write", "ctx.actions.write(output, content, is_executable=False)"),
    ("actions.expand_template", "ctx.actions.expand_template", "ctx.actions.expand_template(template, output, substitutions, is_executable=False)"),
    ("actions.symlink", "ctx.actions.symlink", "ctx.actions.symlink(output, target_file=None, target_path=None, **kwargs)"),
    ("actions.args", "ctx.actions.args", "ctx.actions.args() -> Args"),
    ("label", "ctx.label", "ctx.label -> Label"),
    ("label.name", "ctx.label.name", "ctx.label.name -> string"),
    ("label.package", "ctx.label.package", "ctx.label.package -> string"),
    ("label.workspace_name", "ctx.label.workspace_name", "ctx.label.workspace_name -> string"),
    ("label.workspace_root", "ctx.label.workspace_root", "ctx.label.workspace_root -> string"),
    ("attr", "ctx.attr", "ctx.attr -> struct"),
    ("file", "ctx.file", "ctx.file -> struct"),
    ("files", "ctx.files", "ctx.files -> struct"),
    ("outputs", "ctx.outputs", "ctx.outputs -> struct"),
    ("executable", "ctx.executable", "ctx.executable -> struct"),
    ("runfiles", "ctx.runfiles", "ctx.runfiles(files=[], transitive_files=None, collect_data=False, collect_default=False) -> runfiles"),
    ("workspace_name", "ctx.workspace_name", "ctx.workspace_name -> string"),
    ("configuration", "ctx.configuration", "ctx.configuration -> configuration"),
    ("bin_dir", "ctx.bin_dir", "ctx.bin_dir -> root"),
    ("genfiles_dir", "ctx.genfiles_dir", "ctx.genfiles_dir -> root"),
    ("var", "ctx.var", "ctx.var -> dict[string, string]"),
    ("build_file_path", "ctx.build_file_path", "ctx.build_file_path -> string"),
    ("coverage_instrumented", "ctx.coverage_instrumented", "ctx.coverage_instrumented(target=None) -> bool"),
    ("expand_location", "ctx.expand_location", "ctx.expand_location(input, targets=[]) -> string"),
    ("expand_make_variables", "ctx.expand_make_variables", "ctx.expand_make_variables(attribute_name, command, additional_substitutions) -> string"),
    ("info_file", "ctx.info_file", "ctx.info_file -> File"),
    ("version_file", "ctx.version_file", "ctx.version_file -> File"),
    ("target_platform_has_constraint", "ctx.target_platform_has_constraint", "ctx.target_platform_has_constraint(constraintValue) -> bool"),
    ("toolchains", "ctx.toolchains", "ctx.toolchains -> struct"),
    ("fragments", "ctx.fragments", "ctx.fragments -> struct"),
];

/// repository_ctx members (available in `repository_rule` implementations).
const REPOSITORY_CTX_MEMBERS: &[(&str, &str, &str)] = &[
    ("execute", "repository_ctx.execute", "repository_ctx.execute(arguments, timeout=600, environment={}, **kwargs) -> exec_result"),
    ("path", "repository_ctx.path", "repository_ctx.path(path) -> path"),
    ("download", "repository_ctx.download", "repository_ctx.download(url, output, sha256='', **kwargs)"),
    ("download_and_extract", "repository_ctx.download_and_extract", "repository_ctx.download_and_extract(url, output='', sha256='', **kwargs)"),
    ("extract", "repository_ctx.extract", "repository_ctx.extract(archive, output='', stripPrefix='')"),
    ("file", "repository_ctx.file", "repository_ctx.file(path, content='', executable=True, **kwargs)"),
    ("read", "repository_ctx.read", "repository_ctx.read(path) -> string"),
    ("symlink", "repository_ctx.symlink", "repository_ctx.symlink(target, link_name)"),
    ("template", "repository_ctx.template", "repository_ctx.template(path, label, substitutions={}, executable=True)"),
    ("which", "repository_ctx.which", "repository_ctx.which(program) -> path"),
    ("workspace_root", "repository_ctx.workspace_root", "repository_ctx.workspace_root -> path"),
    ("name", "repository_ctx.name", "repository_ctx.name -> string"),
    ("attr", "repository_ctx.attr", "repository_ctx.attr -> struct"),
    ("os", "repository_ctx.os", "repository_ctx.os -> struct"),
    ("environ", "repository_ctx.environ", "repository_ctx.environ -> dict[string, string]"),
];

/// Bazel Target struct members. Targets are passed to rule implementations
/// via `ctx.attr.<name>` and accessed through `target.runfiles`,
/// `target.files`, etc. Without the synthetic, every `target.runfiles` in a
/// rule impl shows up as an unresolved Calls ref.
const TARGET_MEMBERS: &[(&str, &str, &str)] = &[
    ("label", "target.label", "target.label -> Label"),
    ("files", "target.files", "target.files -> depset[File]"),
    ("default_outputs", "target.default_outputs", "target.default_outputs -> list[File]"),
    ("data_runfiles", "target.data_runfiles", "target.data_runfiles -> runfiles"),
    ("default_runfiles", "target.default_runfiles", "target.default_runfiles -> runfiles"),
    ("runfiles", "target.runfiles", "target.runfiles -> runfiles"),
    ("attr", "target.attr", "target.attr -> struct"),
    ("provider", "target.provider", "target.provider(provider_type) -> any"),
    ("providers", "target.providers", "target.providers() -> list"),
    ("files_to_run", "target.files_to_run", "target.files_to_run -> FilesToRun"),
    ("output_groups", "target.output_groups", "target.output_groups -> struct"),
    ("aspect_ids", "target.aspect_ids", "target.aspect_ids -> list[string]"),
];

/// Bazel runfiles type members — returned by ctx.runfiles()/target.runfiles
/// and have `.merge(...)` / `.merge_all(...)` chaining methods.
const RUNFILES_MEMBERS: &[(&str, &str, &str)] = &[
    ("merge", "runfiles.merge", "runfiles.merge(other) -> runfiles"),
    ("merge_all", "runfiles.merge_all", "runfiles.merge_all(others) -> runfiles"),
    ("files", "runfiles.files", "runfiles.files -> depset[File]"),
    ("symlinks", "runfiles.symlinks", "runfiles.symlinks -> depset"),
    ("root_symlinks", "runfiles.root_symlinks", "runfiles.root_symlinks -> depset"),
    ("empty_filenames", "runfiles.empty_filenames", "runfiles.empty_filenames -> depset[string]"),
];

/// Bazel ctx.actions.args() builder type members.
const ARGS_MEMBERS: &[(&str, &str, &str)] = &[
    ("add", "args.add", "args.add(arg_name_or_value, value=None, ...) -> Args"),
    ("add_all", "args.add_all", "args.add_all(arg_name_or_values, values=None, ...) -> Args"),
    ("add_joined", "args.add_joined", "args.add_joined(arg_name_or_values, values=None, join_with, ...) -> Args"),
    ("set_param_file_format", "args.set_param_file_format", "args.set_param_file_format(format)"),
    ("use_param_file", "args.use_param_file", "args.use_param_file(param_file_arg, use_always=False)"),
];

/// Plain `entries` / `arguments` aliases for the Args type. Bazel rule
/// implementations commonly store the result of `ctx.actions.args()` in a
/// local named `args`, `arguments`, or `entries`; the chain walker sees
/// `entries.add_joined` and needs a synthetic at that exact qualified name.
const ARGS_LOCAL_ALIASES: &[(&str, &[&str])] = &[
    ("entries", &["add", "add_all", "add_joined", "set_param_file_format", "use_param_file"]),
    ("arguments", &["add", "add_all", "add_joined", "set_param_file_format", "use_param_file"]),
];

/// Bazel `attr.*` and `config.*` factory functions used in rule attribute
/// declarations. `attr.label_list(...)`, `config.exec(...)`, etc.
const ATTR_FACTORIES: &[(&str, &str, &str)] = &[
    ("label", "attr.label", "attr.label(...) -> attr"),
    ("label_list", "attr.label_list", "attr.label_list(...) -> attr"),
    ("label_keyed_string_dict", "attr.label_keyed_string_dict", "attr.label_keyed_string_dict(...) -> attr"),
    ("string", "attr.string", "attr.string(...) -> attr"),
    ("string_list", "attr.string_list", "attr.string_list(...) -> attr"),
    ("string_dict", "attr.string_dict", "attr.string_dict(...) -> attr"),
    ("string_list_dict", "attr.string_list_dict", "attr.string_list_dict(...) -> attr"),
    ("int", "attr.int", "attr.int(...) -> attr"),
    ("int_list", "attr.int_list", "attr.int_list(...) -> attr"),
    ("bool", "attr.bool", "attr.bool(...) -> attr"),
    ("output", "attr.output", "attr.output(...) -> attr"),
    ("output_list", "attr.output_list", "attr.output_list(...) -> attr"),
];

const CONFIG_FACTORIES: &[(&str, &str, &str)] = &[
    ("none", "config.none", "config.none() -> config"),
    ("target", "config.target", "config.target() -> config"),
    ("exec", "config.exec", "config.exec(exec_group=None) -> config"),
    ("string", "config.string", "config.string(name, default) -> config"),
    ("bool", "config.bool", "config.bool(name, default) -> config"),
    ("int", "config.int", "config.int(name, default) -> config"),
];

/// `mrctx`/`mctx` is a common shortname for `module_ctx`. Mirror the
/// repository_ctx members under each alias so chain walks like
/// `mrctx.path` resolve to the synthetic.
const MODULE_CTX_ALIASES: &[&str] = &["mctx", "mrctx", "module_ctx"];

// ---------------------------------------------------------------------------
// env / env_expect / subject types — analysistest / unittest assertion chains
// (Round 3)
// ---------------------------------------------------------------------------

/// Top-level members of the analysistest `env` object. `env.expect` returns an
/// `env_expect` value; the chain walker resolves 2-level refs (env.expect,
/// env.fail, env.assert_equals) directly against these symbols.
const ENV_MEMBERS: &[(&str, &str, &str)] = &[
    ("expect",              "env.expect",              "env.expect -> env_expect"),
    ("assert_equals",       "env.assert_equals",       "env.assert_equals(expected, actual)"),
    ("fail",                "env.fail",                "env.fail(msg)"),
    ("ctx",                 "env.ctx",                 "env.ctx -> ctx"),
    ("analysistest_target", "env.analysistest_target", "env.analysistest_target -> Target"),
];

/// Methods on the `env_expect` object (returned by `env.expect`). Each
/// `that_*` factory returns a typed subject for further assertion chaining.
///
/// Uses the `env_expect.*` type-level qname form.
const ENV_EXPECT_MEMBERS: &[(&str, &str, &str)] = &[
    ("that_str",             "env_expect.that_str",             "env_expect.that_str(value) -> env_str_subject"),
    ("that_int",             "env_expect.that_int",             "env_expect.that_int(value) -> env_int_subject"),
    ("that_bool",            "env_expect.that_bool",            "env_expect.that_bool(value) -> env_bool_subject"),
    ("that_collection",      "env_expect.that_collection",      "env_expect.that_collection(value) -> env_collection_subject"),
    ("that_file",            "env_expect.that_file",            "env_expect.that_file(value) -> env_file_subject"),
    ("that_target",          "env_expect.that_target",          "env_expect.that_target(value) -> env_target_subject"),
    ("that_depset_of_files", "env_expect.that_depset_of_files", "env_expect.that_depset_of_files(value) -> env_depset_subject"),
];

/// Flat dotted call-site aliases for the chain walker. The Starlark extractor
/// emits `env.expect.that_str` as a dotted ref (root "env"). The chain walker's
/// `resolve_ctx_chain_direct` does `by_qualified_name("env.expect.that_str")`,
/// so these symbols must exist under exactly those qnames. This mirrors how
/// CTX_MEMBERS has `ctx.actions.run_shell` as both the dotted ref and the qname.
const ENV_EXPECT_FLAT_ALIASES: &[(&str, &str, &str)] = &[
    ("that_str",             "env.expect.that_str",             "env.expect.that_str(value) -> env_str_subject"),
    ("that_int",             "env.expect.that_int",             "env.expect.that_int(value) -> env_int_subject"),
    ("that_bool",            "env.expect.that_bool",            "env.expect.that_bool(value) -> env_bool_subject"),
    ("that_collection",      "env.expect.that_collection",      "env.expect.that_collection(value) -> env_collection_subject"),
    ("that_file",            "env.expect.that_file",            "env.expect.that_file(value) -> env_file_subject"),
    ("that_target",          "env.expect.that_target",          "env.expect.that_target(value) -> env_target_subject"),
    ("that_depset_of_files", "env.expect.that_depset_of_files", "env.expect.that_depset_of_files(value) -> env_depset_subject"),
];

/// Assertion methods shared across all subject types. Void return — no further
/// chain continuation. Emitted once per subject type as `<subject>.<method>`.
const SUBJECT_ASSERTION_METHODS: &[(&str, &str)] = &[
    ("equals",           "equals(expected)"),
    ("is_none",          "is_none()"),
    ("is_true",          "is_true()"),
    ("is_false",         "is_false()"),
    ("contains",         "contains(item)"),
    ("does_not_contain", "does_not_contain(item)"),
    ("is_empty",         "is_empty()"),
    ("contains_exactly", "contains_exactly(*items)"),
    ("starts_with",      "starts_with(prefix)"),
    ("ends_with",        "ends_with(suffix)"),
    ("is_in",            "is_in(collection)"),
    ("is_at_least",      "is_at_least(min)"),
    ("is_at_most",       "is_at_most(max)"),
];

/// All typed subject names, one per `that_*` factory on `env_expect`.
const ENV_SUBJECT_TYPES: &[&str] = &[
    "env_str_subject",
    "env_int_subject",
    "env_bool_subject",
    "env_collection_subject",
    "env_file_subject",
    "env_target_subject",
    "env_depset_subject",
];

fn make_symbol(short: &str, qname: &str, sig: &str, line: u32) -> ExtractedSymbol {
    ExtractedSymbol {
        name: short.to_string(),
        qualified_name: qname.to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: 0,
        end_col: 0,
        signature: Some(sig.to_string()),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

/// Emit a synthetic `ParsedFile` for the Bazel `ctx` and `repository_ctx` APIs.
///
/// Path: `ext:bazel-builtins:ctx.bzl`
///
/// Provides exact-match symbols for chain walkers if Starlark ever gains full
/// chain-walker wiring. Until then, the predicate-based externalization in
/// `resolve.rs` handles these refs at the classifier level.
pub fn synth_ctx_api() -> ParsedFile {
    let virtual_path = "ext:bazel-builtins:ctx.bzl".to_string();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut line: u32 = 0;

    for (short, qname, sig) in CTX_MEMBERS {
        symbols.push(make_symbol(short, qname, sig, line));
        line += 1;
    }
    for (short, qname, sig) in REPOSITORY_CTX_MEMBERS {
        symbols.push(make_symbol(short, qname, sig, line));
        line += 1;
    }
    // module_ctx aliases (`mctx`, `mrctx`, `module_ctx`) — same surface
    // as repository_ctx for path/execute/etc.
    for alias in MODULE_CTX_ALIASES {
        for (short, qname, sig) in REPOSITORY_CTX_MEMBERS {
            let aliased_qname = qname.replacen("repository_ctx.", &format!("{alias}."), 1);
            symbols.push(make_symbol(short, &aliased_qname, sig, line));
            line += 1;
        }
    }
    for (short, qname, sig) in TARGET_MEMBERS {
        symbols.push(make_symbol(short, qname, sig, line));
        line += 1;
    }
    for (short, qname, sig) in RUNFILES_MEMBERS {
        symbols.push(make_symbol(short, qname, sig, line));
        line += 1;
    }
    for (short, qname, sig) in ARGS_MEMBERS {
        symbols.push(make_symbol(short, qname, sig, line));
        line += 1;
    }
    // Args local aliases — `entries.add_joined`, `arguments.use_param_file`.
    for (alias, methods) in ARGS_LOCAL_ALIASES {
        for method in *methods {
            let qname = format!("{alias}.{method}");
            let sig = format!("{alias}.{method}(...) -> Args");
            symbols.push(make_symbol(method, &qname, &sig, line));
            line += 1;
        }
    }
    for (short, qname, sig) in ATTR_FACTORIES {
        symbols.push(make_symbol(short, qname, sig, line));
        line += 1;
    }
    for (short, qname, sig) in CONFIG_FACTORIES {
        symbols.push(make_symbol(short, qname, sig, line));
        line += 1;
    }

    let sym_count = symbols.len();
    ParsedFile {
        path: virtual_path,
        language: "starlark".to_string(),
        content_hash: format!("bazel-ctx-api-{sym_count}"),
        size: 0,
        line_count: 0,
        mtime: None,
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
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
    }
}

/// Emit a synthetic `ParsedFile` for the analysistest `env` assertion API.
///
/// Path: `ext:bazel-builtins:env.bzl`
///
/// Models the full type hierarchy:
///   env → env.expect → env.expect.that_str(…) → env_str_subject.equals(…)
///
/// The flat dotted aliases (`env.expect.that_str`, etc.) are what the Starlark
/// extractor emits as ref target names, and what `resolve_ctx_chain_direct`
/// looks up via `by_qualified_name`. These mirror the pattern of CTX_MEMBERS
/// where `ctx.actions.run_shell` is both the call-site dotted ref and the qname.
pub fn synth_env_api() -> ParsedFile {
    let virtual_path = "ext:bazel-builtins:env.bzl".to_string();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut line: u32 = 0;

    // Top-level env members (env.expect, env.fail, env.assert_equals, …).
    for (short, qname, sig) in ENV_MEMBERS {
        symbols.push(make_symbol(short, qname, sig, line));
        line += 1;
    }

    // env_expect type-level factory methods (env_expect.that_str, etc.).
    for (short, qname, sig) in ENV_EXPECT_MEMBERS {
        symbols.push(make_symbol(short, qname, sig, line));
        line += 1;
    }

    // Flat dotted aliases: env.expect.that_str, env.expect.that_collection, etc.
    // These are the qnames the chain walker's by_qualified_name lookup uses.
    for (short, qname, sig) in ENV_EXPECT_FLAT_ALIASES {
        symbols.push(make_symbol(short, qname, sig, line));
        line += 1;
    }

    // Per-subject assertion methods: env_str_subject.equals, etc.
    for subject in ENV_SUBJECT_TYPES {
        for (method, method_sig) in SUBJECT_ASSERTION_METHODS {
            let qname = format!("{subject}.{method}");
            let sig = format!(
                "{qname}({}",
                method_sig.trim_start_matches(&format!("{method}("))
            );
            symbols.push(make_symbol(method, &qname, &sig, line));
            line += 1;
        }
    }

    let sym_count = symbols.len();
    ParsedFile {
        path: virtual_path,
        language: "starlark".to_string(),
        content_hash: format!("bazel-env-api-{sym_count}"),
        size: 0,
        line_count: 0,
        mtime: None,
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
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
    }
}

/// The Bazel native built-in rules. These are implemented in Java and have no
/// .bzl source. We emit a single synthetic `ParsedFile` at the virtual path
/// `ext:bazel-builtins:rules.bzl` so that BUILD-file references like
/// `cc_library(...)` resolve to a real symbol instead of an unresolved ref.
const BUILTIN_RULES: &[&str] = &[
    "cc_library",
    "cc_binary",
    "cc_test",
    "java_library",
    "java_binary",
    "java_test",
    "py_library",
    "py_binary",
    "py_test",
    "genrule",
    "filegroup",
    "exports_files",
    "package",
    "alias",
    "config_setting",
    "constraint_value",
    "platform",
    "toolchain",
    "sh_library",
    "sh_binary",
    "sh_test",
    "proto_library",
    "test_suite",
];

pub fn synth_builtin_rules() -> ParsedFile {
    let virtual_path = "ext:bazel-builtins:rules.bzl".to_string();
    let symbols: Vec<ExtractedSymbol> = BUILTIN_RULES
        .iter()
        .enumerate()
        .map(|(i, &rule)| ExtractedSymbol {
            name: rule.to_string(),
            qualified_name: rule.to_string(),
            kind: SymbolKind::Function,
            visibility: Some(Visibility::Public),
            start_line: i as u32,
            end_line: i as u32,
            start_col: 0,
            end_col: 0,
            signature: Some(format!("def {rule}(**kwargs)")),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
        })
        .collect();

    let sym_count = symbols.len();
    ParsedFile {
        path: virtual_path,
        language: "starlark".to_string(),
        content_hash: format!("bazel-builtins-{sym_count}"),
        size: 0,
        line_count: 0,
        mtime: None,
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
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        let eco = BazelCentralRegistryEcosystem;
        assert_eq!(eco.id(), ID);
        assert_eq!(Ecosystem::kind(&eco), EcosystemKind::Package);
        assert_eq!(Ecosystem::languages(&eco), &["starlark"]);
        assert_eq!(eco.id().as_str(), "bazel-central-registry");
    }

    #[test]
    fn parse_module_bazel_extracts_bazel_deps() {
        let content = r#"
module(
    name = "bazel_skylib",
    version = "1.9.0",
    compatibility_level = 1,
)

bazel_dep(name = "platforms", version = "0.0.10")
bazel_dep(name = "rules_license", version = "1.0.0")
bazel_dep(name = "stardoc", version = "0.8.0", dev_dependency = True, repo_name = "io_bazel_stardoc")
bazel_dep(name = "rules_cc", version = "0.0.17", dev_dependency = True)
"#;
        let deps = extract_bzlmod_deps(content);
        assert!(deps.contains(&"platforms".to_string()), "platforms missing");
        assert!(deps.contains(&"rules_license".to_string()), "rules_license missing");
        assert!(deps.contains(&"stardoc".to_string()), "stardoc missing");
        assert!(deps.contains(&"rules_cc".to_string()), "rules_cc missing");
        // module() itself is not a dep.
        assert!(!deps.contains(&"bazel_skylib".to_string()), "module name should not be a dep");
    }

    #[test]
    fn parse_workspace_extracts_http_archive_deps() {
        let content = r#"
workspace(name = "bazel_skylib")

http_archive(
    name = "rules_cc",
    sha256 = "abc605dd850f813bb37004b77db20106a19311a96b2da1c92b789da529d28fe1",
    strip_prefix = "rules_cc-0.0.17",
    urls = ["https://github.com/bazelbuild/rules_cc/releases/download/0.0.17/rules_cc-0.0.17.tar.gz"],
)

http_archive(
    name = "rules_shell",
    sha256 = "d8cd4a3a91fc1dc68d4c7d6b655f09def109f7186437e3f50a9b60ab436a0c53",
    url = "https://github.com/bazelbuild/rules_shell/releases/download/v0.3.0/rules_shell-v0.3.0.tar.gz",
)
"#;
        let deps = extract_workspace_deps(content);
        assert!(deps.contains(&"rules_cc".to_string()), "rules_cc missing from WORKSPACE");
        assert!(deps.contains(&"rules_shell".to_string()), "rules_shell missing from WORKSPACE");
    }

    #[test]
    fn builtin_rules_contains_cc_library() {
        let pf = synth_builtin_rules();
        assert_eq!(pf.path, "ext:bazel-builtins:rules.bzl");
        assert_eq!(pf.language, "starlark");
        let has_cc = pf.symbols.iter().any(|s| s.name == "cc_library");
        assert!(has_cc, "cc_library not in builtin rules");
        let has_genrule = pf.symbols.iter().any(|s| s.name == "genrule");
        assert!(has_genrule, "genrule not in builtin rules");
        assert_eq!(pf.symbols.len(), BUILTIN_RULES.len());
    }

    #[test]
    fn builtin_rule_count() {
        // Keep in sync with the BUILTIN_RULES constant.
        assert_eq!(BUILTIN_RULES.len(), 23);
    }

    #[test]
    fn walk_bazel_root_returns_starlark_files() {
        let tmp = std::env::temp_dir().join("bw-test-bazel-walk");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("lib")).unwrap();
        std::fs::write(tmp.join("lib").join("paths.bzl"), "def join(*args): pass").unwrap();
        std::fs::write(tmp.join("BUILD"), "filegroup(name = \"all\")").unwrap();
        std::fs::write(tmp.join("not_starlark.py"), "x = 1").unwrap();

        let dep = ExternalDepRoot {
            module_path: "test_dep".to_string(),
            version: String::new(),
            root: tmp.clone(),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        };
        let files = walk_bazel_root(&dep);
        assert_eq!(files.len(), 2, "expected BUILD + paths.bzl, got {}", files.len());
        assert!(files.iter().all(|f| f.language == "starlark"));
        assert!(files.iter().any(|f| f.relative_path.ends_with("paths.bzl")));
        assert!(files.iter().any(|f| f.relative_path.ends_with("BUILD")));
        // .py files must not appear.
        assert!(files.iter().all(|f| !f.relative_path.ends_with(".py")));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn legacy_locator_tag() {
        assert_eq!(
            ExternalSourceLocator::ecosystem(&BazelCentralRegistryEcosystem),
            "bazel-central-registry"
        );
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }

    #[test]
    fn synth_ctx_api_has_expected_symbols() {
        let pf = synth_ctx_api();
        assert_eq!(pf.path, "ext:bazel-builtins:ctx.bzl");
        assert_eq!(pf.language, "starlark");

        let has_run_shell = pf.symbols.iter().any(|s| s.qualified_name == "ctx.actions.run_shell");
        assert!(has_run_shell, "ctx.actions.run_shell not in ctx API");

        let has_label_name = pf.symbols.iter().any(|s| s.qualified_name == "ctx.label.name");
        assert!(has_label_name, "ctx.label.name not in ctx API");

        let has_label_pkg = pf.symbols.iter().any(|s| s.qualified_name == "ctx.label.package");
        assert!(has_label_pkg, "ctx.label.package not in ctx API");

        let has_repo_execute = pf.symbols.iter().any(|s| s.qualified_name == "repository_ctx.execute");
        assert!(has_repo_execute, "repository_ctx.execute not in ctx API");

        let has_repo_os = pf.symbols.iter().any(|s| s.qualified_name == "repository_ctx.os");
        assert!(has_repo_os, "repository_ctx.os not in ctx API");

        let expected_count = CTX_MEMBERS.len() + REPOSITORY_CTX_MEMBERS.len();
        assert_eq!(
            pf.symbols.len(), expected_count,
            "ctx API symbol count mismatch: expected {expected_count}, got {}",
            pf.symbols.len()
        );
    }

    #[test]
    fn parse_metadata_only_returns_all_synth_files() {
        let eco = BazelCentralRegistryEcosystem;
        let dep = ExternalDepRoot {
            module_path: "dummy".to_string(),
            version: String::new(),
            root: std::path::PathBuf::from("/tmp"),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        };
        let files = <BazelCentralRegistryEcosystem as Ecosystem>::parse_metadata_only(&eco, &dep)
            .expect("expected Some");
        assert_eq!(files.len(), 3, "expected rules.bzl + ctx.bzl + env.bzl synthetic files");
        assert!(files.iter().any(|f| f.path == "ext:bazel-builtins:rules.bzl"));
        assert!(files.iter().any(|f| f.path == "ext:bazel-builtins:ctx.bzl"));
        assert!(files.iter().any(|f| f.path == "ext:bazel-builtins:env.bzl"));
    }

    #[test]
    fn synth_env_api_has_expected_symbols() {
        let pf = synth_env_api();
        assert_eq!(pf.path, "ext:bazel-builtins:env.bzl");
        assert_eq!(pf.language, "starlark");

        // Top-level env members.
        assert!(pf.symbols.iter().any(|s| s.qualified_name == "env.expect"),
            "env.expect missing");
        assert!(pf.symbols.iter().any(|s| s.qualified_name == "env.fail"),
            "env.fail missing");
        assert!(pf.symbols.iter().any(|s| s.qualified_name == "env.assert_equals"),
            "env.assert_equals missing");

        // env_expect type-level factory methods.
        assert!(pf.symbols.iter().any(|s| s.qualified_name == "env_expect.that_str"),
            "env_expect.that_str missing");
        assert!(pf.symbols.iter().any(|s| s.qualified_name == "env_expect.that_collection"),
            "env_expect.that_collection missing");

        // Flat dotted aliases (what the chain walker looks up).
        assert!(pf.symbols.iter().any(|s| s.qualified_name == "env.expect.that_str"),
            "env.expect.that_str flat alias missing");
        assert!(pf.symbols.iter().any(|s| s.qualified_name == "env.expect.that_collection"),
            "env.expect.that_collection flat alias missing");

        // Subject assertion methods.
        assert!(pf.symbols.iter().any(|s| s.qualified_name == "env_str_subject.equals"),
            "env_str_subject.equals missing");
        assert!(pf.symbols.iter().any(|s| s.qualified_name == "env_collection_subject.contains"),
            "env_collection_subject.contains missing");
        assert!(pf.symbols.iter().any(|s| s.qualified_name == "env_bool_subject.is_true"),
            "env_bool_subject.is_true missing");

        // Total = ENV_MEMBERS + ENV_EXPECT_MEMBERS + ENV_EXPECT_FLAT_ALIASES +
        //         (subject types x assertion methods).
        let expected = ENV_MEMBERS.len()
            + ENV_EXPECT_MEMBERS.len()
            + ENV_EXPECT_FLAT_ALIASES.len()
            + ENV_SUBJECT_TYPES.len() * SUBJECT_ASSERTION_METHODS.len();
        assert_eq!(
            pf.symbols.len(), expected,
            "env API symbol count mismatch: expected {expected}, got {}",
            pf.symbols.len()
        );
    }
}
