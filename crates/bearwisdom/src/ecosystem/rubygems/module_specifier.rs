//! RubyGems module-entry keys for RubyGems virtual source paths.

const EXTERNAL_PATH_PREFIX: &str = "ext:ruby:";

/// Ruby's require spelling starts from the gem name. The virtual-path scheme
/// and root-segment grammar are owned here, not by generic entry population.
pub(crate) fn package_entry_key(path: &str) -> Option<String> {
    path.strip_prefix(EXTERNAL_PATH_PREFIX)?
        .split('/')
        .next()
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::package_entry_key;

    #[test]
    fn accepts_only_rubygems_virtual_paths() {
        assert_eq!(
            package_entry_key("ext:ruby:devise/lib/devise.rb").as_deref(),
            Some("devise")
        );
        assert_eq!(package_entry_key("ext:ts:devise/index.d.ts"), None);
    }
}
