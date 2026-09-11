//! Go module-cache and standard-library virtual identities.

pub(crate) fn for_pulled(path: &str) -> Option<String> {
    if let Some(index) = path.find("/pkg/mod/") {
        return Some(format!("ext:go/{}", &path[index + "/pkg/mod/".len()..]));
    }
    let index = path.find("/src/")?;
    let relative = &path[index + "/src/".len()..];
    (!relative.is_empty()).then(|| format!("ext:go-stdlib/{relative}"))
}
