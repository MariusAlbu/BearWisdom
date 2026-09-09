//! Ingestion-only migration of legacy parameter names into canonical IDs.
use super::{Type, TypeArena, TypeId};
use rustc_hash::FxHashMap;

impl TypeArena {
    /// Rewrite every `Class(name)` whose `name` is a key of `params` to that
    /// param's `Type::Generic` id, recursing through structural types.
    /// `intern_type_str` is param-blind — it interns a generic return like
    /// `Iter<T>` as `Apply{Iter,[Class("T")]}`. Applying this with the owning
    /// type's `{name → Type::Generic id}` map turns the nominal `Class("T")`
    /// into the bindable `Generic(T)` so the chain walker's `substitute` can
    /// resolve it against the receiver's bound args.
    pub fn rebind_class_params(&self, id: TypeId, params: &FxHashMap<String, TypeId>) -> TypeId {
        match self.get(id) {
            Type::Class(name) => params.get(&name).copied().unwrap_or(id),
            // A bound nominal can never be a generic-param name.
            Type::Decl { .. } => id,
            Type::Callable(_) | Type::Object(_) => id, // Source-owned types never rebind by spelling.
            Type::Operator(op) => self.intern(Type::Operator(
                op.map(|ty| self.rebind_class_params(*ty, params)),
            )),
            Type::Apply { base, args } => {
                let base = self.rebind_class_params(base, params);
                let args = args
                    .iter()
                    .map(|&a| self.rebind_class_params(a, params))
                    .collect();
                self.intern(Type::Apply { base, args })
            }
            Type::Optional(inner) => {
                let inner = self.rebind_class_params(inner, params);
                self.intern(Type::Optional(inner))
            }
            Type::AsyncWrapper(inner) => {
                let inner = self.rebind_class_params(inner, params);
                self.intern(Type::AsyncWrapper(inner))
            }
            Type::Iterator(inner) => {
                let inner = self.rebind_class_params(inner, params);
                self.intern(Type::Iterator(inner))
            }
            Type::Constructor(inner) => {
                let inner = self.rebind_class_params(inner, params);
                self.intern(Type::Constructor(inner))
            }
            Type::Tuple(elems) => {
                let elems = elems
                    .iter()
                    .map(|&e| self.rebind_class_params(e, params))
                    .collect();
                self.intern(Type::Tuple(elems))
            }
            Type::Union(branches) => {
                let branches = branches
                    .iter()
                    .map(|&b| self.rebind_class_params(b, params))
                    .collect();
                self.intern(Type::Union(branches))
            }
            Type::Intersection(branches) => {
                let branches = branches
                    .iter()
                    .map(|&b| self.rebind_class_params(b, params))
                    .collect();
                self.intern(Type::Intersection(branches))
            }
            Type::Function {
                params: ps,
                return_,
            } => {
                let ps = ps
                    .iter()
                    .map(|&p| self.rebind_class_params(p, params))
                    .collect();
                let return_ = self.rebind_class_params(return_, params);
                self.intern(Type::Function {
                    params: ps,
                    return_,
                })
            }
            Type::Indirect {
                kind,
                mutability,
                inner,
            } => {
                let inner = self.rebind_class_params(inner, params);
                self.intern(Type::Indirect {
                    kind,
                    mutability,
                    inner,
                })
            }
            Type::Primitive(_)
            | Type::Intrinsic(_)
            | Type::UniqueSymbol(_)
            | Type::Generic { .. }
            | Type::Region(_)
            | Type::Literal(_)
            | Type::Unknown => id,
        }
    }
}

#[cfg(test)]
#[path = "type_rebind_tests.rs"]
mod tests;
