//! Cargo registry virtual identities owned by Rust.

pub(crate) fn for_pulled(path: &str) -> Option<String> {
    let index = path.find("/registry/src/")?;
    let after = &path[index + "/registry/src/".len()..];
    let (_, after_index) = after.split_once('/')?;
    let (crate_dir, relative) = after_index.split_once('/')?;
    let (name, _) = crate::ecosystem::cargo::split_crate_dir_name(crate_dir)?;
    (!relative.is_empty()).then(|| format!("ext:rust:{name}/{relative}"))
}
