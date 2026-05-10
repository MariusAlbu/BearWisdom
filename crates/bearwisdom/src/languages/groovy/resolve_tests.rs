// =============================================================================
// groovy/resolve_tests.rs — unit tests for GroovyResolver
// =============================================================================

use super::resolve::GroovyResolver;
use crate::indexer::resolve::engine::{FileContext, LanguageResolver};

#[test]
fn groovy_resolver_declares_only_groovy_language() {
    let r = GroovyResolver;
    assert_eq!(r.language_ids(), &["groovy"]);
}
