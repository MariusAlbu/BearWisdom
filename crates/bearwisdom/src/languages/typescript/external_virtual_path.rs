//! npm-shaped virtual identities owned by the TypeScript/JavaScript family.

pub(crate) fn for_pulled(path: &str) -> Option<String> {
    let after = if let Some(index) = path.rfind("/node_modules/") {
        path[index + "/node_modules/".len()..].to_owned()
    } else {
        let root = std::env::var_os("BEARWISDOM_TS_NODE_MODULES")?;
        let root = root.to_string_lossy().replace('\\', "/");
        path.strip_prefix(&format!("{}/", root.trim_end_matches('/')))?
            .to_owned()
    };
    let parts: Vec<_> = after.splitn(4, '/').collect();
    let (package, relative) = if parts.first()?.starts_with('@') && parts.len() >= 3 {
        (format!("{}/{}", parts[0], parts[1]), parts[2..].join("/"))
    } else {
        (parts[0].to_owned(), parts[1..].join("/"))
    };
    crate::ecosystem::npm::is_valid_npm_module_path(&package)
        .then(|| format!("ext:ts:{package}/{relative}"))
}
