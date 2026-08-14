// =============================================================================
// engine/cause — first-uncaptured-type cause for an unresolved ref
//
// A death site (a chain root that can't be typed, a member that can't be
// found, a rule ladder with no binding) carries a reason more specific than
// "unresolved": the symbol whose OWN type was never captured, which made the
// receiver — or a hop in its chain — untypable. This module names that
// vocabulary; the death sites in `chain.rs` / `semantic_model.rs` populate it
// from state already in hand, never by re-deriving a resolution.
// =============================================================================

/// Why a death site failed to bind, independent of which symbol (if any) it
/// names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CauseKind {
    /// A call root's, or a mid-chain member's, callee resolved but its own
    /// return type was never captured.
    UncapturedReturn,
    /// A value root's, or a mid-chain member's, declaring symbol resolved but
    /// its own field/declared type was never captured.
    UncapturedField,
    /// A parameter or local binding carries no captured declared type and no
    /// initializer this engine can attribute to another symbol.
    UntypedBinding,
    /// The receiver's type declaration is external and carries zero
    /// materialized members — the member lookup miss traces to the externals
    /// pipeline never having exposed this type's surface, not to a genuinely
    /// absent member.
    ExternalUnmaterialized,
    /// The receiver's type declaration is internal (or external with other
    /// materialized members) and genuinely has no member of this name.
    MemberMissing,
    /// The receiver's type expanded through a capture-only alias arm (Union /
    /// Intersection / Keyof / Other) that member lookup can never resolve
    /// into a concrete member set.
    AliasOpaque,
    /// The chain's root segment, or the ladder's target name, names nothing
    /// this file imports, declares, or has in ambient scope — and no probe
    /// below could say anything more specific.
    UnboundRoot,
    /// An import statement binds exactly this name in the file, but no rung
    /// produced a resolution through it — the import's module never linked
    /// to an indexed file or symbol.
    ImportUnlinked,
    /// An enclosing scope of the ref site declares a member of this name —
    /// an implicit-receiver root (bare method/field access inside a type
    /// body) the engine failed to dispatch. Blames the member candidate.
    ScopeMemberRoot,
    /// The name is externally attributable (primitive, framework global, or
    /// manifest-declared dependency surface) yet no external binding
    /// materialized — supply exists, the link failed.
    ExternalKnownUnbound,
    /// The project index holds at least one declaration of this name, but no
    /// rung could reach it from this file — a reachability gap (missing
    /// import semantics, scope rung, or qualification mismatch). Blames the
    /// declaration when it is unique.
    DefinedUnimported,
    /// No declaration of this name exists anywhere the engine can see —
    /// internal or external. Missing supply, or a dynamically-constructed
    /// name.
    NameUnknown,
    /// A multi-segment member walk anchored its root but declined a later
    /// hop without recording a cause of its own.
    ChainDeclined,
}

impl CauseKind {
    /// The stable string persisted to `unresolved_refs.cause_kind`.
    pub const fn as_db_str(self) -> &'static str {
        match self {
            Self::UncapturedReturn => "uncaptured_return",
            Self::UncapturedField => "uncaptured_field",
            Self::UntypedBinding => "untyped_binding",
            Self::ExternalUnmaterialized => "external_unmaterialized",
            Self::MemberMissing => "member_missing",
            Self::AliasOpaque => "alias_opaque",
            Self::UnboundRoot => "unbound_root",
            Self::ImportUnlinked => "unbound_import_unlinked",
            Self::ScopeMemberRoot => "unbound_scope_member",
            Self::ExternalKnownUnbound => "unbound_external_known",
            Self::DefinedUnimported => "unbound_defined_unimported",
            Self::NameUnknown => "unbound_name_unknown",
            Self::ChainDeclined => "chain_declined",
        }
    }
}

/// The first-uncaptured-type cause recorded when a ref fails to resolve.
///
/// `symbol_id` names the symbol whose own type was never captured — `None`
/// only for `UnboundRoot`, where there is no symbol to blame.
#[derive(Debug, Clone, Copy)]
pub struct Cause {
    pub symbol_id: Option<i64>,
    pub kind: CauseKind,
}

impl Cause {
    pub fn new(symbol_id: Option<i64>, kind: CauseKind) -> Self {
        Self { symbol_id, kind }
    }
}
