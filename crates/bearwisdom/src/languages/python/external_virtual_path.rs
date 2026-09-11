//! PyPI and Python standard-library virtual identities.

pub(crate) fn for_pulled(path: &str) -> Option<String> {
    if let Some(index) = path.rfind("/site-packages/") {
        return from_remainder(&path[index + "/site-packages/".len()..]);
    }
    let index = path.rfind("/Lib/")?;
    from_remainder(&path[index + "/Lib/".len()..])
}

fn from_remainder(relative: &str) -> Option<String> {
    if relative.is_empty() {
        return None;
    }
    if relative.contains('/') {
        return Some(format!("ext:py:{relative}"));
    }
    let stem = relative
        .strip_suffix(".pyi")
        .or_else(|| relative.strip_suffix(".py"))?;
    (!stem.is_empty()).then(|| format!("ext:py:{stem}/{relative}"))
}
