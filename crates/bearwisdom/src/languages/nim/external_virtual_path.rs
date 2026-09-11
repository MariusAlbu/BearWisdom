//! Nimble package and Nim standard-library virtual identities.

pub(crate) fn for_pulled(path: &str) -> Option<String> {
    if let Some(index) = path.rfind("/pkgs2/") {
        let after = &path[index + "/pkgs2/".len()..];
        let slash = after.find('/')?;
        let directory = &after[..slash];
        let relative = &after[slash + 1..];
        let package = directory
            .split('-')
            .take_while(|part| part.chars().next().is_some_and(|c| !c.is_ascii_digit()))
            .collect::<Vec<_>>()
            .join("-");
        if !package.is_empty() && !relative.is_empty() {
            return Some(format!("ext:nim:{package}/{relative}"));
        }
    }
    let index = path.rfind("/lib/")?;
    let relative = &path[index + "/lib/".len()..];
    (relative.ends_with(".nim") && !path.contains("/site-packages/"))
        .then(|| format!("ext:nim:nim-stdlib/{relative}"))
}
