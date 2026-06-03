// =============================================================================
// containment.rs — first-class symbol containment (the `ContainingSymbol` chain)
//
// A symbol's place in the program is a kind-tagged chain of frames, innermost
// first: `[self, parent, …, namespace, project]`. It is built once per symbol
// and consumed by resolution and query code instead of re-deriving ancestry by
// splitting dotted `qualified_name` strings.
//
// Selection is always by *kind* (`containing_of_kind` / `enclosing_of_kind`),
// never by position. A parameter root, a field, and a namespace member all
// resolve through the same frame walk — which is what positional indexing into
// a flattened scope chain (`get(1)`, `len - 2`) cannot express, because it
// loses the kind that distinguishes a method frame from its class.
//
// The qualified-name string remains the cross-file/DB key; it is *derived* from
// this chain through one serializer rather than hand-built at each call site.
// =============================================================================

use crate::types::{ExtractedSymbol, SymbolKind};

/// The kind of one containment frame. Extends `SymbolKind` with `Project` —
/// the repository/assembly root, which has no per-symbol analog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameKind {
    /// The project root (Roslyn `IAssemblySymbol`). Outermost frame; never
    /// part of the serialized qualified name.
    Project,
    /// Any indexed symbol — namespace, type, callable, value, …
    Sym(SymbolKind),
}

impl FrameKind {
    /// True for type-defining frames (class/struct/interface/trait/enum). The
    /// `ContainingType` selector.
    pub fn is_type(self) -> bool {
        matches!(
            self,
            FrameKind::Sym(
                SymbolKind::Class
                    | SymbolKind::Struct
                    | SymbolKind::Interface
                    | SymbolKind::Trait
                    | SymbolKind::Enum
            )
        )
    }

    /// True for namespace/module frames. The `ContainingNamespace` selector.
    pub fn is_namespace(self) -> bool {
        matches!(
            self,
            FrameKind::Sym(SymbolKind::Namespace | SymbolKind::Module)
        )
    }

    /// True for callable frames (function/method/constructor).
    pub fn is_callable(self) -> bool {
        matches!(
            self,
            FrameKind::Sym(
                SymbolKind::Function | SymbolKind::Method | SymbolKind::Constructor
            )
        )
    }
}

/// One level of a containment chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeFrame {
    pub kind: FrameKind,
    /// Simple (unqualified) name of this level: `"repo"`, `"run"`, `"App"`.
    pub name: String,
    /// In-file index of the symbol this frame *is*, when it is an indexed
    /// symbol. `None` for synthesized namespace/project frames. Lets a consumer
    /// recover the frame's full symbol record (generics, member index, …).
    pub sym: Option<u32>,
}

impl ScopeFrame {
    pub fn new(kind: FrameKind, name: impl Into<String>, sym: Option<u32>) -> Self {
        Self {
            kind,
            name: name.into(),
            sym,
        }
    }
}

/// A symbol's containment chain, innermost first: `[self, …, namespace, project]`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContainingScope {
    frames: Vec<ScopeFrame>,
}

impl ContainingScope {
    pub fn new(frames: Vec<ScopeFrame>) -> Self {
        Self { frames }
    }

    pub fn frames(&self) -> &[ScopeFrame] {
        &self.frames
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// The symbol's own (innermost) frame. `None` for the empty/global scope.
    pub fn own(&self) -> Option<&ScopeFrame> {
        self.frames.first()
    }

    /// `ContainingSymbol`: the immediately enclosing frame, skipping self.
    pub fn containing(&self) -> Option<&ScopeFrame> {
        self.frames.get(1)
    }

    /// Self + enclosing frames, innermost → outermost.
    pub fn chain(&self) -> impl Iterator<Item = &ScopeFrame> {
        self.frames.iter()
    }

    /// Enclosing frames only (excludes self), innermost → outermost. The
    /// containment-probe order for `{scope}.{name}` member lookups.
    pub fn ancestors(&self) -> impl Iterator<Item = &ScopeFrame> {
        self.frames.iter().skip(1)
    }

    /// Nearest frame matching `pred`, scanning from self outward (self
    /// included). Answers "what <kind> is this symbol lexically inside?" — e.g.
    /// the enclosing class of a ref whose source symbol *is* that class (a
    /// field initializer) is the class itself.
    pub fn enclosing_of_kind(&self, pred: impl Fn(FrameKind) -> bool) -> Option<&ScopeFrame> {
        self.frames.iter().find(|f| pred(f.kind))
    }

    /// Nearest enclosing frame matching `pred`, *excluding* self — Roslyn's
    /// `ContainingType` / `ContainingNamespace`. A class's containing type is
    /// its outer type, not itself.
    pub fn containing_of_kind(&self, pred: impl Fn(FrameKind) -> bool) -> Option<&ScopeFrame> {
        self.frames.iter().skip(1).find(|f| pred(f.kind))
    }

    /// Serialize to a qualified name joined by `sep`, outermost → innermost,
    /// omitting the `Project` root. The dotted default; languages whose qname
    /// scheme differs supply their own serializer over the same frames.
    pub fn to_qualified_name(&self, sep: &str) -> String {
        let parts: Vec<&str> = self
            .frames
            .iter()
            .rev()
            .filter(|f| f.kind != FrameKind::Project)
            .map(|f| f.name.as_str())
            .collect();
        parts.join(sep)
    }
}

/// Interned handle to a `ContainingScope`. Cheap `Copy` — this is what
/// consumers pass around in place of a cloned `Vec<String>` scope chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeId(pub u32);

/// Per-index arena of containment scopes — one per symbol, built once. Holds
/// the frames so consumers carry a `Copy` `ScopeId` and resolve through the
/// arena, rather than reconstructing a scope chain per reference.
#[derive(Debug, Default)]
pub struct ScopeArena {
    scopes: Vec<ContainingScope>,
}

impl ScopeArena {
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern a scope, returning its handle.
    pub fn push(&mut self, scope: ContainingScope) -> ScopeId {
        let id = ScopeId(self.scopes.len() as u32);
        self.scopes.push(scope);
        id
    }

    pub fn get(&self, id: ScopeId) -> &ContainingScope {
        &self.scopes[id.0 as usize]
    }

    pub fn len(&self) -> usize {
        self.scopes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Builder — assemble a symbol's containing scope from the parent_index chain.
// ---------------------------------------------------------------------------

/// Build a symbol's containing scope from its `parent_index` chain.
///
/// The chain is structural: it follows the in-file parent pointers, not the
/// dotted `qualified_name` string, so it is immune to qname-construction bugs —
/// a parameter whose stored qname dropped its package still resolves through
/// its method's chain.
///
/// When the structural chain tops out at a non-namespace symbol that still
/// carries a dotted `scope_path` (the namespace was hoisted into the qname
/// rather than modeled as a parent — Java packages), the namespace tail is
/// synthesized from that path so the chain reaches the namespace root. The tail
/// frames are tagged `Namespace`; a caller with a symbol index can refine their
/// kinds. An optional `project` root caps the chain (omitted from serialized
/// qnames).
pub fn build_containing_scope(
    symbols: &[ExtractedSymbol],
    idx: usize,
    project: Option<&str>,
) -> ContainingScope {
    let mut frames: Vec<ScopeFrame> = Vec::new();

    let mut cur = Some(idx);
    let mut topmost = idx;
    while let Some(i) = cur {
        let sym = &symbols[i];
        frames.push(ScopeFrame {
            kind: FrameKind::Sym(sym.kind),
            name: sym.name.clone(),
            sym: Some(i as u32),
        });
        topmost = i;
        cur = sym.parent_index;
    }

    let top = &symbols[topmost];
    if !matches!(top.kind, SymbolKind::Namespace | SymbolKind::Module) {
        if let Some(sp) = top.scope_path.as_deref().filter(|s| !s.is_empty()) {
            // rsplit yields innermost-first, matching the frame order.
            for seg in sp.rsplit('.') {
                frames.push(ScopeFrame {
                    kind: FrameKind::Sym(SymbolKind::Namespace),
                    name: seg.to_string(),
                    sym: None,
                });
            }
        }
    }

    if let Some(proj) = project {
        frames.push(ScopeFrame {
            kind: FrameKind::Project,
            name: proj.to_string(),
            sym: None,
        });
    }

    ContainingScope::new(frames)
}

/// Build one `ContainingScope` per symbol, returning the arena and the
/// per-symbol handle map (symbol index → `ScopeId`).
pub fn build_scope_arena(
    symbols: &[ExtractedSymbol],
    project: Option<&str>,
) -> (ScopeArena, Vec<ScopeId>) {
    let mut arena = ScopeArena::new();
    let ids: Vec<ScopeId> = (0..symbols.len())
        .map(|i| arena.push(build_containing_scope(symbols, i, project)))
        .collect();
    (arena, ids)
}

#[cfg(test)]
#[path = "containment_tests.rs"]
mod tests;
