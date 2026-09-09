//! Object/key operations retain all operands and modifiers, never display names.
use super::*;

impl Eval<'_, '_> {
    pub(super) fn operator(&mut self, op: TypeOperator<TypeId>, depth: usize) -> Option<TypeId> {
        self.spend(depth)?;
        match op {
            TypeOperator::Object(properties) => {
                let properties = properties
                    .into_iter()
                    .map(|p| {
                        Some(TypeProperty {
                            key: self.ty(p.key, depth + 1)?,
                            value: self.ty(p.value, depth + 1)?,
                            ..p
                        })
                    })
                    .collect::<Option<Vec<_>>>()?;
                self.object(properties, depth + 1)
            }
            TypeOperator::KeyOf(object) => {
                let object = self.ty(object, depth + 1)?;
                let properties = self.key_properties(object, depth + 1)?;
                let mut keys = Vec::new();
                for p in properties {
                    keys.push(p.key);
                    if p.index && p.key == self.atom(Intrinsic::String) {
                        keys.push(self.atom(Intrinsic::Number));
                    }
                }
                self.union(keys, depth + 1)
            }
            TypeOperator::IndexedAccess { object, index } => {
                let object = self.ty(object, depth + 1)?;
                let index = self.ty(index, depth + 1)?;
                self.indexed(object, index, depth + 1)
            }
            TypeOperator::Mapped {
                parameter,
                keys,
                remap,
                value,
                optional,
                readonly,
            } => {
                let Type::Generic { param } = self.arena().get(parameter) else {
                    return None;
                };
                self.constraint(param)?;
                // Keep the unevaluated keyof origin: a flattened key union no
                // longer carries the source property's optional/readonly axes.
                let origin = match self.arena().get(keys) {
                    Type::Operator(TypeOperator::KeyOf(object)) => {
                        let object = self.ty(object, depth + 1)?;
                        let properties = self.key_properties(object, depth + 1)?;
                        Some(properties)
                    }
                    _ => None,
                };
                let keys = self.ty(keys, depth + 1)?;
                let keys = match self.arena().get(keys) {
                    Type::Union(keys) => keys,
                    Type::Intrinsic(Intrinsic::Never) => vec![],
                    _ => vec![keys],
                };
                if keys.is_empty() {
                    // Empty output is not evidence that an unknown body or
                    // remapping expression was valid. Preserve that barrier.
                    self.ty(value, depth + 1)?;
                    if let Some(remap) = remap {
                        self.ty(remap, depth + 1)?;
                    }
                }
                let mut properties: Vec<TypeProperty<TypeId>> = Vec::new();
                for key in keys {
                    self.spend(depth)?;
                    if !singleton(self.arena(), key) {
                        return None;
                    }
                    let source = origin
                        .as_ref()
                        .and_then(|p| p.iter().find(|p| p.key == key && !p.index));
                    let bindings = FxHashMap::from_iter([(param, key)]);
                    let mut result =
                        self.ty(substitute(self.arena(), value, &bindings), depth + 1)?;
                    if optional == MappedModifier::Remove && source.is_some_and(|p| p.optional) {
                        result = self.remove_undefined(result, depth + 1)?;
                    }
                    let mapped = match remap {
                        Some(remap) => {
                            self.ty(substitute(self.arena(), remap, &bindings), depth + 1)?
                        }
                        None => key,
                    };
                    let targets = match self.arena().get(mapped) {
                        Type::Union(keys) => keys,
                        Type::Intrinsic(Intrinsic::Never) => vec![],
                        _ => vec![mapped],
                    };
                    for target in targets {
                        self.spend(depth)?;
                        if !singleton(self.arena(), target) {
                            return None;
                        }
                        let optional = modifier(optional, source.is_some_and(|p| p.optional));
                        let readonly = modifier(readonly, source.is_some_and(|p| p.readonly));
                        if let Some(prior) = properties.iter_mut().find(|p| p.key == target) {
                            // Mixed source modifiers under key collision require
                            // an independently attested combination policy.
                            if prior.optional != optional || prior.readonly != readonly {
                                return None;
                            }
                            prior.value = self.union(vec![prior.value, result], depth + 1)?;
                        } else {
                            properties.push(TypeProperty {
                                key: target,
                                value: result,
                                optional,
                                readonly,
                                index: false,
                            });
                        }
                    }
                }
                self.object(properties, depth + 1)
            }
            TypeOperator::Conditional {
                check,
                extends,
                when_true,
                when_false,
                distributive,
            } => self.conditional(
                check,
                extends,
                when_true,
                when_false,
                distributive,
                depth + 1,
            ),
            TypeOperator::Readonly(inner) => {
                // Optional tuple slots carry arity, not just undefined in a
                // required position. Preserve that axis during normalization.
                let inner = if let Type::Tuple(items) = self.arena().get(inner) {
                    let mut mapped = Vec::new();
                    for item in items {
                        mapped.push(if let Type::Optional(value) = self.arena().get(item) {
                            let value = self.ty(value, depth + 1)?;
                            self.arena().intern(Type::Optional(value))
                        } else {
                            self.ty(item, depth + 1)?
                        });
                    }
                    self.arena().intern(Type::Tuple(mapped))
                } else {
                    self.ty(inner, depth + 1)?
                };
                arrays::readonly(self.relation.lookup, self.arena(), inner)
            }
            // Infer is a pattern binder, not an independently evaluable type.
            TypeOperator::Infer(_) => None,
        }
    }

    pub(super) fn object(
        &mut self,
        mut properties: Vec<TypeProperty<TypeId>>,
        depth: usize,
    ) -> Option<TypeId> {
        self.spend(depth)?;
        properties.sort_by_key(|p| (p.key, p.index));
        if properties.windows(2).any(|p| p[0].key == p[1].key) {
            return None;
        }
        for property in &properties {
            self.spend(depth)?;
            if property.index {
                if property.optional
                    || !matches!(
                        self.arena().get(property.key),
                        Type::Intrinsic(Intrinsic::String | Intrinsic::Number | Intrinsic::Symbol)
                    )
                {
                    return None;
                }
            } else if !singleton(self.arena(), property.key) {
                return None;
            }
        }
        for index in properties.iter().filter(|p| p.index) {
            for property in &properties {
                if !property.index
                    && index.key == self.atom(Intrinsic::Number)
                    && matches!(
                        self.arena().get(property.key),
                        Type::Literal(LitValue::Str(_) | LitValue::Utf16(_))
                    )
                {
                    return None;
                }
                if property.key == index.key || !key_in(self.arena(), property.key, index.key) {
                    continue;
                }
                self.spend(depth)?;
                if !self.relation.assign(
                    self.relation.read_property(property),
                    index.value,
                    depth + 1,
                ) {
                    return None;
                }
            }
        }
        Some(
            self.arena()
                .intern(Type::Operator(TypeOperator::Object(properties))),
        )
    }

    pub(super) fn indexed(
        &mut self,
        object: TypeId,
        index: TypeId,
        depth: usize,
    ) -> Option<TypeId> {
        self.spend(depth)?;
        if let Type::Union(objects) = self.arena().get(object) {
            let values = objects
                .into_iter()
                .map(|object| self.indexed(object, index, depth + 1))
                .collect::<Option<Vec<_>>>()?;
            return self.union(values, depth + 1);
        }
        if let Type::Union(keys) = self.arena().get(index) {
            let values = keys
                .into_iter()
                .map(|key| self.indexed(object, key, depth + 1))
                .collect::<Option<Vec<_>>>()?;
            return self.union(values, depth + 1);
        }
        if crate::indexer::resolve::engine::head_decl::head_decl_id(self.arena(), object).is_some()
        {
            return self.nominal_indexed(object, index, depth + 1);
        }
        let object = self.surface(object, depth + 1)?;
        let Type::Operator(TypeOperator::Object(properties)) = self.arena().get(object) else {
            return None;
        };
        if index == self.atom(Intrinsic::Never) {
            return Some(index);
        }
        if let Some(property) = properties.iter().find(|p| !p.index && p.key == index) {
            return if property.optional {
                self.union(
                    vec![property.value, self.atom(Intrinsic::Undefined)],
                    depth + 1,
                )
            } else {
                Some(property.value)
            };
        }
        // An exact index domain wins over a wider one (number before string).
        if let Some(property) = properties.iter().find(|p| p.index && p.key == index) {
            return Some(property.value);
        }
        properties
            .iter()
            .find(|p| p.index && key_in(self.arena(), index, p.key))
            .map(|p| p.value)
    }

    fn remove_undefined(&mut self, ty: TypeId, depth: usize) -> Option<TypeId> {
        let undefined = self.atom(Intrinsic::Undefined);
        let values = match self.arena().get(ty) {
            Type::Union(values) => values,
            _ => vec![ty],
        };
        self.union(
            values.into_iter().filter(|&ty| ty != undefined).collect(),
            depth,
        )
    }
}

fn modifier(modifier: MappedModifier, original: bool) -> bool {
    match modifier {
        MappedModifier::Preserve => original,
        MappedModifier::Add => true,
        MappedModifier::Remove => false,
    }
}

fn singleton(arena: &TypeArena, key: TypeId) -> bool {
    // Numeric property keys need a source-attested numeric/string equivalence
    // bridge. Do not synthesize a string and search it during evaluation.
    matches!(
        arena.get(key),
        Type::Literal(LitValue::Str(_) | LitValue::Utf16(_)) | Type::UniqueSymbol(_)
    )
}

#[cfg(test)]
#[path = "program_structural_ops_tests.rs"]
mod tests;
