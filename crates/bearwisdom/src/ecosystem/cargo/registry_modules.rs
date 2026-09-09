//! Manifest/lock/source evidence enters the shared module graph at ingestion.
//! No Cargo execution, network access, symbol search or newest-version fallback.
use super::{module_manifest, ReaderEntry};
use crate::ecosystem::manifest::module_config::{ModuleDependency, ModulePackage, TargetKind};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use toml::Value;

#[derive(Clone)]
struct Package {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
    dependencies: Vec<String>,
}
struct Installed {
    package: ModulePackage,
    value: Value,
    directory: PathBuf,
    index: usize,
}
struct Registry {
    packages: Vec<Package>,
    installed: Vec<Installed>,
    fingerprint: String,
}

pub(super) fn extend(entries: &mut [ReaderEntry], project: &Path, registries: &[PathBuf]) {
    let mut locks = BTreeMap::<PathBuf, Option<Registry>>::new();
    let mut providers = Vec::new();
    for entry in entries.iter_mut() {
        let Some(path) = entry
            .package_dir
            .ancestors()
            .take_while(|p| p.starts_with(project))
            .map(|p| p.join("Cargo.lock"))
            .find(|p| p.is_file())
        else {
            continue;
        };
        let registry = locks
            .entry(path.clone())
            .or_insert_with(|| Registry::read(&path, registries));
        let Some(registry) = registry else {
            continue;
        };
        let Some(value) = read_value(&entry.manifest_path) else {
            continue;
        };
        let inherited =
            module_manifest::workspace(&entry.package_dir, project).map(|(_, _, value)| value);
        for package in &mut entry.data.module_packages {
            registry.bind(package, &value, inherited.as_ref(), None);
        }
    }
    let mut identities = BTreeMap::<&str, Vec<(&str, Option<&str>)>>::new();
    for package in locks.values().flatten().flat_map(|r| &r.packages) {
        let set = identities.entry(&package.name).or_default();
        let key = (package.version.as_str(), package.source.as_deref());
        if !set.contains(&key) {
            set.push(key);
        }
    }
    let conflicts: std::collections::HashSet<_> = identities
        .iter()
        .filter(|(_, ids)| ids.len() > 1)
        .map(|(name, _)| address(name))
        .collect();
    for package in entries.iter_mut().flat_map(|e| &mut e.data.module_packages) {
        for dep in &mut package.dependencies {
            if dep
                .root
                .as_ref()
                .is_some_and(|root| conflicts.contains(root))
            {
                dep.conditional = true;
            }
        }
    }
    for registry in locks.values().flatten() {
        for installed in &registry.installed {
            if conflicts.contains(&installed.package.root) {
                continue;
            }
            let mut package = installed.package.clone();
            registry.bind(&mut package, &installed.value, None, Some(installed.index));
            // Registry manifests are normalized at publication; no local path
            // dependency may accidentally become a workspace-relative target.
            for dep in &mut package.dependencies {
                if dep
                    .root
                    .as_ref()
                    .is_some_and(|root| !root.starts_with("ext:"))
                {
                    dep.root = None;
                    dep.conditional = true;
                }
                if dep
                    .root
                    .as_ref()
                    .is_some_and(|root| conflicts.contains(root))
                {
                    dep.conditional = true;
                }
            }
            if !providers.contains(&package) {
                providers.push(package);
            }
        }
    }
    if let Some(first) = entries.first_mut() {
        first.data.module_packages.extend(providers);
    }
}

fn read_value(path: &Path) -> Option<Value> {
    std::fs::read_to_string(path).ok()?.parse().ok()
}
fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}
fn address(name: &str) -> String {
    format!("ext:rust:{name}")
}

impl Registry {
    fn read(path: &Path, registries: &[PathBuf]) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let value: Value = text.parse().ok()?;
        let mut packages = Vec::new();
        for item in value.get("package")?.as_array()? {
            packages.push(Package {
                name: string(item, "name")?.into(),
                version: string(item, "version")?.into(),
                source: string(item, "source").map(str::to_owned),
                checksum: string(item, "checksum").map(str::to_owned),
                dependencies: match item.get("dependencies") {
                    None => Vec::new(),
                    Some(v) => v
                        .as_array()?
                        .iter()
                        .map(|v| v.as_str().map(str::to_owned))
                        .collect::<Option<_>>()?,
                },
            });
        }
        let mut installed = Vec::new();
        for (index, item) in packages.iter().enumerate() {
            if item.name.is_empty()
                || !item
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                || semver::Version::parse(&item.version).is_err()
            {
                continue;
            }
            if !item
                .source
                .as_ref()
                .is_some_and(|s| s.starts_with("registry+"))
            {
                continue;
            }
            // Existing external addresses omit versions. Until source-instance
            // identity is migrated, no two pinned packages may share that path.
            if packages.iter().filter(|p| p.name == item.name).count() != 1 {
                continue;
            }
            let Some(checksum) = item
                .checksum
                .as_deref()
                .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            else {
                continue;
            };
            let candidates: Vec<_> = registries
                .iter()
                .map(|r| r.join(format!("{}-{}", item.name, item.version)))
                .filter(|dir| dir.is_dir())
                .collect();
            let [directory] = candidates.as_slice() else {
                continue;
            };
            let payload: serde_json::Value =
                match std::fs::read_to_string(directory.join(".cargo-checksum.json"))
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                {
                    Some(value) => value,
                    None => continue,
                };
            if payload.get("package").and_then(|v| v.as_str()) != Some(checksum) {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(directory.join("Cargo.toml")) else {
                continue;
            };
            let Ok(value) = content.parse::<Value>() else {
                continue;
            };
            let Some(manifest) = value.get("package") else {
                continue;
            };
            if string(manifest, "name") != Some(&item.name)
                || string(manifest, "version") != Some(&item.version)
            {
                continue;
            }
            let Some(mut package) = module_manifest::configuration(&content, directory, directory)
            else {
                continue;
            };
            package.root = address(&item.name);
            package
                .targets
                .retain(|t| t.kind == TargetKind::Library && confined(&t.path));
            installed.push(Installed {
                package,
                value,
                directory: directory.clone(),
                index,
            });
        }
        let mut hash = Sha256::new();
        hash.update(text.as_bytes());
        for source in &installed {
            hash.update(source.directory.to_string_lossy().as_bytes());
            hash.update(source.package.fingerprint.as_bytes());
        }
        Some(Self {
            packages,
            installed,
            fingerprint: format!("{:x}", hash.finalize()),
        })
    }

    fn bind(
        &self,
        config: &mut ModulePackage,
        value: &Value,
        inherited: Option<&Value>,
        owner: Option<usize>,
    ) {
        let owner = owner.or_else(|| {
            let version = string(value.get("package")?, "version")?;
            let matching: Vec<_> = self
                .packages
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    p.name == config.name && p.version == version && p.source.is_none()
                })
                .collect();
            match matching.as_slice() {
                [(id, _)] => Some(*id),
                _ => None,
            }
        });
        for dep in &mut config.dependencies {
            if dep.root.is_some() {
                continue;
            }
            let Some(definition) = definition(value, inherited, dep) else {
                continue;
            };
            if definition.get("git").is_some() || definition.get("path").is_some() {
                continue;
            }
            let requirement = definition
                .as_str()
                .or_else(|| string(definition, "version"))
                .and_then(|s| semver::VersionReq::parse(s).ok());
            let target = owner.and_then(|id| {
                requirement
                    .as_ref()
                    .and_then(|req| self.dependency(id, &dep.package, req))
            });
            dep.root = Some(address(&dep.package));
            if definition.get("registry").is_some()
                || target.is_none_or(|id| !self.installed.iter().any(|p| p.index == id))
            {
                dep.conditional = true;
            }
        }
        let mut hash = Sha256::new();
        hash.update(config.fingerprint.as_bytes());
        hash.update(self.fingerprint.as_bytes());
        config.fingerprint = format!("{:x}", hash.finalize());
    }

    fn dependency(
        &self,
        owner: usize,
        name: &str,
        requirement: &semver::VersionReq,
    ) -> Option<usize> {
        let mut found = Vec::new();
        for link in &self.packages.get(owner)?.dependencies {
            let mut parts = link.splitn(3, ' ');
            let linked_name = parts.next()?;
            if linked_name != name {
                continue;
            }
            let version = parts.next();
            let source = parts.next();
            for (id, item) in self.packages.iter().enumerate() {
                if item.name != name
                    || version.is_some_and(|v| item.version != v)
                    || source.is_some_and(|s| {
                        s.strip_prefix('(').and_then(|s| s.strip_suffix(')'))
                            != item.source.as_deref()
                    })
                {
                    continue;
                }
                if semver::Version::parse(&item.version)
                    .ok()
                    .is_some_and(|v| requirement.matches(&v))
                    && item
                        .source
                        .as_ref()
                        .is_some_and(|s| s.starts_with("registry+"))
                {
                    found.push(id);
                }
            }
        }
        match found.as_slice() {
            [id] => Some(*id),
            _ => None,
        }
    }
}

fn definition<'a>(
    value: &'a Value,
    inherited: Option<&'a Value>,
    dep: &ModuleDependency,
) -> Option<&'a Value> {
    // Conditional target tables are retained as incomplete by module_manifest;
    // do not reinterpret an arbitrary host target as the selected build.
    let section = match dep.kind {
        TargetKind::Development => "dev-dependencies",
        TargetKind::Build => "build-dependencies",
        _ => "dependencies",
    };
    let entries = value.get(section)?.as_table()?;
    let matches: Vec<_> = entries
        .iter()
        .filter(|(key, _)| key.replace('-', "_") == dep.alias)
        .collect();
    let [(key, original)] = matches.as_slice() else {
        return None;
    };
    if original.get("workspace").and_then(Value::as_bool) == Some(true) {
        inherited?
            .get("workspace")?
            .get("dependencies")?
            .get(key.as_str())
    } else {
        Some(original)
    }
}

fn confined(path: &str) -> bool {
    !path.is_empty()
        && std::path::Path::new(path).components().all(|c| {
            matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
}

#[cfg(test)]
#[path = "registry_modules_tests.rs"]
mod tests;
