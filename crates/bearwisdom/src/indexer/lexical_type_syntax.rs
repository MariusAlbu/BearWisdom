//! Syntax-to-identity ingestion. Bound heads never retain a spelling for lookup.
use super::{BindingId, LexicalBindings, LexicalSyntax};
use crate::types::{ExtractedSymbol, SourceSpan};
use std::collections::HashMap;
use tree_sitter::Node;

#[path = "lexical_callable_types.rs"]
pub(crate) mod callables;
#[path = "lexical_signature_types.rs"]
pub(crate) mod signatures;

#[path = "lexical_initializer_types.rs"]
pub(crate) mod initializers;

#[path = "lexical_array_types.rs"]
pub(crate) mod arrays;
#[path = "lexical_atomic_types.rs"]
pub(crate) mod atoms;
#[path = "lexical_compiler_intrinsics.rs"]
pub(crate) mod compiler_intrinsics;
#[path = "lexical_unique_symbols.rs"]
pub(crate) mod unique_symbols;

#[path = "lexical_type_heritage.rs"]
mod heritage;
#[path = "lexical_infer_types.rs"]
pub(crate) mod inference;
#[path = "lexical_structural_types.rs"]
pub(crate) mod structural;

#[derive(Debug, Clone, Copy)]
pub(crate) enum TypeForm {
    Name,
    ValueQuery,
    Transparent,
    Apply,
    Function(&'static callables::Forms),
    Array(&'static arrays::Forms),
    Tuple,
    Union,
    Intersection,
    Optional,
    Atomic(&'static atoms::Forms),
    KeyOf,
    Readonly,
    IndexedAccess,
    Infer,
    Object(&'static structural::Forms),
    AtomicOrUnique(&'static atoms::Forms, &'static unique_symbols::Forms),
    Conditional {
        check: &'static str,
        extends: &'static str,
        when_true: &'static str,
        when_false: &'static str,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum TypeExpr {
    /// Imported/qualified/unsupported syntax remains an explicit legacy boundary.
    Legacy(String),
    /// Source name ID; spelling is compatibility payload for unconfigured files only.
    Global {
        name: super::NameId,
        legacy: String,
    },
    Primitive(crate::type_checker::core::types::PrimKind),
    Intrinsic(crate::type_checker::core::types::Intrinsic),
    Literal(crate::type_checker::core::types::LitValue),
    UniqueSymbol(SourceSpan),
    /// Display payload is retained only for the existing unconfigured boundary.
    ValueQuery {
        site: SourceSpan,
        legacy: String,
    },
    Operator(Box<crate::type_checker::core::types::TypeOperator<Self>>),
    Declaration(BindingId),
    /// Namespace recipe IDs are a separate arena from local declaration IDs.
    Source {
        usage: crate::indexer::namespaces::Use,
        legacy: Option<String>,
    },
    Parameter {
        owner: Option<usize>,
        index: usize,
    },
    /// Generic owner is a source signature even when extraction emits no row.
    SignatureParameter {
        owner: signatures::SignatureId,
        index: usize,
    },
    SourceParameter {
        owner: crate::indexer::namespaces::Use,
        index: usize,
    },
    Apply(Box<Self>, Vec<Self>),
    InputRegion {
        owner: usize,
        byte: u32,
    },
    InputApplication {
        owner: usize,
        byte: u32,
        base: Box<Self>,
        args: Vec<Self>,
    },
    Output {
        inputs: Vec<Self>,
        result: Box<Self>,
    },
    OutputRegion,
    OutputApplication {
        base: Box<Self>,
        args: Vec<Self>,
    },
    Region(crate::type_checker::core::types::Lifetime),
    Indirect {
        kind: crate::type_checker::core::types::Indirection,
        mutability: crate::type_checker::core::types::Mutability,
        region: Option<Box<Self>>,
        inner: Box<Self>,
    },
    Function(Vec<Self>, Box<Self>),
    Callable(
        Box<crate::type_checker::core::types::Callable<Self, SourceSpan>>,
        Box<Self>,
    ),
    Tuple(Vec<Self>),
    Union(Vec<Self>),
    Intersection(Vec<Self>),
    Optional(Box<Self>),
    Unknown,
}

impl TypeExpr {
    pub(crate) fn is_bound(&self) -> bool {
        match self {
            Self::Primitive(_)
            | Self::Intrinsic(_)
            | Self::Literal(_)
            | Self::Declaration(_)
            | Self::Source { .. }
            | Self::Parameter { .. }
            | Self::SourceParameter { .. }
            | Self::SignatureParameter { .. } => true,
            Self::Apply(base, args) => base.is_bound() || args.iter().any(Self::is_bound),
            Self::Function(args, ret) => ret.is_bound() || args.iter().any(Self::is_bound),
            Self::Callable(..) => true,
            Self::Indirect { .. }
            | Self::Region(_)
            | Self::InputRegion { .. }
            | Self::InputApplication { .. } => true,
            Self::Output { .. } | Self::OutputRegion | Self::OutputApplication { .. } => true,
            Self::Optional(inner) => inner.is_bound(),
            Self::Operator(_) | Self::UniqueSymbol(_) => true,
            Self::Tuple(items) | Self::Union(items) | Self::Intersection(items) => {
                items.iter().any(Self::is_bound)
            }
            Self::Global { .. } | Self::ValueQuery { .. } | Self::Legacy(_) | Self::Unknown => {
                false
            }
        }
    }
}

/// Value recipes never contain display names. Source owner slots are lowered
/// to declaration IDs before runtime evaluation reads position-correct facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ValueExpr<Owner = usize, Name = super::NameId> {
    Read {
        binding: BindingId,
        byte: u32,
    },
    Borrow {
        owner: Owner,
        span: SourceSpan,
        mutability: crate::type_checker::core::types::Mutability,
        operand: Box<Self>,
    },
    VariantField {
        variant: Name,
        field: Name,
        byte: u32,
        operand: Box<Self>,
    },
    Field {
        name: Name,
        byte: u32,
        operand: Box<Self>,
    },
    TupleIndex {
        index: usize,
        operand: Box<Self>,
    },
    Dereference {
        operand: Box<Self>,
    },
    Unknown,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TypeUses {
    pub compiler_intrinsics: HashMap<usize, compiler_intrinsics::Role>,
    pub unique_symbols: Vec<SourceSpan>,
    pub member_values: Vec<signatures::MemberValue>,
    pub computed_keys: Vec<(SourceSpan, super::globals::member_surface::Key<BindingId>)>,
    pub value_queries: Vec<(SourceSpan, super::globals::member_surface::Key<BindingId>)>,
    pub intrinsic_members: HashMap<crate::type_checker::core::types::Intrinsic, super::NameId>,
    pub array_types: HashMap<arrays::Kind, super::NameId>,
    pub signatures: Vec<signatures::Signature>,
    pub initializers: Vec<initializers::Input>,
    pub objects: Vec<initializers::objects::Input>,
    /// Physical class slot -> source value binding and generic type arguments.
    pub bases: HashMap<usize, Option<(BindingId, Vec<TypeExpr>)>>,
    /// Complete source interface headers, separate from runtime class bases.
    pub interface_bases: HashMap<usize, Option<Vec<TypeExpr>>>,
    pub generic_declarations: HashMap<
        usize,
        Vec<(
            super::NameId,
            crate::type_checker::core::types::GenericParamKind,
        )>,
    >,
    pub annotations: HashMap<BindingId, TypeExpr>,
    pub arguments: HashMap<u32, Vec<TypeExpr>>,
    pub member_arguments: HashMap<u32, Vec<TypeExpr>>,
    pub parameters: HashMap<usize, Vec<TypeExpr>>,
    pub receivers: HashMap<usize, TypeExpr>,
    /// Receiver value binding -> physical method slot; reuse its bound signature.
    pub receiver_values: HashMap<BindingId, usize>,
    pub returns: HashMap<usize, TypeExpr>,
    pub fields: HashMap<usize, TypeExpr>,
    pub aliases: HashMap<usize, TypeExpr>,
    pub constructed: HashMap<BindingId, TypeExpr>,
    /// Source-owned initializer expressions; reads are snapshots, not live aliases.
    pub values: HashMap<BindingId, ValueExpr>,
    pub expressions: HashMap<SourceSpan, ValueExpr>,
    pub reference_fields: bool,
    pub pattern_heads: HashMap<u32, TypeExpr>,
    /// Declaration slot -> (initializer expression byte, called selector byte).
    /// Only a direct call/new expression owns the initializer's whole value.
    pub call_initializers: HashMap<usize, (u32, u32)>,
}

pub(super) fn capture(
    root: Node,
    source: &[u8],
    syntax: &LexicalSyntax,
    symbols: &[ExtractedSymbol],
    graph: &mut LexicalBindings,
) {
    let mut anchors = HashMap::new();
    for (slot, symbol) in symbols.iter().enumerate() {
        anchors
            .entry((symbol.start_line, symbol.start_col))
            .and_modify(|old| *old = None)
            .or_insert(Some(slot));
    }
    intern_type_names(root, source, syntax, graph);
    anchors.extend(
        graph
            .declaration_slots
            .iter()
            .map(|(&point, &slot)| (point, slot)),
    );
    let mut uses = TypeUses {
        computed_keys: std::mem::take(&mut graph.types.computed_keys),
        value_queries: std::mem::take(&mut graph.types.value_queries),
        ..Default::default()
    };
    for (_, form) in syntax.type_forms {
        if let TypeForm::Array(forms) = form {
            arrays::register(forms, graph, &mut uses);
        }
        if let TypeForm::Atomic(forms) | TypeForm::AtomicOrUnique(forms, _) = form {
            for &(kind, wrapper) in forms.member_wrappers {
                uses.intrinsic_members.insert(kind, graph.intern(wrapper));
            }
        }
    }
    let capture = Capture {
        source,
        syntax,
        graph,
        anchors,
    };
    capture.walk(root, &mut uses);
    graph.types = uses;
}

struct Capture<'a> {
    source: &'a [u8],
    syntax: &'a LexicalSyntax,
    graph: &'a LexicalBindings,
    anchors: HashMap<(u32, u32), Option<usize>>,
}

impl Capture<'_> {
    fn slot(&self, node: Node) -> Option<usize> {
        let p = node.start_position();
        self.anchors
            .get(&(p.row as u32, p.column as u32))
            .copied()
            .flatten()
    }

    fn walk(&self, node: Node, uses: &mut TypeUses) {
        if let Some(input) = initializers::capture(self, node) {
            uses.initializers.push(input);
        }
        if let Some(object) = initializers::capture_object(self, node) {
            uses.objects.push(object);
        }
        heritage::capture(self, node, uses);
        if let Some((_, TypeForm::AtomicOrUnique(_, forms))) = self
            .syntax
            .type_forms
            .iter()
            .find(|(kind, _)| *kind == node.kind())
        {
            if let Some(Some(owner)) = unique_symbols::owner(node, forms) {
                let site = unique_symbols::span(owner);
                uses.unique_symbols.push(site);
            }
        }
        if let Some(signature) = signatures::capture(self, node) {
            uses.signatures.push(signature);
        }
        if let Some(member) = signatures::member_value(self, node) {
            uses.member_values.push(member);
        }
        if let (Some(slot), Some(parameters)) =
            (self.slot(node), node.child_by_field_name("type_parameters"))
        {
            let mut cursor = parameters.walk();
            let names = parameters
                .named_children(&mut cursor)
                .filter(|n| !n.is_extra())
                .filter_map(|param| {
                    self.graph
                        .name_id(
                            param
                                .child_by_field_name("name")?
                                .utf8_text(self.source)
                                .ok()?,
                        )
                        .map(|name| {
                            (
                                name,
                                crate::type_checker::core::types::GenericParamKind::Type,
                            )
                        })
                })
                .collect();
            uses.generic_declarations.insert(slot, names);
        }
        if self.syntax.base_types.0.contains(&node.kind()) {
            if let Some(slot) = self.slot(node) {
                uses.bases.insert(slot, self.base_type(node));
            }
        }
        if let (Some(slot), Some(value)) = (self.slot(node), node.child_by_field_name("value")) {
            if let Some(&(_, field)) = self
                .syntax
                .call_roots
                .iter()
                .find(|&&(kind, _)| kind == value.kind())
            {
                if let Some(callee) = value.child_by_field_name(field) {
                    let selector = callee.child_by_field_name("property").unwrap_or(callee);
                    uses.call_initializers.insert(
                        slot,
                        (value.start_byte() as u32, selector.start_byte() as u32),
                    );
                }
            }
        }
        if let (Some(slot), Some(parameters)) =
            (self.slot(node), node.child_by_field_name("parameters"))
        {
            uses.parameters.insert(slot, self.parameters(parameters));
        }
        if let Some(&(_, field)) = self
            .syntax
            .call_roots
            .iter()
            .find(|&&(kind, _)| kind == node.kind())
        {
            if let (Some(callee), Some(args)) = (
                node.child_by_field_name(field),
                node.child_by_field_name("type_arguments"),
            ) {
                if let Some(selector) = callee.child_by_field_name("property") {
                    uses.member_arguments
                        .insert(selector.start_byte() as u32, self.children(args));
                }
            }
        }
        if let Some(&(_, field)) = self
            .syntax
            .alias_values
            .iter()
            .find(|&&(kind, _)| kind == node.kind())
        {
            if let Some(value) = node.child_by_field_name(field) {
                if let Some(slot) = self.slot(node) {
                    compiler_intrinsics::alias(self, node, value, slot, uses);
                }
            }
        }
        if let Some(ty) = node.child_by_field_name("type") {
            let recipe = self.expr(ty);
            if let Some(name) = node
                .child_by_field_name("pattern")
                .or_else(|| node.child_by_field_name("name"))
            {
                let span = SourceSpan {
                    start: name.start_byte() as u32,
                    end: name.end_byte() as u32,
                };
                if let Some(&binding) = self.graph.declarations.get(&span) {
                    uses.annotations.insert(binding, recipe.clone());
                    if let Some(slot) = self.graph.symbol_slots.get(&binding).copied().flatten() {
                        uses.fields.insert(slot, recipe.clone());
                    }
                }
            }
            if let Some(slot) = self.slot(node) {
                uses.fields.insert(slot, recipe);
            }
        }
        if let Some(ty) = node.child_by_field_name("return_type") {
            if let Some(slot) = self.slot(node) {
                uses.returns.insert(slot, self.expr(ty));
            }
        }
        if self
            .graph
            .call_type_args
            .contains_key(&(node.start_byte() as u32))
        {
            if let Some(args) = node.child_by_field_name("type_arguments") {
                uses.arguments
                    .insert(node.start_byte() as u32, self.children(args));
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, uses);
        }
    }

    fn base_type(&self, node: Node) -> Option<(BindingId, Vec<TypeExpr>)> {
        let (_, container, clause, field) = self.syntax.base_types;
        let (identifier, selector_field, arguments_field) = self.syntax.base_head;
        let mut cursor = node.walk();
        let heritage = node
            .named_children(&mut cursor)
            .find(|n| n.kind() == container)?;
        let mut cursor = heritage.walk();
        let explicit: Vec<_> = heritage
            .named_children(&mut cursor)
            .filter(|n| n.kind() == clause)
            .collect();
        let (head, args) = match explicit.as_slice() {
            [clause] => (
                clause.child_by_field_name(field)?,
                clause.child_by_field_name(arguments_field),
            ),
            [] => (heritage.named_child(0)?, None),
            _ => return None,
        };
        let binding = if let Some(selector) = head.child_by_field_name(selector_field) {
            self.graph
                .module
                .members
                .get(&(selector.start_byte() as u32))
                .copied()
        } else if head.kind() == identifier {
            self.graph
                .name_id(head.utf8_text(self.source).ok()?)
                .and_then(|name| {
                    self.graph
                        .reference_binding_at(head.start_byte() as u32, name)
                })
        } else {
            None
        }?;
        Some((
            binding,
            args.map(|args| self.children(args)).unwrap_or_default(),
        ))
    }

    fn children(&self, node: Node) -> Vec<TypeExpr> {
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .filter(|n| !n.is_extra())
            .map(|n| self.expr(n))
            .collect()
    }

    fn parameters(&self, node: Node) -> Vec<TypeExpr> {
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .filter(|n| !n.is_extra())
            .map(|p| {
                p.child_by_field_name("type")
                    .map(|ty| self.expr(ty))
                    .unwrap_or(TypeExpr::Unknown)
            })
            .collect()
    }

    fn expr(&self, node: Node) -> TypeExpr {
        let span = SourceSpan {
            start: node.start_byte() as u32,
            end: node.end_byte() as u32,
        };
        if let Some(&binding) = self.graph.module.qualified_types.get(&span) {
            return TypeExpr::Declaration(binding);
        }
        let form = self
            .syntax
            .type_forms
            .iter()
            .find(|&&(kind, _)| kind == node.kind())
            .map(|&(_, form)| form);
        match form {
            Some(TypeForm::ValueQuery) => TypeExpr::ValueQuery {
                site: unique_symbols::span(node),
                legacy: node.utf8_text(self.source).unwrap_or_default().into(),
            },
            Some(TypeForm::Object(forms)) => structural::capture(self, node, forms),
            Some(TypeForm::Infer) => inference::capture(self, node),
            Some(TypeForm::Name) => {
                let name = node.utf8_text(self.source).unwrap_or_default();
                if let Some(binding) = self
                    .graph
                    .name_id(name)
                    .and_then(|name| self.graph.type_binding_at(node.start_byte() as u32, name))
                {
                    if let Some(&(row, col, index)) = self.graph.type_parameters.get(&binding) {
                        match (
                            self.anchors.get(&(row, col)).copied().flatten(),
                            self.graph.type_parameter_sites.get(&binding),
                        ) {
                            (None, Some(&owner)) => TypeExpr::SignatureParameter { owner, index },
                            (owner, _) => TypeExpr::Parameter { owner, index },
                        }
                    } else {
                        TypeExpr::Declaration(binding)
                    }
                } else {
                    TypeExpr::Global {
                        name: self.graph.name_id(name).expect("type name ingestion"),
                        legacy: name.to_owned(),
                    }
                }
            }
            Some(TypeForm::Transparent) => {
                let mut cursor = node.walk();
                let child = node.named_children(&mut cursor).find(|n| !n.is_extra());
                child.map(|n| self.expr(n)).unwrap_or(TypeExpr::Unknown)
            }
            Some(TypeForm::Apply) => match (
                node.child_by_field_name("name"),
                node.child_by_field_name("type_arguments"),
            ) {
                (Some(base), Some(args)) => {
                    TypeExpr::Apply(Box::new(self.expr(base)), self.children(args))
                }
                _ => TypeExpr::Unknown,
            },
            Some(TypeForm::Function(forms)) => callables::capture(self, node, forms),
            Some(TypeForm::Array(forms)) => TypeExpr::Apply(
                Box::new(TypeExpr::Global {
                    name: self
                        .graph
                        .name_id(forms.mutable)
                        .expect("profile constructor ingestion"),
                    legacy: forms.mutable.to_owned(),
                }),
                vec![node
                    .named_child(0)
                    .map(|n| self.expr(n))
                    .unwrap_or(TypeExpr::Unknown)],
            ),
            Some(TypeForm::Optional) => TypeExpr::Optional(Box::new(
                node.named_child(0)
                    .map(|n| self.expr(n))
                    .unwrap_or(TypeExpr::Unknown),
            )),
            Some(TypeForm::Tuple) => TypeExpr::Tuple(self.children(node)),
            Some(TypeForm::Union) => TypeExpr::Union(self.children(node)),
            Some(TypeForm::Intersection) => TypeExpr::Intersection(self.children(node)),
            Some(TypeForm::Atomic(forms)) => atoms::capture(node, self.source, forms),
            Some(TypeForm::AtomicOrUnique(atoms, forms)) => {
                match unique_symbols::owner(node, forms) {
                    Some(owner) => owner
                        .map(|n| TypeExpr::UniqueSymbol(unique_symbols::span(n)))
                        .unwrap_or(TypeExpr::Unknown),
                    None => atoms::capture(node, self.source, atoms),
                }
            }
            Some(TypeForm::Readonly) => arrays::readonly(self, node),
            Some(TypeForm::KeyOf | TypeForm::IndexedAccess | TypeForm::Conditional { .. }) => {
                self.operator(node, form.unwrap())
            }
            None => TypeExpr::Legacy(node.utf8_text(self.source).unwrap_or_default().to_owned()),
        }
    }

    fn operator(&self, node: Node, form: TypeForm) -> TypeExpr {
        use crate::type_checker::core::types::TypeOperator;
        let op = match form {
            TypeForm::KeyOf => {
                let mut children = self.children(node).into_iter();
                let Some(inner) = children.next() else {
                    return TypeExpr::Unknown;
                };
                if children.next().is_some() {
                    return TypeExpr::Unknown;
                }
                TypeOperator::KeyOf(inner)
            }
            TypeForm::IndexedAccess => {
                let mut children = self.children(node).into_iter();
                let (Some(object), Some(index)) = (children.next(), children.next()) else {
                    return TypeExpr::Unknown;
                };
                if children.next().is_some() {
                    return TypeExpr::Unknown;
                }
                TypeOperator::IndexedAccess { object, index }
            }
            TypeForm::Conditional {
                check,
                extends,
                when_true,
                when_false,
            } => {
                let fields = [check, extends, when_true, when_false]
                    .map(|field| node.child_by_field_name(field).map(|n| self.expr(n)));
                let [Some(check), Some(extends), Some(when_true), Some(when_false)] = fields else {
                    return TypeExpr::Unknown;
                };
                let distributive = match check {
                    TypeExpr::Parameter { .. }
                    | TypeExpr::SignatureParameter { .. }
                    | TypeExpr::SourceParameter { .. } => Some(true),
                    TypeExpr::Tuple(_)
                    | TypeExpr::Function(..)
                    | TypeExpr::Callable(..)
                    | TypeExpr::Primitive(_)
                    | TypeExpr::Intrinsic(_)
                    | TypeExpr::Literal(_) => Some(false),
                    // Aliases and simplifiable unions/intersections require binding/evaluation first.
                    _ => None,
                };
                TypeOperator::Conditional {
                    check,
                    extends,
                    when_true,
                    when_false,
                    distributive,
                }
            }
            _ => return TypeExpr::Unknown,
        };
        TypeExpr::Operator(Box::new(op))
    }
}

fn intern_type_names(
    node: Node,
    source: &[u8],
    syntax: &LexicalSyntax,
    graph: &mut LexicalBindings,
) {
    initializers::intern_paths(node, source, syntax, graph);
    if syntax
        .globals
        .surface
        .kinds
        .iter()
        .any(|(kind, _)| *kind == node.kind())
    {
        if let Some(name) = node
            .child_by_field_name("name")
            .filter(|n| syntax.globals.member_names.contains(&n.kind()))
        {
            if let Ok(name) = name.utf8_text(source) {
                graph.intern(name);
            }
        }
    }
    if node.kind() == syntax.globals.surface.computed
        && node.parent().is_some_and(|owner| {
            syntax
                .globals
                .surface
                .kinds
                .iter()
                .any(|(kind, _)| *kind == owner.kind())
                && owner.child_by_field_name("name") == Some(node)
        })
    {
        let key = super::globals::member_surface::key(node, source, syntax, graph);
        graph
            .types
            .computed_keys
            .push((unique_symbols::span(node), key));
    }
    if let Some((_, form)) = syntax
        .type_forms
        .iter()
        .find(|(kind, _)| *kind == node.kind())
    {
        match form {
            TypeForm::ValueQuery => {
                use super::globals::member_surface::{self, Key, Root};
                let mut cursor = node.walk();
                let child = node.named_children(&mut cursor).find(|n| !n.is_extra());
                let key = child
                    .map(|n| {
                        let (root, selectors) = member_surface::path(n, source, syntax, graph, 0)
                            .unwrap_or((Root::Unknown, vec![]));
                        Key::Computed {
                            expression: unique_symbols::span(n),
                            root,
                            selectors,
                            usage: member_surface::ValueUse::Query,
                        }
                    })
                    .unwrap_or(Key::Unknown);
                graph
                    .types
                    .value_queries
                    .push((unique_symbols::span(node), key));
            }
            TypeForm::Name => {
                if let Ok(name) = node.utf8_text(source) {
                    graph.intern(name);
                }
            }
            TypeForm::Function(forms) => callables::intern(node, source, graph, forms),
            TypeForm::Array(forms) => {
                graph.intern(forms.mutable);
                graph.intern(forms.readonly);
            }
            _ => {}
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        intern_type_names(child, source, syntax, graph);
    }
}

#[cfg(test)]
#[path = "lexical_type_syntax_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "lexical_structural_types_tests.rs"]
mod structural_tests;
