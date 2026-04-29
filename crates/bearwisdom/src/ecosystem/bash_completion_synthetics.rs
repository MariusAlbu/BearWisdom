// =============================================================================
// ecosystem/bash_completion_synthetics.rs — bash-completion runtime stubs
//
// `bash-completion` (https://github.com/scop/bash-completion) is the
// system-installed library every bash dotfile / oh-my-bash / bash-it
// project assumes is loaded into the user's shell. It exposes a fixed
// set of helper functions that completion scripts call by bare name —
// `_filedir`, `_init_completion`, `__git_*`, etc. — without any explicit
// `source` directive in user code, because the user's `~/.bashrc` or
// shell-init flow handles that.
//
// On a developer machine where `bash-completion` isn't installed (e.g.
// most Windows setups, minimal containers), the package's `.sh` source
// isn't on disk so the BearWisdom externals walker has nothing to walk.
// The functions are still legitimately external: classifying every
// `_filedir` reference as unresolved misrepresents the codebase shape.
//
// Mirrors `sdl_synthetics.rs`: synthesize a fixed set of symbols
// representing the bash-completion API surface, attach them to a
// dedicated synthetic dep root, and let the resolver bind shell-script
// refs against them via the standard symbol-index lookup.
//
// Activation: `bash` / `shell` is present AND any project file uses one
// of the bash-completion entry points (`complete -F`, `_filedir`,
// `_init_completion`, etc.). `locate_roots` scans for these markers;
// projects that don't use bash-completion don't pay any cost.
// =============================================================================

use std::path::Path;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("bash-completion-synthetics");
const TAG: &str = "bash-completion-synthetics";
const LANGUAGES: &[&str] = &["bash", "shell"];

/// bash-completion library functions. Sourced from the upstream package's
/// `bash_completion` script + per-tool completion fragments shipped under
/// `/usr/share/bash-completion/completions/`. Coverage targets every
/// helper that surfaces as an unresolved call in a serious bash project
/// (oh-my-bash, bash-it, dotfiles, completion-driven CLIs).
const BASH_COMPLETION_FUNCTIONS: &[&str] = &[
    // Core completion helpers — `_filedir` is the dominant unresolved on
    // any completion-using project. The rest cover argument-style
    // completion (`_init_completion`, `_count_args`, `_command_offset`),
    // word manipulation (`__reassemble_comp_words_by_ref`,
    // `__get_cword_at_cursor_by_ref`), and value sources (`_pids`,
    // `_pgids`, `_uids`, `_gids`, `_groups`, `_known_hosts`, `_services`).
    "_filedir",
    "_filedir_xspec",
    "_init_completion",
    "_count_args",
    "_command_offset",
    "_command",
    "_completion_loader",
    "_get_comp_words_by_ref",
    "_get_cword",
    "_get_first_arg",
    "_get_pword",
    "_longopt",
    "_minimal",
    "_parse_help",
    "_parse_usage",
    "_realcommand",
    "_split_longopt",
    "_upvars",
    "_variables",
    "_have",
    "have",
    "_terms",
    "__reassemble_comp_words_by_ref",
    "__get_cword_at_cursor_by_ref",
    "__ltrim_colon_completions",
    "__expand_tilde_by_ref",
    "_quote_readline_by_ref",
    "_xfunc",
    "_allowed_users",
    "_allowed_groups",
    "_pids",
    "_pgids",
    "_pnames",
    "_uids",
    "_gids",
    "_users",
    "_groups",
    "_known_hosts",
    "_known_hosts_real",
    "_services",
    "_modules",
    "_installed_modules",
    "_kernel_versions",
    "_available_interfaces",
    "_configured_interfaces",
    "_mac_addresses",
    "_signals",
    "_shells",
    "_fstypes",
    "_usb_ids",
    "_pci_ids",
    "_cd",
    "_user_at_host",
    "_pname",
    "_root_command",
    "_dvd_devices",
    "_cd_devices",
    // Git-specific completion helpers shipped under
    // `/usr/share/bash-completion/completions/git`. Universally referenced
    // from project-local git completion fragments.
    "__git_main",
    "__git_complete",
    "__git_commands",
    "__git_list_all_commands",
    "__git_list_all_commands_without_hub",
    "__git_porcelain_commands",
    "__git_complete_revlist",
    "__git_complete_revlist_file",
    "__git_complete_remote_or_refspec",
    "__git_refs",
    "__git_refs_remotes",
    "__git_remotes",
    "__git_heads",
    "__git_tags",
    "__git_branches",
    "__git_show_branches",
    "__git_remote_branches",
    "__git_aliases",
    "__git_aliased_command",
    "__git_find_on_cmdline",
    "__git_find_subcommand",
    "__git_get_option_value",
    "__git_match_ctag",
    "__git_compute_porcelain_commands",
    "__git_compute_all_commands",
    "__git_compute_merge_strategies",
    "__git_compute_config_vars",
    "__gitcomp",
    "__gitcomp_builtin",
    "__gitcomp_direct",
    "__gitcomp_nl",
    "__gitcomp_nl_append",
    "__gitcompappend",
    "__gitcompadd",
    "__gitcompappend_helper",
    "__git_pretty_aliases",
    "__git_eread",
    "__git_command_idx",
    "__git_diff_algorithms",
    "__git_diff_submodule_formats",
    "__git_fetch_recurse_submodules",
    "__git_log_pretty_formats",
    "__git_log_date_formats",
    "__git_index_files",
    "__git_diff_index_options",
    "__git_diff_common_options",
];

// ---------------------------------------------------------------------------
// Detection: does this project look like it relies on bash-completion?
// ---------------------------------------------------------------------------

fn project_uses_bash_completion(project_root: &Path) -> bool {
    scan_for_completion_marker(project_root, 0)
}

fn scan_for_completion_marker(dir: &Path, depth: u32) -> bool {
    if depth > 4 { return false }
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || matches!(name, "node_modules" | "target" | "dist" | "build") {
                continue;
            }
            if scan_for_completion_marker(&path, depth + 1) {
                return true;
            }
        } else if ft.is_file() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Look for shell scripts and completion-specific filenames.
            let is_shell = matches!(
                name.rsplit('.').next().unwrap_or(""),
                "sh" | "bash" | "zsh" | "completion"
            ) || name == "bash_completion"
                || name.contains("completion")
                || name.contains("complete");
            if !is_shell {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                // Any reference to a bash-completion helper, or a
                // `complete -F` registration line, signals usage.
                if content.contains("_filedir")
                    || content.contains("_init_completion")
                    || content.contains("_get_comp_words_by_ref")
                    || content.contains("__gitcomp")
                    || content.contains("complete -F ")
                    || content.contains("complete -W ")
                {
                    return true;
                }
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Symbol construction
// ---------------------------------------------------------------------------

fn fn_sym(name: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("/* bash-completion runtime */ {name}()")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

fn synthesize_file() -> ParsedFile {
    let symbols: Vec<ExtractedSymbol> = BASH_COMPLETION_FUNCTIONS
        .iter()
        .map(|f| fn_sym(f))
        .collect();
    let n = symbols.len();
    ParsedFile {
        path: "ext:bash-completion-synthetics:bash_completion.sh".to_string(),
        language: "bash".to_string(),
        content_hash: format!("bash-completion-synthetics-{n}"),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: vec![None; n],
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: vec![false; n],
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
    }
}

fn synthetic_dep_root() -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "bash-completion-synthetics".to_string(),
        version: String::new(),
        root: std::path::PathBuf::from("ext:bash-completion-synthetics"),
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Ecosystem impl
// ---------------------------------------------------------------------------

pub struct BashCompletionSyntheticsEcosystem;

impl Ecosystem for BashCompletionSyntheticsEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::LanguagePresent("bash"),
            EcosystemActivation::LanguagePresent("shell"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        if !project_uses_bash_completion(ctx.project_root) {
            return Vec::new();
        }
        vec![synthetic_dep_root()]
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn parse_metadata_only(&self, _dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        Some(vec![synthesize_file()])
    }
}

impl ExternalSourceLocator for BashCompletionSyntheticsEcosystem {
    fn ecosystem(&self) -> &'static str { TAG }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        if !project_uses_bash_completion(project_root) {
            return Vec::new();
        }
        vec![synthetic_dep_root()]
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        Some(vec![synthesize_file()])
    }
}

#[cfg(test)]
#[path = "bash_completion_synthetics_tests.rs"]
mod tests;
