//! Rest parameters retain finite tuple correlation and configured array tails.
use super::*;

pub(super) struct Parameters {
    fixed: Vec<(TypeId, bool)>,
    finite: Option<TypeId>,
    variadic: Option<TypeId>,
    variadic_rest: Option<TypeId>,
    contextual: Option<TypeId>,
}
impl Parameters {
    pub(super) fn count(&self) -> usize {
        self.fixed.len()
            + usize::from(
                self.finite.is_some() || self.variadic.is_some() || self.contextual.is_some(),
            )
    }
    pub(super) fn minimum(&self) -> usize {
        self.fixed
            .iter()
            .rposition(|(_, optional)| !optional)
            .map_or(0, |i| i + 1)
    }
    fn has_rest(&self) -> bool {
        self.finite.is_some() || self.variadic.is_some() || self.contextual.is_some()
    }
    pub(super) fn parameter(&self, index: usize) -> Option<(TypeId, bool)> {
        self.fixed
            .get(index)
            .copied()
            .or_else(|| self.variadic.map(|ty| (ty, false)))
    }
    pub(super) fn has_finite(&self) -> bool {
        self.finite.is_some()
    }
    pub(super) fn fixed_count(&self) -> usize {
        self.fixed.len()
    }
    pub(super) fn contextual_rest(&self) -> Option<TypeId> {
        self.contextual
    }
}

impl Eval<'_, '_> {
    pub(super) fn canonical_rest(&mut self, ty: TypeId, depth: usize) -> Option<TypeId> {
        self.spend(depth)?;
        match self.arena().get(ty) {
            Type::Tuple(items) => {
                let mut mapped = Vec::new();
                for item in items {
                    mapped.push(if let Type::Optional(inner) = self.arena().get(item) {
                        let inner = self.ty(inner, depth + 1)?;
                        self.arena().intern(Type::Optional(inner))
                    } else {
                        self.ty(item, depth + 1)?
                    });
                }
                Some(self.arena().intern(Type::Tuple(mapped)))
            }
            Type::Union(items) => {
                let items = items
                    .into_iter()
                    .map(|ty| self.canonical_rest(ty, depth + 1))
                    .collect::<Option<Vec<_>>>()?;
                self.union(items, depth + 1)
            }
            Type::Operator(TypeOperator::Readonly(inner)) => {
                let inner = self.canonical_rest(inner, depth + 1)?;
                Some(
                    self.arena()
                        .intern(Type::Operator(TypeOperator::Readonly(inner))),
                )
            }
            _ => self.ty(ty, depth + 1),
        }
    }

    pub(super) fn callable_parameters(
        &mut self,
        callable: &Callable<TypeId>,
        depth: usize,
    ) -> Option<Parameters> {
        let mut result = Parameters {
            fixed: vec![],
            finite: None,
            variadic: None,
            variadic_rest: None,
            contextual: None,
        };
        let mut slots = FxHashSet::default();
        let mut receiver = false;
        for (index, p) in callable.parameters.iter().enumerate() {
            self.spend(depth)?;
            if !slots.insert(p.declaration) {
                return None;
            }
            if p.receiver {
                if receiver || index != 0 || p.optional || p.rest {
                    return None;
                }
                receiver = true;
                continue;
            }
            if !p.rest {
                result.fixed.push((p.ty, p.optional));
                continue;
            }
            if index + 1 != callable.parameters.len() || p.optional {
                return None;
            }
            match self.arena().get(p.ty) {
                Type::Generic { .. } => result.contextual = Some(p.ty),
                Type::Tuple(items) => {
                    for item in items {
                        self.spend(depth)?;
                        result.fixed.push(match self.arena().get(item) {
                            Type::Optional(ty) => (ty, true),
                            Type::Unknown => return None,
                            _ => (item, false),
                        });
                    }
                }
                Type::Union(parts)
                    if parts
                        .iter()
                        .all(|&ty| matches!(self.arena().get(ty), Type::Tuple(_))) =>
                {
                    result.finite = Some(p.ty)
                }
                _ => {
                    result.variadic =
                        Some(arrays::shape(self.relation.lookup, self.arena(), p.ty)?.element);
                    result.variadic_rest = Some(p.ty);
                }
            }
        }
        Some(result)
    }

    pub(super) fn contextual_tail(
        &mut self,
        parameters: &Parameters,
        start: usize,
        depth: usize,
    ) -> Option<TypeId> {
        if let Some(rest) = parameters.variadic_rest {
            // A fixed prefix followed by an array rest would need a variadic
            // tuple representation. At or beyond the array boundary the
            // compiler supplies the original array rest type.
            if start < parameters.fixed.len() {
                return None;
            }
            return Some(rest);
        }
        if let Some(rest) = parameters.contextual {
            if start < parameters.fixed.len() {
                return None;
            }
            return Some(rest);
        }
        self.parameter_tail(parameters, start, depth)
    }

    pub(super) fn signature_parameters(
        &mut self,
        source: &Callable<TypeId>,
        target: &Callable<TypeId>,
        source_minimum: Option<usize>,
        policy: CallablePolicy,
        mode: Mode,
        depth: usize,
    ) -> Option<bool> {
        let a = self.callable_parameters(source, depth + 1)?;
        let b = self.callable_parameters(target, depth + 1)?;
        if a.contextual.is_some() || b.contextual.is_some() {
            return None;
        }
        if !b.has_rest() && source_minimum.unwrap_or_else(|| a.minimum()) > b.count() {
            return Some(false);
        }
        // A union of finite tuple tails is compared as a correlated remainder.
        // An array tail follows the compiler's ordinary positional path: its
        // configured element type repeats at every position beyond the prefix.
        let finite = a.finite.is_some() || b.finite.is_some();
        let count = if finite {
            a.count().min(b.count())
        } else {
            a.count().max(b.count())
        };
        for i in 0..count {
            self.spend(depth)?;
            if finite && i + 1 == count {
                let from = self.parameter_tail(&a, i, depth + 1)?;
                let to = self.parameter_tail(&b, i, depth + 1)?;
                let result = self.rest_value(to, from, policy, depth + 1);
                let bivariant = matches!(mode, Mode::Ordinary | Mode::Method)
                    && !strict_variance(policy, mode)?;
                let result = if bivariant && result != Some(true) {
                    either(result, self.rest_value(from, to, policy, depth + 1))
                } else {
                    result
                };
                if !result? {
                    return Some(false);
                }
            } else if let (Some((a, oa)), Some((b, ob))) = (a.parameter(i), b.parameter(i)) {
                let read = |ty, optional| {
                    if optional && policy.strict_nulls {
                        self.arena().intern(Type::Optional(ty))
                    } else {
                        ty
                    }
                };
                if !self.callable_parameter(read(a, oa), read(b, ob), policy, mode, depth + 1)? {
                    return Some(false);
                }
            }
        }
        Some(true)
    }

    fn parameter_tail(
        &mut self,
        parameters: &Parameters,
        start: usize,
        depth: usize,
    ) -> Option<TypeId> {
        if parameters.variadic.is_some() {
            return None;
        }
        let prefix: Vec<_> = parameters
            .fixed
            .get(start..)
            .unwrap_or(&[])
            .iter()
            .map(|&(ty, optional)| {
                if optional {
                    self.arena().intern(Type::Optional(ty))
                } else {
                    ty
                }
            })
            .collect();
        let parts = match parameters.finite {
            None => vec![self.arena().intern(Type::Tuple(vec![]))],
            Some(ty) => match self.arena().get(ty) {
                Type::Union(parts) => parts,
                _ => return None,
            },
        };
        let mut result = Vec::new();
        for part in parts {
            self.spend(depth)?;
            let Type::Tuple(items) = self.arena().get(part) else {
                return None;
            };
            let mut combined = prefix.clone();
            combined.extend(items);
            result.push(self.arena().intern(Type::Tuple(combined)));
        }
        self.union(result, depth + 1)
    }

    pub(super) fn rest_value(
        &mut self,
        from: TypeId,
        to: TypeId,
        policy: CallablePolicy,
        depth: usize,
    ) -> Option<bool> {
        self.spend(depth)?;
        let a = self.arena().get(from);
        let b = self.arena().get(to);
        if let Type::Union(parts) = a {
            let mut unknown = false;
            for part in parts {
                match self.rest_value(part, to, policy, depth + 1) {
                    Some(false) => return Some(false),
                    None => unknown = true,
                    _ => {}
                }
            }
            return if unknown { None } else { Some(true) };
        }
        if let Type::Union(parts) = b {
            let mut unknown = false;
            for part in parts {
                match self.rest_value(from, part, policy, depth + 1) {
                    Some(true) => return Some(true),
                    None => unknown = true,
                    _ => {}
                }
            }
            return if unknown { None } else { Some(false) };
        }
        let (Type::Tuple(a), Type::Tuple(b)) = (a, b) else {
            return None;
        };
        let minimum = |items: &[TypeId]| {
            items
                .iter()
                .rposition(|&ty| !matches!(self.arena().get(ty), Type::Optional(_)))
                .map_or(0, |i| i + 1)
        };
        if minimum(&a) < minimum(&b) || a.len() > b.len() {
            return Some(false);
        }
        for (a, b) in a.into_iter().zip(b) {
            if !self.callable_value(a, b, policy, depth + 1)? {
                return Some(false);
            }
        }
        Some(true)
    }
}
