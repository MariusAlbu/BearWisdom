//! RubyGems and Ruby standard-library virtual identities.

pub(crate) fn for_pulled(path: &str) -> Option<String> {
    if let Some(index) = path.rfind("/gems/") {
        if let Some((directory, relative)) = path[index + "/gems/".len()..].split_once('/') {
            if let Some((name, _)) = crate::ecosystem::cargo::split_crate_dir_name(directory) {
                if !relative.is_empty() {
                    return Some(format!("ext:ruby:{name}/{relative}"));
                }
            }
        }
    }
    let index = path.rfind("/lib/ruby/")?;
    let (_, relative) = path[index + "/lib/ruby/".len()..].split_once('/')?;
    (!relative.is_empty()).then(|| format!("ext:ruby-stdlib:{relative}"))
}
