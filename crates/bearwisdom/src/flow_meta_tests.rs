use super::*;

#[test]
fn metadata_clone_keeps_lexical_opt_in_and_legacy_defaults() {
    let mut flow = FlowMeta::default();
    assert!(flow.lexical.is_none());
    assert!(flow.callback_lexical.is_none());
    flow.lexical = Some(crate::indexer::lexical::LexicalBindings::default());
    flow.callback_lexical = Some(crate::indexer::lexical::LexicalBindings::default());
    assert!(flow.clone().lexical.is_some());
    assert!(flow.clone().callback_lexical.is_some());
}
