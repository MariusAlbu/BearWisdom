//! Structural inference collects source-owned candidates; applicability follows.
use super::*;

pub(super) fn infer_members(
    relation: &Relation,
    signature: &Bound,
    expected: TypeId,
    actual: TypeId,
    bindings: &mut FxHashMap<GenericParamId, TypeId>,
    site: InferenceSite<'_>,
) -> Option<bool> {
    let target = relation.inference_members(expected)?;
    let source = relation.inference_members(actual)?;
    let pair = (expected, actual);
    if !site.active.borrow_mut().insert(pair) {
        return Some(false);
    }
    let result = (|| {
        let mut inferred = false;
        for member in target {
            if member.property.index {
                return None;
            }
            let expected = relation.member_value(&member)?;
            if !inference_parameters(
                relation.arena,
                expected,
                &signature.generic_parameters,
                site.depth + 1,
            )? {
                continue;
            }
            let mut matches = source
                .iter()
                .filter(|m| !m.property.index && m.property.key == member.property.key);
            let Some(actual) = matches.next() else {
                continue;
            };
            // Overload inference needs selected declaration-group ordering.
            // Applicability independently covers every target overload.
            if matches.next().is_some() {
                return None;
            }
            let actual = relation.member_value(actual)?;
            inferred |= infer(
                relation,
                signature,
                expected,
                actual,
                bindings,
                site.nested(),
            )?;
        }
        Some(inferred)
    })();
    site.active.borrow_mut().remove(&pair);
    result
}
