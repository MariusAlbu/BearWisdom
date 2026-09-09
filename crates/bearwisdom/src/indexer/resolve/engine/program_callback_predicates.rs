//! Inferred predicates require source-bound, two-sided parameter refinement.
use super::*;

pub(super) fn infer(
    arena: &TypeArena,
    graph: &LexicalBindings,
    callback: &Callback,
    parameters: &[CallableParameter<TypeId>],
) -> Option<Option<CallablePredicate<TypeId>>> {
    let mut body = &callback.body;
    let mut inverted = false;
    let mut boolean_test = false;
    for depth in 0..64 {
        if let Expr::Not(operand) = body {
            if depth == 63 {
                return None;
            }
            inverted = !inverted;
            boolean_test = true;
            body = operand;
        } else {
            break;
        }
    }
    let (operand, test) = match body {
        Expr::TypeTest {
            operand,
            kind,
            negated,
        } => (operand.as_ref(), Some((*kind, *negated ^ inverted))),
        Expr::Read(_) if boolean_test => (body, None),
        _ => return Some(None),
    };
    let Expr::Read(site) = operand else {
        return Some(None);
    };
    let binding = graph.argument_reads.get(site)?;
    let Some(index) = callback
        .parameters
        .iter()
        .position(|site| graph.declarations.get(site) == Some(binding))
    else {
        return Some(None);
    };
    let original = parameters[index].ty;
    let narrowed = if let Some((kind, negated)) = test {
        narrow(arena, original, kind, negated, 0)?
    } else {
        let Some((truthy, falsy)) = partition(arena, original)? else {
            return Some(None);
        };
        if inverted {
            falsy
        } else {
            truthy
        }
    };
    if narrowed == original || matches!(arena.get(narrowed), Type::Intrinsic(Intrinsic::Never)) {
        return Some(None);
    }
    Some(Some(CallablePredicate {
        parameter: parameters[index].declaration,
        asserted: Some(narrowed),
        asserts: false,
    }))
}

// Some(None): a broad domain overlaps both outcomes, so no two-sided predicate
// follows. None: unsupported evidence must not eliminate a predicate overload.
fn partition(arena: &TypeArena, ty: TypeId) -> Option<Option<(TypeId, TypeId)>> {
    let mut pending = vec![ty];
    let mut truthy = Vec::new();
    let mut falsy = Vec::new();
    let mut remaining = 4096usize;
    while let Some(part) = pending.pop() {
        remaining = remaining.checked_sub(1)?;
        let value = match arena.get(part) {
            Type::Union(parts) => {
                pending.extend(parts);
                continue;
            }
            Type::Intrinsic(Intrinsic::Never) => continue,
            Type::Intrinsic(Intrinsic::Boolean) => {
                truthy.push(arena.intern(Type::Literal(LitValue::Bool(true))));
                falsy.push(arena.intern(Type::Literal(LitValue::Bool(false))));
                continue;
            }
            Type::Intrinsic(
                Intrinsic::String
                | Intrinsic::Number
                | Intrinsic::BigInt
                | Intrinsic::Unknown
                | Intrinsic::Any,
            ) => return Some(None),
            Type::Intrinsic(Intrinsic::Null | Intrinsic::Undefined) => false,
            Type::Intrinsic(Intrinsic::Object | Intrinsic::Symbol)
            | Type::UniqueSymbol(_)
            | Type::Decl { .. }
            | Type::Apply { .. }
            | Type::Tuple(_)
            | Type::Callable(_) => true,
            Type::Literal(LitValue::Bool(value)) => value,
            Type::Literal(LitValue::Str(value)) => !value.is_empty(),
            Type::Literal(LitValue::Utf16(value)) => !value.is_empty(),
            Type::Literal(LitValue::Int(value)) => value != 0,
            Type::Literal(LitValue::Number(bits)) => {
                let value = f64::from_bits(bits);
                value != 0.0 && !value.is_nan()
            }
            Type::Literal(LitValue::BigInt { words, .. }) => !words.is_empty(),
            _ => return None,
        };
        if value {
            truthy.push(part);
        } else {
            falsy.push(part);
        }
    }
    // The compiler deliberately skips a parameter whose complete type is
    // boolean, including the equivalent true|false representation.
    if truthy
        .iter()
        .chain(&falsy)
        .all(|&ty| matches!(arena.get(ty), Type::Literal(LitValue::Bool(_))))
    {
        return Some(None);
    }
    let union = |mut parts: Vec<TypeId>| {
        parts.sort_unstable();
        parts.dedup();
        match parts.len() {
            0 => arena.intern(Type::Intrinsic(Intrinsic::Never)),
            1 => parts[0],
            _ => arena.intern(Type::Union(parts)),
        }
    };
    Some(Some((union(truthy), union(falsy))))
}
