//! Cargo configuration ingestion; semantic module traversal never reads TOML.
use super::*;
use crate::ecosystem::manifest::module_config::{
    ModuleDependency, ModulePackage, ModuleTarget, TargetKind,
};
use sha2::{Digest, Sha256};
use toml::Value;

pub(super) fn entry(manifest_path: PathBuf, project_root: &Path) -> Option<ReaderEntry> {
    let content = std::fs::read_to_string(&manifest_path).ok()?;
    let package_dir = manifest_path.parent()?.to_path_buf();
    let mut data = ManifestData {
        dependencies: parse_cargo_dependencies(&content).into_iter().collect(),
        project_refs: parse_cargo_path_dependencies(&content),
        dep_renames: parse_cargo_dep_renames(&content),
        ..Default::default()
    };
    if let Some(config) = configuration(&content, &package_dir, project_root) {
        data.module_packages.push(config);
    }
    Some(ReaderEntry {
        name: parse_cargo_package_name(&content),
        package_dir,
        manifest_path,
        data,
    })
}

fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}

pub(super) fn configuration(content: &str, dir: &Path, project: &Path) -> Option<ModulePackage> {
    let value: Value = content.parse().ok()?;
    let package = value.get("package")?;
    let name = string(package, "name")?;
    let relative = dir
        .strip_prefix(project)
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    let mut out = ModulePackage {
        root: relative,
        name: name.into(),
        fingerprint: String::new(),
        targets: targets(&value, package, dir, name),
        dependencies: Vec::new(),
    };
    let inherited = workspace(dir, project);
    let inheritance = inherited
        .as_ref()
        .map(|(root, text, value)| (root.as_path(), text.as_str(), value));
    dependencies(
        &value,
        dir,
        project,
        inheritance,
        false,
        &mut out.dependencies,
    );
    if let Some(targets) = value.get("target").and_then(Value::as_table) {
        for target in targets.values() {
            dependencies(
                target,
                dir,
                project,
                inheritance,
                true,
                &mut out.dependencies,
            );
        }
    }
    let mut hash = Sha256::new();
    hash.update(content.as_bytes());
    if let Some((_, text, _)) = inherited {
        hash.update(text.as_bytes());
    }
    // Auto-discovered target changes are configuration changes even with identical TOML.
    hash.update(serde_json::to_vec(&out.targets).ok()?);
    out.fingerprint = format!("{:x}", hash.finalize());
    Some(out)
}

pub(super) fn workspace(dir: &Path, project: &Path) -> Option<(PathBuf, String, Value)> {
    for ancestor in dir.ancestors().take_while(|p| p.starts_with(project)) {
        let Ok(text) = std::fs::read_to_string(ancestor.join("Cargo.toml")) else {
            continue;
        };
        let Ok(value) = text.parse::<Value>() else {
            continue;
        };
        if value.get("workspace").is_some() {
            return Some((ancestor.into(), text, value));
        }
    }
    None
}

fn dependencies(
    value: &Value,
    dir: &Path,
    project: &Path,
    workspace: Option<(&Path, &str, &Value)>,
    conditional: bool,
    out: &mut Vec<ModuleDependency>,
) {
    for (section, kind) in [
        ("dependencies", TargetKind::Library),
        ("dev-dependencies", TargetKind::Development),
        ("build-dependencies", TargetKind::Build),
    ] {
        let Some(table) = value.get(section).and_then(Value::as_table) else {
            continue;
        };
        for (key, original) in table {
            let inherited = original.get("workspace").and_then(Value::as_bool) == Some(true);
            let definition = if inherited {
                workspace
                    .and_then(|(_, _, value)| value.get("workspace")?.get("dependencies")?.get(key))
            } else {
                Some(original)
            };
            let base = if inherited {
                workspace.map(|(root, _, _)| root)
            } else {
                Some(dir)
            };
            let package = definition.and_then(|v| string(v, "package"));
            let root = definition.and_then(|v| string(v, "path")).and_then(|path| {
                let path = base?.strip_prefix(project).ok()?.join(path);
                Some(path.to_string_lossy().replace('\\', "/"))
            });
            out.push(ModuleDependency {
                alias: key.replace('-', "_"),
                package: package.unwrap_or(key).into(),
                root,
                renamed: package.is_some(),
                kind,
                conditional: conditional
                    || definition.is_none()
                    || original.get("optional").and_then(Value::as_bool) == Some(true)
                    || definition
                        .and_then(|v| v.get("optional"))
                        .and_then(Value::as_bool)
                        == Some(true),
            });
        }
    }
}

fn targets(value: &Value, package: &Value, dir: &Path, package_name: &str) -> Vec<ModuleTarget> {
    let mut out = Vec::new();
    if value.get("lib").is_some()
        || (package.get("autolib").and_then(Value::as_bool) != Some(false)
            && dir.join("src/lib.rs").is_file())
    {
        let lib = value.get("lib");
        out.push(ModuleTarget {
            name: lib
                .and_then(|v| string(v, "name"))
                .map(str::to_owned)
                .unwrap_or_else(|| package_name.replace('-', "_")),
            path: lib
                .and_then(|v| string(v, "path"))
                .unwrap_or("src/lib.rs")
                .into(),
            kind: TargetKind::Library,
            conditional: false,
        });
    }
    for (section, flag, folder, kind) in [
        ("bin", "autobins", "src/bin", TargetKind::Executable),
        (
            "example",
            "autoexamples",
            "examples",
            TargetKind::Development,
        ),
        ("test", "autotests", "tests", TargetKind::Development),
        ("bench", "autobenches", "benches", TargetKind::Development),
    ] {
        let mut inferred = discover(dir, folder, kind);
        if section == "bin" && dir.join("src/main.rs").is_file() {
            inferred.push(ModuleTarget {
                name: package_name.into(),
                path: "src/main.rs".into(),
                kind,
                conditional: false,
            });
        }
        let explicit = value
            .get(section)
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for item in explicit {
            let Some(name) = string(item, "name") else {
                continue;
            };
            let path = string(item, "path").map(str::to_owned).or_else(|| {
                let matching: Vec<_> = inferred.iter().filter(|t| t.name == name).collect();
                (matching.len() == 1).then(|| matching[0].path.clone())
            });
            if let Some(path) = path {
                out.push(ModuleTarget {
                    name: name.into(),
                    path,
                    kind,
                    conditional: item
                        .get("required-features")
                        .and_then(Value::as_array)
                        .is_some_and(|v| !v.is_empty()),
                });
            }
        }
        if package.get(flag).and_then(Value::as_bool) != Some(false) {
            inferred.retain(|t| {
                !explicit.iter().any(|v| {
                    string(v, "name") == Some(&t.name) || string(v, "path") == Some(&t.path)
                })
            });
            out.extend(inferred);
        }
    }
    match package.get("build") {
        Some(Value::Boolean(false)) => {}
        option => {
            let path = option.and_then(Value::as_str).unwrap_or("build.rs");
            if option.and_then(Value::as_str).is_some() || dir.join(path).is_file() {
                out.push(ModuleTarget {
                    name: "build_script_build".into(),
                    path: path.into(),
                    kind: TargetKind::Build,
                    conditional: false,
                });
            }
        }
    }
    out
}

fn discover(dir: &Path, folder: &str, kind: TargetKind) -> Vec<ModuleTarget> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir.join(folder)) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let candidate = if path.is_dir() {
            path.join("main.rs")
        } else {
            path.clone()
        };
        if candidate.extension().and_then(|v| v.to_str()) != Some("rs") || !candidate.is_file() {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|v| v.to_str()) else {
            continue;
        };
        let Ok(relative) = candidate.strip_prefix(dir) else {
            continue;
        };
        out.push(ModuleTarget {
            name: name.into(),
            path: relative.to_string_lossy().replace('\\', "/"),
            kind,
            conditional: false,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

#[cfg(test)]
#[path = "module_manifest_tests.rs"]
mod tests;
