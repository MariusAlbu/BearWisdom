// =============================================================================
// ecosystem/jinja_ansible_runtime.rs — on-disk discovery of Jinja2/Ansible
// runtime grammars
//
// Jinja2 templates and Ansible playbooks reference filter/global/lookup
// names (`indent`, `to_nice_yaml`, `inventory_hostname`, `lookup`, etc.)
// that are defined inside the upstream Python packages on the user's
// machine — `jinja2.defaults` for filters/tests/globals, `ansible.plugins.
// {filter,lookup,test}` for the Ansible-specific surface.
//
// We discover them via the standard Python externals walker: locate any
// reachable site-packages, look for `jinja2/` and `ansible/` package
// directories, return them as ExternalDepRoots. The regular Python
// extractor then walks those trees and emits real symbols.
//
// **No vendored data, no synthetic ParsedFiles.** If the user doesn't
// have these packages installed, references stay unresolved — that's the
// honest signal. Most Jinja2/Ansible projects either ship their own venv
// or have the package available at the system Python.
//
// Activation: `.j2`/`.jinja`/`.jinja2` files (Jinja2 path) OR canonical
// Ansible project markers (`ansible.cfg`, `roles/`, `inventory/`,
// `playbook.yml`, `playbooks/`).
//
// Site-packages search supplements the project-local venv discovery in
// `pypi.rs::find_python_site_packages`:
//   * `BEARWISDOM_PYTHON_SITE_PACKAGES` env var (split on `;` / `:`).
//   * `python -c "import site; print(';'.join(site.getsitepackages()))"`
//     (system-wide Python install).
//   * `python -m site --user-site` (user-site, used by `pip install --user`).
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::ecosystem::pypi;
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("jinja-ansible-runtime");
const ECOSYSTEM_TAG: &str = "jinja-ansible-runtime";
const LANGUAGES: &[&str] = &["jinja", "yaml"];

pub struct JinjaAnsibleRuntimeEcosystem;

impl Ecosystem for JinjaAnsibleRuntimeEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        // Two activation paths matching this ecosystem's two consumers:
        //   * `LanguagePresent("jinja")` — Jinja2 is a substrate for every
        //     `.j2`/`.jinja` template. Flask, Django, Pelican, Salt, Hexo's
        //     Jinja-flavoured fork all benefit from the upstream filter set.
        //   * `ansible.cfg` — Ansible projects' canonical marker. Playbooks
        //     are `.yml` (typed `yaml`, not `jinja`), so the substrate path
        //     doesn't fire on Ansible-only repos. The `.cfg` extension hits
        //     the evaluator's plain-text substring branch; `[defaults]` is
        //     the section every ansible.cfg installs ship as a template, and
        //     a substring match on `ansible_managed` covers configs that
        //     drop the section header in favour of inline keys.
        EcosystemActivation::Any(&[
            EcosystemActivation::LanguagePresent("jinja"),
            EcosystemActivation::ManifestFieldContains {
                manifest_glob: "**/ansible.cfg",
                field_path: "",
                value: "[defaults]",
            },
            EcosystemActivation::ManifestFieldContains {
                manifest_glob: "**/ansible.cfg",
                field_path: "",
                value: "ansible_managed",
            },
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_runtime_roots(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        // Reuse the regular Python tree walk — the runtime packages are
        // ordinary Python packages, parsed by the Python extractor.
        pypi::walk_python_external_root(dep)
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
}

impl ExternalSourceLocator for JinjaAnsibleRuntimeEcosystem {
    fn ecosystem(&self) -> &'static str { ECOSYSTEM_TAG }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_runtime_roots(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        pypi::walk_python_external_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<JinjaAnsibleRuntimeEcosystem>> = OnceLock::new();
    LOCATOR
        .get_or_init(|| Arc::new(JinjaAnsibleRuntimeEcosystem))
        .clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn discover_runtime_roots(project_root: &Path) -> Vec<ExternalDepRoot> {
    let mut roots: Vec<ExternalDepRoot> = Vec::new();
    let want_ansible = project_has_ansible_markers(project_root);

    // Collect every reachable site-packages: project-local venvs (via the
    // shared `find_python_site_packages` helper) plus the user/system
    // installs that pypi.rs's helper intentionally skips.
    let mut search_paths = pypi::find_python_site_packages(project_root);
    augment_with_global_site_packages(&mut search_paths);

    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for sp in &search_paths {
        // jinja2: every `.j2`-using project benefits (Flask, Django,
        // Ansible, raw Jinja).
        let jinja_dir = sp.join("jinja2");
        if jinja_dir.is_dir() && seen.insert(jinja_dir.clone()) {
            roots.push(ExternalDepRoot {
                module_path: "jinja2".to_string(),
                version: String::from("local"),
                root: jinja_dir,
                ecosystem: ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: Vec::new(),
            });
        }
        if want_ansible {
            let ansible_dir = sp.join("ansible");
            if ansible_dir.is_dir() && seen.insert(ansible_dir.clone()) {
                roots.push(ExternalDepRoot {
                    module_path: "ansible".to_string(),
                    version: String::from("local"),
                    root: ansible_dir,
                    ecosystem: ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: Vec::new(),
                });
            }
        }
    }

    if !roots.is_empty() {
        tracing::debug!(
            "jinja-ansible-runtime: found {} runtime package root(s)",
            roots.len()
        );
    }
    roots
}

fn project_has_ansible_markers(project_root: &Path) -> bool {
    project_root.join("ansible.cfg").is_file()
        || project_root.join("roles").is_dir()
        || project_root.join("inventory").is_dir()
        || project_root.join("playbook.yml").is_file()
        || project_root.join("playbooks").is_dir()
        || project_root.join("galaxy.yml").is_file()
}

/// Extend the site-packages search with system-wide and user-site Python
/// installs. The pypi.rs helper deliberately scopes to project-local
/// venvs; runtime grammars don't ship per-project so we cast wider.
fn augment_with_global_site_packages(out: &mut Vec<PathBuf>) {
    let mut push_if_new = |p: PathBuf, out: &mut Vec<PathBuf>| {
        if p.is_dir() && !out.contains(&p) {
            out.push(p);
        }
    };

    // 1. Explicit override: BEARWISDOM_PYTHON_SITE_PACKAGES (split on `;` / `:`).
    if let Some(raw) = std::env::var_os("BEARWISDOM_PYTHON_SITE_PACKAGES") {
        for seg in std::env::split_paths(&raw) {
            if !seg.as_os_str().is_empty() {
                push_if_new(seg, out);
            }
        }
    }

    // 2. Ask Python directly. `site.getsitepackages()` returns system-wide
    // dirs; `site.getusersitepackages()` returns the user-site dir.
    for arg in &[
        "-c",
        "import site,sys;print(';'.join([*site.getsitepackages(),site.getusersitepackages()]))",
    ] {
        let _ = arg; // keep clippy quiet
    }
    if let Ok(output) = Command::new("python")
        .args([
            "-c",
            "import site,sys;\
             paths = list(site.getsitepackages()) + [site.getusersitepackages()];\
             print(';'.join(p for p in paths if p))",
        ])
        .output()
    {
        if output.status.success() {
            if let Ok(text) = std::str::from_utf8(&output.stdout) {
                for seg in text.trim().split(';') {
                    let p = PathBuf::from(seg.trim());
                    if !p.as_os_str().is_empty() {
                        push_if_new(p, out);
                    }
                }
            }
        }
    }

    // 3. Try `python3` as a fallback name (Linux distros that don't symlink
    // `python` to Python 3).
    if out.is_empty() {
        if let Ok(output) = Command::new("python3")
            .args([
                "-c",
                "import site;print(';'.join(list(site.getsitepackages())+[site.getusersitepackages()]))",
            ])
            .output()
        {
            if output.status.success() {
                if let Ok(text) = std::str::from_utf8(&output.stdout) {
                    for seg in text.trim().split(';') {
                        let p = PathBuf::from(seg.trim());
                        if !p.as_os_str().is_empty() {
                            push_if_new(p, out);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "jinja_ansible_runtime_tests.rs"]
mod tests;
