// Perl / cpanm externals

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Perl cpanm/local::lib → `discover_perl_externals` + `walk_perl_external_root`.
///
/// Perl modules installed via cpanm with local::lib live in `local/lib/perl5/`.
/// Declared deps come from `cpanfile` `requires 'Module::Name';` lines.
/// Walk: `*.pm` and `*.pl` under the module root.
pub struct PerlExternalsLocator;

impl ExternalSourceLocator for PerlExternalsLocator {
    fn ecosystem(&self) -> &'static str { "perl" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_perl_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_perl_external_root(dep)
    }
}

pub fn discover_perl_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let cpanfile = project_root.join("cpanfile");
    if !cpanfile.is_file() {
        return Vec::new();
    }
    let Ok(content) = std::fs::read_to_string(&cpanfile) else {
        return Vec::new();
    };
    let declared = parse_cpanfile_requires(&content);
    if declared.is_empty() {
        return Vec::new();
    }

    let lib_dirs = perl_lib_dirs(project_root);
    if lib_dirs.is_empty() {
        return Vec::new();
    }

    let mut roots = Vec::new();
    for module_name in &declared {
        // Perl module Foo::Bar lives at Foo/Bar.pm or Foo/Bar/
        let path_fragment = module_name.replace("::", std::path::MAIN_SEPARATOR_STR);
        for lib in &lib_dirs {
            let module_dir = lib.join(&path_fragment);
            if module_dir.is_dir() {
                roots.push(ExternalDepRoot {
                    module_path: module_name.clone(),
                    version: String::new(),
                    root: module_dir,
                    ecosystem: "perl",
                    package_id: None,
                });
                break;
            }
            // Single-file module: Foo/Bar.pm
            let module_file = lib.join(format!("{path_fragment}.pm"));
            if module_file.is_file() {
                roots.push(ExternalDepRoot {
                    module_path: module_name.clone(),
                    version: String::new(),
                    root: module_file.parent().unwrap_or(lib).to_path_buf(),
                    ecosystem: "perl",
                    package_id: None,
                });
                break;
            }
        }
    }
    debug!("Perl: discovered {} external module roots", roots.len());
    roots
}

pub fn parse_cpanfile_requires(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') { continue; }
        // requires 'Module::Name';  or  requires 'Module::Name', '>= 1.0';
        if trimmed.starts_with("requires") {
            let rest = trimmed["requires".len()..].trim();
            let name = rest.trim_start_matches(|c: char| c == '\'' || c == '"' || c.is_whitespace());
            if let Some(end) = name.find(|c: char| c == '\'' || c == '"' || c == ',' || c == ';') {
                let module = &name[..end];
                if !module.is_empty() && module != "perl" {
                    if !deps.contains(&module.to_string()) {
                        deps.push(module.to_string());
                    }
                }
            }
        }
    }
    deps
}

fn perl_lib_dirs(project_root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    // local::lib: local/lib/perl5/
    let local = project_root.join("local").join("lib").join("perl5");
    if local.is_dir() { dirs.push(local); }
    // PERL5LIB / PERL_LOCAL_LIB_ROOT
    for var in &["PERL5LIB", "PERL_LOCAL_LIB_ROOT"] {
        if let Ok(val) = std::env::var(var) {
            for p in val.split(if cfg!(windows) { ';' } else { ':' }) {
                let pb = PathBuf::from(p);
                if pb.is_dir() { dirs.push(pb); }
            }
        }
    }
    dirs
}

pub fn walk_perl_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_perl_dir(&dep.root, &dep.root, dep, &mut out);
    out
}

fn walk_perl_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_perl_dir_bounded(dir, root, dep, out, 0);
}

fn walk_perl_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue; };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "t" | "xt" | "blib" | "examples") || name.starts_with('.') {
                    continue;
                }
            }
            walk_perl_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue; };
            if !(name.ends_with(".pm") || name.ends_with(".pl")) { continue; }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:perl:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "perl",
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perl_parses_cpanfile() {
        let content = r#"
requires 'perl', 5.014000;
requires 'Carp';
requires 'Clone';
requires 'Config::Any';
requires 'Data::Censor' => '0.04';
"#;
        let deps = parse_cpanfile_requires(content);
        assert_eq!(deps, vec!["Carp", "Clone", "Config::Any", "Data::Censor"]);
    }
}
