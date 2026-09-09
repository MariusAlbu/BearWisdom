//! Exact structural indirection metadata; source tokens are decoded at ingestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Mutability {
    Shared,
    Mutable,
}

/// Regions not yet source-bound remain explicit uncertainty, never a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Lifetime {
    Static,
    Parameter(super::GenericParamId),
    Unknown,
    /// Snapshot-local inference variable at a method-call selector. Identity
    /// does not prove an outlives relationship or a successful borrow check.
    Inference {
        owner: i64,
        byte: u32,
    },
}

#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum GenericParamKind {
    #[default]
    Type,
    Lifetime,
    Const,
}

impl super::TypeArena {
    /// Hot-path metadata read; do not clone the display name for resolution.
    pub fn generic_kind(&self, param: super::GenericParamId) -> GenericParamKind {
        self.inner.read().unwrap().generic_params[param.index()].kind
    }

    /// Preserve argument kind; unsupported const parameters never masquerade as types.
    pub fn generic_type(&self, param: super::GenericParamId) -> super::TypeId {
        self.intern(match self.generic_kind(param) {
            GenericParamKind::Type => super::Type::Generic { param },
            GenericParamKind::Lifetime => super::Type::Region(Lifetime::Parameter(param)),
            GenericParamKind::Const => super::Type::Unknown,
        })
    }

    pub fn intern_type_parameter(
        &self,
        name: String,
        owner_symbol_index: usize,
        bound: Option<super::TypeId>,
    ) -> super::GenericParamId {
        self.intern_generic(super::GenericParamData {
            name,
            owner_symbol_index,
            bound,
            kind: GenericParamKind::Type,
        })
    }

    pub fn region(&self, ty: super::TypeId) -> Lifetime {
        match self.get(ty) {
            super::Type::Region(region) => region,
            _ => Lifetime::Unknown,
        }
    }

    pub fn format_region(&self, region: Lifetime) -> String {
        match region {
            Lifetime::Static => "'static".into(),
            Lifetime::Parameter(param) => self.generic_param(param).name,
            Lifetime::Unknown | Lifetime::Inference { .. } => "'_".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Indirection {
    Reference(Lifetime),
    Pointer,
}

#[cfg(test)]
#[path = "indirection_tests.rs"]
mod tests;
