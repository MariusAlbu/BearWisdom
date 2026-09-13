// Tests for chain.rs — the receiver path a Ruby call expression states.

use super::super::extract;
use crate::types::{EdgeKind, SegmentKind};

/// The `(name, kind)` of every segment of the chain on the `Calls` ref to
/// `target`.
fn chain_of(source: &str, target: &str) -> Vec<(String, SegmentKind)> {
    let r = extract::extract(source);
    let reference = r
        .refs
        .iter()
        .find(|reference| reference.kind == EdgeKind::Calls && reference.target_name == target)
        .unwrap_or_else(|| panic!("no call to {target} in {:?}", r.refs));
    reference
        .chain
        .as_ref()
        .map(|c| {
            c.segments
                .iter()
                .map(|s| (s.name.clone(), s.kind))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn an_instance_variable_receiver_starts_at_the_implicit_receiver() {
    let source = "class Svc\n  def run\n    @cache.get\n  end\nend\n";
    assert_eq!(
        chain_of(source, "get"),
        vec![
            ("self".to_string(), SegmentKind::SelfRef),
            ("@cache".to_string(), SegmentKind::Property),
            ("get".to_string(), SegmentKind::Property),
        ]
    );
}

#[test]
fn an_explicit_self_receiver_states_the_same_root() {
    let source = "class Svc\n  def run\n    self.cache\n  end\nend\n";
    assert_eq!(
        chain_of(source, "cache"),
        vec![
            ("self".to_string(), SegmentKind::SelfRef),
            ("cache".to_string(), SegmentKind::Property),
        ]
    );
}

#[test]
fn a_local_receiver_roots_on_its_own_name() {
    let source = "class Svc\n  def run(cache)\n    cache.get\n  end\nend\n";
    assert_eq!(
        chain_of(source, "get"),
        vec![
            ("cache".to_string(), SegmentKind::Identifier),
            ("get".to_string(), SegmentKind::Property),
        ]
    );
}

#[test]
fn a_constant_receiver_roots_on_the_constant() {
    let source = "class Svc\n  def run\n    Cache.build\n  end\nend\n";
    assert_eq!(
        chain_of(source, "build"),
        vec![
            ("Cache".to_string(), SegmentKind::Identifier),
            ("build".to_string(), SegmentKind::Property),
        ]
    );
}

#[test]
fn a_chained_instance_variable_keeps_every_hop() {
    let source = "class Svc\n  def run\n    @inbox.channel.name\n  end\nend\n";
    assert_eq!(
        chain_of(source, "name"),
        vec![
            ("self".to_string(), SegmentKind::SelfRef),
            ("@inbox".to_string(), SegmentKind::Property),
            ("channel".to_string(), SegmentKind::Property),
            ("name".to_string(), SegmentKind::Property),
        ]
    );
}
