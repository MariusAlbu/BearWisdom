//! Standard Grails `g:` namespace taglib contract.
//!
//! Grails ships a fixed set of core tags (link/form/validation/format/render
//! families) in its own framework sources. In a GSP these tags are reachable
//! two ways: as `<g:message .../>` markup and as bare method calls inside a
//! `${...}` expression (`${message(code:'x')}`, `${resource(dir:'i',file:'f')}`).
//! The expression form is sub-parsed as Groovy, so the call surfaces as a bare
//! Groovy `Calls` ref whose target is the tag name.
//!
//! When the Grails framework sources are materialized on disk (Maven/Gradle
//! sources jars) these resolve as ordinary external symbols through the locator
//! pipeline. When they are not — the dependency is declared but unmaterialized —
//! the names have no in-index target and would otherwise count as unresolved.
//! This module declares the finite framework contract as profile data so a bare
//! standard-tag call from a GSP host is branded as a framework builtin rather
//! than mis-counted. Project-defined taglibs are NOT modeled here — they require
//! their own indexed closure symbol or the framework dependency on disk.

/// Standard Grails core tag names invocable as a bare method from a GSP `${...}`
/// expression. Kept in ascending order for binary search (asserted in tests).
/// Codec methods (`encodeAsHTML`) are excluded — those are receiver-chained
/// calls on a value, not bare tags.
const STANDARD_TAGS: &[&str] = &[
    "actionSubmit",
    "applyLayout",
    "checkBox",
    "createLink",
    "createLinkTo",
    "datePicker",
    "each",
    "eachError",
    "external",
    "fieldValue",
    "form",
    "formatBoolean",
    "formatDate",
    "formatNumber",
    "hasErrors",
    "hiddenField",
    "img",
    "include",
    "layoutBody",
    "layoutHead",
    "layoutTitle",
    "link",
    "message",
    "meta",
    "pageProperty",
    "paginate",
    "passwordField",
    "radio",
    "render",
    "renderErrors",
    "resource",
    "select",
    "set",
    "sortableColumn",
    "submitButton",
    "textArea",
    "textField",
    "uploadForm",
];

/// Grails core logical/iteration tags reachable only as `<g:...>` markup
/// (`<g:if>`, `<g:each>`, `<g:set>`). They drive control flow and emit no
/// callable target — most are Groovy keywords, the rest collide with Groovy
/// collection methods (`collect`, `findAll`, `grep`), so they are NOT part of
/// the bare-expression tag contract (`STANDARD_TAGS`). In markup position the
/// name is unambiguously a logical tag, so it is branded a framework builtin
/// there. Kept in ascending order for binary search (asserted in tests).
const LOGICAL_MARKUP_TAGS: &[&str] = &[
    "collect",
    "def",
    "each",
    "else",
    "elseif",
    "embedNode",
    "findAll",
    "grep",
    "if",
    "isSet",
    "join",
    "set",
    "unless",
    "while",
];

/// Returns true when `name` is a standard Grails core tag callable bare from a
/// GSP expression. The caller is responsible for gating on a GSP host and on a
/// receiver-less call — a same-named project method in a plain `.groovy` file
/// must never be declined by this contract.
pub(crate) fn is_standard_grails_tag(name: &str) -> bool {
    STANDARD_TAGS.binary_search(&name).is_ok()
}

/// Returns true when `name` is a Grails core tag reachable as `<g:...>` markup
/// — either a standard rendering tag (`message`/`link`/...) or a logical /
/// iteration tag (`if`/`each`/`set`/...). The markup form carries no receiver
/// ambiguity, so the wider logical set is recognized here that is excluded from
/// the bare-expression contract.
pub(crate) fn is_grails_markup_tag(name: &str) -> bool {
    STANDARD_TAGS.binary_search(&name).is_ok() || LOGICAL_MARKUP_TAGS.binary_search(&name).is_ok()
}

/// A `<ns:tag ...>` markup invocation recovered from a GSP source line: the
/// 0-based byte offset of the `<`, the tag local name, and the line index.
pub(crate) struct MarkupTag {
    pub name: String,
    pub byte_offset: usize,
    pub line: usize,
}

/// Scan GSP source for namespaced markup tags (`<ns:tag ...>`), one entry per
/// opening tag. A taglib invocation is the only markup whose element name
/// carries a `namespace:localName` colon — plain HTML elements never do — so
/// the shape is recognized structurally: a `<`, a lowercase namespace
/// identifier, a `:`, then the tag local name. The closing `</ns:tag>` form is
/// skipped (the open tag already names the invocation). The returned name is
/// the local name, which is what a taglib closure is indexed under; the caller
/// emits a `Calls` ref so a custom taglib binds to its closure definition and a
/// framework tag is branded by `is_grails_markup_tag`.
pub(crate) fn scan_markup_tags(source: &str) -> Vec<MarkupTag> {
    let bytes = source.as_bytes();
    let mut tags: Vec<MarkupTag> = Vec::new();
    let mut line: usize = 0;
    let mut i: usize = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                line += 1;
                i += 1;
                continue;
            }
            b'<' => {
                // Skip a closing tag's slash; the open form already emitted.
                let mut j = i + 1;
                if j < bytes.len() && bytes[j] == b'/' {
                    j += 1;
                }
                let ns_start = j;
                while j < bytes.len() && (bytes[j].is_ascii_lowercase()) {
                    j += 1;
                }
                // Require a non-empty lowercase namespace followed by `:`.
                if j > ns_start && j < bytes.len() && bytes[j] == b':' {
                    let name_start = j + 1;
                    let mut k = name_start;
                    while k < bytes.len()
                        && (bytes[k].is_ascii_alphanumeric() || bytes[k] == b'_')
                    {
                        k += 1;
                    }
                    if k > name_start {
                        // Only the opening form names a fresh invocation.
                        if bytes[i + 1] != b'/' {
                            if let Some(name) = source.get(name_start..k) {
                                tags.push(MarkupTag {
                                    name: name.to_string(),
                                    byte_offset: i,
                                    line,
                                });
                            }
                        }
                        i = k;
                        continue;
                    }
                }
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }
    tags
}

#[cfg(test)]
#[path = "taglib_tests.rs"]
mod taglib_tests;
