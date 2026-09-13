// =============================================================================
// type_checker/profile/name_spelling.rs — source-spelling helpers
//
// Every method here translates between a language's SOURCE qualification
// spelling and the index's canonical dotted qname surface. They are inherent
// methods on `LanguageProfile`, so call sites do not name this module.
// =============================================================================

use std::borrow::Cow;

use super::language_profile::*;

impl LanguageProfile {
    /// Whether a source call target contains a spelling this language uses for
    /// member or qualified-chain access.  Extractors must represent such a
    /// call with `MemberChain` rather than leaving a compound target string.
    pub fn call_target_requires_member_chain(&self, target: &str) -> bool {
        self.member_chain_markers
            .iter()
            .any(|marker| !marker.is_empty() && target.contains(marker))
    }

    /// Split a source-qualified name at its first language-owned separator.
    /// Returns `None` when this profile has no separator or the spelling is
    /// not source-qualified.
    pub fn split_source_qualified_name<'a>(&self, name: &'a str) -> Option<(&'a str, &'a str)> {
        (!self.qname_separator.is_empty())
            .then(|| name.split_once(self.qname_separator))
            .flatten()
    }

    /// Return source-qualified name components under this profile.  A bare
    /// name is returned as one component; an empty component denotes malformed
    /// source qualification and lets callers decline it.
    pub fn source_qualified_name_parts<'a>(&self, name: &'a str) -> Vec<&'a str> {
        if self.qname_separator.is_empty() {
            vec![name]
        } else {
            name.split(self.qname_separator).collect()
        }
    }

    /// Join source-qualified components with the language-owned separator.
    pub fn join_source_qualified_name<'a, I>(&self, parts: I) -> String
    where
        I: IntoIterator<Item = &'a str>,
    {
        parts
            .into_iter()
            .collect::<Vec<_>>()
            .join(self.qname_separator)
    }

    /// Adapt a workspace module specifier into neutral slash-separated path
    /// evidence when this profile's self-package syntax uses a source
    /// qualification separator. Existing slash-shaped package specifiers are
    /// preserved verbatim.
    pub fn workspace_specifier_path<'a>(&self, specifier: &'a str) -> Cow<'a, str> {
        if self.imports.self_package_root.is_some()
            && !self.qname_separator.is_empty()
            && self.qname_separator != "/"
            && specifier.contains(self.qname_separator)
        {
            Cow::Owned(specifier.replace(self.qname_separator, "/"))
        } else {
            Cow::Borrowed(specifier)
        }
    }

    /// Identify a self-package specifier and return its optional sub-path.
    /// `Some(None)` is the package root, `Some(Some(path))` is a descendant,
    /// and `None` means this specifier belongs to normal package lookup.
    pub fn self_package_sub_path(&self, specifier: &str) -> Option<Option<String>> {
        let keyword = self.imports.self_package_root?;
        let rest = specifier.strip_prefix(keyword)?;
        if rest.is_empty() {
            return Some(None);
        }
        rest.strip_prefix(self.qname_separator)
            .or_else(|| rest.strip_prefix('/'))
            .map(|sub| Some(sub.to_string()))
    }

    /// Package-alias head of a workspace specifier. Only profiles with explicit
    /// self-package qualification may split source syntax here; other module
    /// grammars keep the whole specifier, including dots in package names.
    pub fn workspace_alias_head<'a>(&self, specifier: &'a str) -> Option<&'a str> {
        if self.imports.self_package_root.is_none() {
            return (!specifier.is_empty()).then_some(specifier);
        }
        let head = self
            .split_source_qualified_name(specifier)
            .map(|(head, _)| head)
            .unwrap_or(specifier);
        (!head.is_empty()).then_some(head)
    }

    /// The declared receiver meaning of an exact source spelling. Does not
    /// infer from the spelling, so an unconfigured language fails closed.
    pub fn receiver_role(&self, spelling: &str) -> Option<ReceiverRole> {
        self.receiver_spellings
            .iter()
            .find(|entry| entry.spelling == spelling)
            .and_then(|entry| entry.role)
    }

    /// Whether a source spelling is declared as a receiver or prefix form.
    pub fn has_receiver_spelling(&self, spelling: &str) -> bool {
        self.receiver_spellings
            .iter()
            .any(|entry| entry.spelling == spelling)
    }

    /// Normalize one profile-declared receiver/prefix member expression to
    /// its member target. The language declares both the prefix and separator
    /// (`.`, `::`, `->`, …); the generic resolver strips neither itself.
    pub fn normalize_receiver_member_target<'a>(&self, target: &'a str) -> &'a str {
        self.receiver_spellings
            .iter()
            .find_map(|entry| {
                target
                    .strip_prefix(entry.spelling)
                    .and_then(|tail| tail.strip_prefix(entry.member_separator))
            })
            .unwrap_or(target)
    }

    /// Ask the owning language whether a primitive head has a nominal member
    /// surface. This is deliberately not a capitalization heuristic.
    pub fn primitive_member_head(&self, head: &str) -> Option<String> {
        crate::languages::default_registry()
            .get_dedicated(self.id)
            .and_then(|plugin| plugin.primitive_member_head(head))
    }

    /// Whether this language explicitly declares an applied head to be a
    /// homogeneous container for computed-access projection.
    pub fn has_homogeneous_computed_access(&self, head: &str) -> bool {
        crate::languages::default_registry()
            .get_dedicated(self.id)
            .is_some_and(|plugin| plugin.has_homogeneous_computed_access(head))
    }

    /// Adapt a source module specifier into generic file-path evidence. The
    /// callback itself is always owned by the language/ecosystem profile.
    pub fn module_path_match(&self, module: &str) -> ModulePathMatch {
        let matched = if let Some(config) = self.chain_qualification.qualified_import_root() {
            (config.module_path_adapter)(module)
        } else if let ModulePrefixRewrites::On {
            module_path_adapter: Some(adapter),
            ..
        } = self.imports.module_prefix_rewrites
        {
            adapter(module)
        } else {
            ModulePathMatch::heuristic(module)
        };
        matched.with_source_separator_path_variant(module, self.qname_separator)
    }

    /// Re-export source-path evidence for one module spelling. Adapters own
    /// extension and bare-module conventions; the generic walker only applies
    /// the returned callbacks to indexed paths.
    pub fn source_module_path_policy(&self, source_module: &str) -> SourceModulePathPolicy {
        if let Some(config) = self.chain_qualification.qualified_import_root() {
            return (config.module_path_adapter)(source_module).source_module_path_policy;
        }
        if let ModulePrefixRewrites::On {
            module_path_adapter: Some(adapter),
            ..
        } = self.imports.module_prefix_rewrites
        {
            return adapter(source_module).source_module_path_policy;
        }
        crate::languages::default_registry()
            .get_dedicated(self.id)
            .map(|plugin| plugin.source_module_path_policy(source_module))
            .unwrap_or_else(SourceModulePathPolicy::unsupported)
    }

    /// Whether module-anchor binding treats this source module as relative.
    /// The profile owns the marker spelling; resolver rules consume only this
    /// normalized decision.
    pub fn module_anchor_is_relative(&self, module: &str) -> bool {
        self.imports.relative_marker.is_relative(module)
    }

    /// Whether source text denotes a qualified name or a path-shaped name under
    /// this profile. Physical `/` path components remain neutral evidence.
    pub fn is_qualified_name(&self, name: &str) -> bool {
        name.contains('/')
            || (!self.qname_separator.is_empty() && name.contains(self.qname_separator))
    }

    /// Whether a name contains this language's qualified-name separator.
    /// URI grammar treats physical `/` path components independently, so it
    /// must not use `is_qualified_name` here.
    pub fn has_qualified_separator(&self, name: &str) -> bool {
        !self.qname_separator.is_empty() && name.contains(self.qname_separator)
    }

    /// The unqualified leaf after a neutral path component and this profile's
    /// qualified-name separator.
    pub fn simple_name<'a>(&self, name: &'a str) -> &'a str {
        let path_leaf = name.rsplit('/').next().unwrap_or(name);
        if self.qname_separator.is_empty() {
            path_leaf
        } else {
            path_leaf
                .rsplit(self.qname_separator)
                .next()
                .unwrap_or(path_leaf)
        }
    }

    /// Convert a source-qualified name to the resolver's canonical index key.
    /// Source separators are profile data; the dot is only the index storage
    /// separator and must not be used to parse source text in engine rules.
    pub fn index_qname_from_source(&self, name: &str) -> String {
        // A leading separator anchors the name at the root namespace (`\Foo`,
        // `::foo`); the index has no root segment, so it contributes nothing.
        let name = if self.qname_separator.is_empty() {
            name
        } else {
            name.strip_prefix(self.qname_separator).unwrap_or(name)
        };
        let qualified = if self.qname_separator.is_empty() || self.qname_separator == "." {
            name.to_string()
        } else {
            name.replace(self.qname_separator, ".")
        };
        qualified.replace('/', ".")
    }

    /// Convert source module/name spelling into a neutral path fragment for
    /// indexed-file evidence. Source separators stay profile-owned; `/` here
    /// denotes physical path components only.
    pub fn index_qname_path_from_source(&self, name: &str) -> String {
        self.index_qname_from_source(name).replace('.', "/")
    }

    /// Form a canonical index qname from source-spelled prefix and leaf.
    pub fn index_qname_join(&self, prefix: &str, leaf: &str) -> String {
        let prefix = self.index_qname_from_source(prefix);
        let leaf = self.index_qname_from_source(leaf);
        if prefix.is_empty() {
            leaf
        } else if leaf.is_empty() {
            prefix
        } else {
            format!("{prefix}.{leaf}")
        }
    }
}
