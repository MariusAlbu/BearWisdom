//! Source type structure and namespace IDs, shared by signatures and locals.
use super::*;
use crate::indexer::lexical::type_syntax::{TypeExpr, TypeUses};
use crate::type_checker::core::types::{GenericParamKind, Lifetime};
use crate::types::SourceSpan;
use tree_sitter::Node;

pub(crate) struct Forms {
    pub primitive_nodes: &'static [&'static str],
    pub primitives: &'static [(&'static str, crate::type_checker::core::types::PrimKind)],
    pub applications: &'static [(&'static str, &'static str, &'static str)],
    pub call_applications: &'static [(&'static str, &'static str, &'static str)],
    pub indirections: &'static [(&'static str, &'static str, bool)],
    pub mutable_node: &'static str,
    pub lifetime_node: &'static str,
    pub static_lifetime: &'static str,
    pub placeholder_lifetime: &'static str,
    pub elide_input_lifetimes: bool,
    pub elide_output_lifetimes: bool,
    pub receiver_reference: &'static str,
    pub tuples: &'static [&'static str],
    pub functions: &'static [&'static str],
    pub annotation: &'static str,
    pub fields: &'static [&'static str],
    pub patterns: &'static [&'static str],
    pub aliases: &'static [(&'static str, &'static str)],
    pub parameters: &'static str,
    pub ignored_parameters: &'static [&'static str],
    pub return_type: &'static str,
    pub generic_parameters: &'static str,
    pub generic_parameter_forms: &'static [(&'static str, GenericParamKind)],
    pub ignored_arguments: &'static [&'static str],
    pub constructions: &'static [(&'static str, &'static str)],
    pub tuple_values: &'static [&'static str],
}

pub(super) fn capture(
    data: &mut NamespaceData,
    root: Node,
    source: &[u8],
    forms: &super::Forms,
    symbols: &[ExtractedSymbol],
) {
    let mut anchors = HashMap::new();
    for (slot, symbol) in symbols.iter().enumerate() {
        anchors
            .entry((symbol.start_line, symbol.start_col))
            .and_modify(|s| *s = None)
            .or_insert(Some(slot));
    }
    let mut capture = Capture {
        data,
        source,
        forms,
        anchors,
        parameters: HashMap::new(),
    };
    capture.generics(root);
    let mut uses = std::mem::take(&mut capture.data.graph.types);
    uses.reference_fields = forms.places.reference_fields;
    capture.walk(root, &mut uses);
    capture.data.graph.types = uses;
}

struct Capture<'a> {
    data: &'a mut NamespaceData,
    source: &'a [u8],
    forms: &'a super::Forms,
    anchors: HashMap<(u32, u32), Option<usize>>,
    parameters: HashMap<BindingId, (Option<usize>, usize)>,
}

impl Capture<'_> {
    fn slot(&self, node: Node) -> Option<usize> {
        let p = node.start_position();
        self.anchors
            .get(&(p.row as u32, p.column as u32))
            .copied()
            .flatten()
    }

    fn generics(&mut self, node: Node) {
        if let Some(params) = node.child_by_field_name(self.forms.types.generic_parameters) {
            let mut cursor = params.walk();
            for (index, param) in params
                .named_children(&mut cursor)
                .filter_map(|p| {
                    self.forms
                        .types
                        .generic_parameter_forms
                        .iter()
                        .find(|&&(kind, _)| kind == p.kind())
                        .map(|&(_, kind)| (p, kind))
                })
                .enumerate()
            {
                let (param, kind) = param;
                let Some(name) = param
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(self.source).ok())
                else {
                    continue;
                };
                let name = self.data.intern(name, self.forms);
                if let Some(slot) = self.slot(node) {
                    self.data
                        .graph
                        .types
                        .generic_declarations
                        .entry(slot)
                        .or_default()
                        .push((name, kind));
                }
                let Some(scope) = self.data.graph.scope_at(param.start_byte() as u32) else {
                    continue;
                };
                if let Some(&binding) =
                    self.data
                        .entries
                        .get(&(scope, name, parameter_domain(kind)))
                {
                    self.parameters.insert(binding, (self.slot(node), index));
                    self.trait_parameter(node, binding, index, kind);
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.generics(child);
        }
    }

    fn walk(&mut self, node: Node, uses: &mut TypeUses) {
        self.trait_recipes(node);
        let forms = self.forms.types;
        self.initializer(node, uses);
        self.pattern_values(node, uses);
        if self.forms.patterns.positional_field(node) {
            if let Some(slot) = self.slot(node) {
                uses.fields.insert(slot, self.exact_expr(node));
            }
        }
        self.place_expression(node, uses);
        if let Some(&(_, head, arguments)) = forms
            .call_applications
            .iter()
            .find(|&&(kind, _, _)| kind == node.kind())
        {
            if let (Some(head), Some(arguments)) = (
                node.child_by_field_name(head),
                node.child_by_field_name(arguments),
            ) {
                let selector = self
                    .forms
                    .locals
                    .selectors
                    .iter()
                    .find(|&&(kind, _)| kind == head.kind())
                    .and_then(|&(_, field)| head.child_by_field_name(field))
                    .unwrap_or(head);
                let mut cursor = arguments.walk();
                let qualified = self
                    .data
                    .traits
                    .qualified_calls
                    .contains_key(&(selector.start_byte() as u32));
                let recipes = arguments
                    .named_children(&mut cursor)
                    .filter(|n| !n.is_extra())
                    .map(|n| {
                        let recipe = self.exact_expr(n);
                        if qualified {
                            traits::strict(recipe)
                        } else {
                            recipe
                        }
                    })
                    .collect();
                uses.member_arguments
                    .insert(selector.start_byte() as u32, recipes);
            }
        }
        if let Some(ty) = node.child_by_field_name(forms.annotation) {
            // Only whole, direct patterns inherit a whole annotation. Pattern
            // projections require their own recipes; never type every leaf as RHS.
            if let Some(pattern) = forms
                .patterns
                .iter()
                .find_map(|field| node.child_by_field_name(field))
            {
                if let Some(binding) = self.direct_binding(pattern) {
                    let recipe = self.expr(ty);
                    uses.annotations.insert(binding, recipe.clone());
                    if let Some(slot) = self
                        .data
                        .graph
                        .symbol_slots
                        .get(&binding)
                        .copied()
                        .flatten()
                    {
                        uses.fields.insert(slot, recipe);
                    }
                }
            }
            if let Some(slot) = self
                .slot(node)
                .filter(|_| forms.fields.contains(&node.kind()))
            {
                uses.fields.insert(slot, self.expr(ty));
            }
        }
        if let Some(slot) = self.slot(node) {
            self.receiver_value(node, slot, uses);
            let receiver = node
                .child_by_field_name(forms.parameters)
                .and_then(|p| self.receiver(p));
            if let Some((recipe, _)) = &receiver {
                uses.receivers.insert(slot, recipe.clone());
            }
            if let Some(ty) = node.child_by_field_name(forms.return_type) {
                let result = self.expr(ty);
                let result = if forms.elide_output_lifetimes && self.signature_item(node) {
                    let inputs = receiver
                        .as_ref()
                        .filter(|(r, _)| matches!(r, TypeExpr::Indirect { .. } | TypeExpr::Unknown))
                        .map(|(_, region)| vec![region.clone()])
                        .or_else(|| {
                            node.child_by_field_name(forms.parameters)
                                .map(|p| self.params(p))
                        })
                        .unwrap_or_else(|| vec![TypeExpr::Unknown]);
                    TypeExpr::Output {
                        inputs,
                        result: Box::new(result),
                    }
                } else {
                    result
                };
                uses.returns.insert(slot, result);
            }
            if let Some(params) = node.child_by_field_name(forms.parameters) {
                uses.parameters.insert(slot, self.params(params));
            }
            if let Some(&(_, field)) = forms.aliases.iter().find(|&&(kind, _)| kind == node.kind())
            {
                if let Some(ty) = node.child_by_field_name(field) {
                    uses.aliases.insert(slot, self.exact_expr(ty));
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, uses);
        }
    }

    fn direct_binding(&self, mut node: Node) -> Option<BindingId> {
        loop {
            let span = SourceSpan {
                start: node.start_byte() as u32,
                end: node.end_byte() as u32,
            };
            if let Some(&binding) = self.data.graph.declarations.get(&span) {
                return Some(binding);
            }
            if !self.forms.locals.direct_patterns.contains(&node.kind())
                || node.named_child_count() != 1
            {
                return None;
            }
            node = node.named_child(0)?;
        }
    }

    fn children(&mut self, node: Node) -> Vec<TypeExpr> {
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .filter(|n| !n.is_extra() && !self.forms.types.ignored_arguments.contains(&n.kind()))
            .map(|n| self.expr(n))
            .collect()
    }

    fn params(&mut self, node: Node) -> Vec<TypeExpr> {
        let mut cursor = node.walk();
        let params: Vec<_> = node
            .named_children(&mut cursor)
            .filter(|n| !n.is_extra() && !self.is_receiver(*n))
            .collect();
        params
            .into_iter()
            .map(
                |p| match p.child_by_field_name(self.forms.types.annotation) {
                    Some(ty) => self.expr(ty),
                    None if self
                        .forms
                        .locals
                        .parameters
                        .iter()
                        .any(|&(kind, _)| kind == p.kind()) =>
                    {
                        TypeExpr::Unknown
                    }
                    None => self.expr(p),
                },
            )
            .collect()
    }

    fn expr(&mut self, node: Node) -> TypeExpr {
        let forms = self.forms.types;
        if node.kind() == forms.lifetime_node {
            return self.lifetime_expr(node);
        }
        if forms.primitive_nodes.contains(&node.kind()) {
            let text = node.utf8_text(self.source).unwrap_or_default();
            let name = self.data.intern(text, self.forms);
            if let Some(target) = self
                .data
                .graph
                .scope_at(node.start_byte() as u32)
                .and_then(|scope| self.data.lookup(scope, name, ExportDomain::Type))
            {
                let recipe = self.bound_target(target, text.to_owned());
                return self.input_application(node, recipe);
            }
            return forms
                .primitives
                .iter()
                .find(|&&(name, _)| name == text)
                .map(|&(_, kind)| TypeExpr::Primitive(kind))
                .unwrap_or(TypeExpr::Unknown);
        }
        if forms
            .indirections
            .iter()
            .any(|&(kind, _, _)| kind == node.kind())
        {
            return self.exact_expr(node);
        }
        if let Some(&(_, head, args)) = forms
            .applications
            .iter()
            .find(|&&(kind, _, _)| kind == node.kind())
        {
            return match (
                node.child_by_field_name(head),
                node.child_by_field_name(args),
            ) {
                (Some(head), Some(args)) => {
                    let mut cursor = args.walk();
                    let head = match self.expr(head) {
                        TypeExpr::InputApplication { base, args, .. }
                        | TypeExpr::OutputApplication { base, args }
                            if args.is_empty() =>
                        {
                            *base
                        }
                        other => other,
                    };
                    let args = args
                        .named_children(&mut cursor)
                        .filter(|n| !n.is_extra())
                        .map(|n| self.exact_expr(n))
                        .collect();
                    match self.input_owner(node) {
                        Some(owner) => TypeExpr::InputApplication {
                            owner,
                            byte: node.start_byte() as u32,
                            base: Box::new(head),
                            args,
                        },
                        None if self.output_site(node) => TypeExpr::OutputApplication {
                            base: Box::new(head),
                            args,
                        },
                        None => TypeExpr::Apply(Box::new(head), args),
                    }
                }
                _ => TypeExpr::Unknown,
            };
        }
        if forms.tuples.contains(&node.kind()) {
            return TypeExpr::Tuple(self.children(node));
        }
        if forms.functions.contains(&node.kind()) {
            let params = node
                .child_by_field_name(forms.parameters)
                .map(|p| self.params(p))
                .unwrap_or_default();
            let ret = node
                .child_by_field_name(forms.return_type)
                .map(|ty| self.expr(ty))
                .unwrap_or(TypeExpr::Unknown);
            return TypeExpr::Function(params, Box::new(ret));
        }
        let legacy = node.utf8_text(self.source).unwrap_or_default().to_owned();
        let Some(path) = paths::tokens(node, self.source, self.forms, self.data) else {
            return TypeExpr::Legacy(legacy);
        };
        let Some(scope) = self.data.graph.scope_at(node.start_byte() as u32) else {
            return TypeExpr::Unknown;
        };
        let target = paths::target(self.data, self.forms, scope, &path, ExportDomain::Type);
        let recipe = self.bound_target(target, legacy);
        self.input_application(node, recipe)
    }

    fn input_application(&self, node: Node, recipe: TypeExpr) -> TypeExpr {
        match self
            .input_owner(node)
            .filter(|_| matches!(recipe, TypeExpr::Source { .. }))
        {
            Some(owner) => TypeExpr::InputApplication {
                owner,
                byte: node.start_byte() as u32,
                base: Box::new(recipe),
                args: vec![],
            },
            None if matches!(recipe, TypeExpr::Source { .. }) && self.output_site(node) => {
                TypeExpr::OutputApplication {
                    base: Box::new(recipe),
                    args: vec![],
                }
            }
            None => recipe,
        }
    }

    fn bound_target(&mut self, target: Target, legacy: String) -> TypeExpr {
        if let Target::Binding(binding) = target {
            if self.data.traits.self_bindings.contains(&binding) {
                return TypeExpr::Source {
                    usage: Use {
                        binding,
                        domain: ExportDomain::Type,
                        local: true,
                    },
                    legacy: None,
                };
            }
            if let Some(&(owner, index)) = self.data.traits.parameters.get(&binding) {
                return TypeExpr::SourceParameter { owner, index };
            }
            if let Some(&(owner, index)) = self.data.extension_parameters.get(&binding) {
                return TypeExpr::SourceParameter { owner, index };
            }
            if let Some(extension) = self
                .data
                .extensions
                .iter()
                .find(|e| e.owner == binding && e.arity > 0)
            {
                let owner = Use {
                    binding,
                    domain: ExportDomain::Type,
                    local: true,
                };
                return TypeExpr::Apply(
                    Box::new(TypeExpr::Source {
                        usage: owner,
                        legacy: None,
                    }),
                    (0..extension.arity)
                        .map(|index| TypeExpr::SourceParameter { owner, index })
                        .collect(),
                );
            }
            if let Some(&(owner, index)) = self.parameters.get(&binding) {
                return TypeExpr::Parameter { owner, index };
            }
        }
        let local = self.data.locally_attested(&target);
        let binding = self.data.query(ExportDomain::Type, target);
        TypeExpr::Source {
            usage: Use {
                binding,
                domain: ExportDomain::Type,
                local,
            },
            legacy: (!local).then_some(legacy),
        }
    }

    fn exact_expr(&mut self, node: Node) -> TypeExpr {
        // Root member projection and exact argument identity are distinct.
        // Preserve indirection here; unsupported region evidence stays unknown.
        let forms = self.forms.types;
        if let Some(&(_, field, reference)) = forms
            .indirections
            .iter()
            .find(|&&(kind, _, _)| kind == node.kind())
        {
            use crate::type_checker::core::types::{Indirection, Lifetime, Mutability};
            let mut cursor = node.walk();
            let mutability = if node
                .named_children(&mut cursor)
                .any(|n| n.kind() == forms.mutable_node)
            {
                Mutability::Mutable
            } else {
                Mutability::Shared
            };
            let mut cursor = node.walk();
            let lifetime = node
                .named_children(&mut cursor)
                .find(|n| n.kind() == forms.lifetime_node)
                .map(|n| self.lifetime_expr(n))
                .or_else(|| reference.then(|| self.elided_region(node)).flatten());
            let (known, region) = match lifetime {
                Some(TypeExpr::Region(region)) => (region, None),
                Some(recipe) => (Lifetime::Unknown, Some(Box::new(recipe))),
                None => (Lifetime::Unknown, None),
            };
            let kind = if reference {
                Indirection::Reference(known)
            } else {
                Indirection::Pointer
            };
            let inner = node
                .child_by_field_name(field)
                .map(|n| self.exact_expr(n))
                .unwrap_or(TypeExpr::Unknown);
            return TypeExpr::Indirect {
                kind,
                mutability,
                region,
                inner: Box::new(inner),
            };
        }
        if forms.ignored_arguments.contains(&node.kind()) || forms.functions.contains(&node.kind())
        {
            return TypeExpr::Unknown;
        }
        if forms.tuples.contains(&node.kind()) {
            let mut cursor = node.walk();
            return TypeExpr::Tuple(
                node.named_children(&mut cursor)
                    .filter(|n| !n.is_extra())
                    .map(|n| self.exact_expr(n))
                    .collect(),
            );
        }
        self.expr(node)
    }

    fn lifetime_expr(&mut self, node: Node) -> TypeExpr {
        let text = node.utf8_text(self.source).unwrap_or_default();
        if text == self.forms.types.static_lifetime {
            return TypeExpr::Region(Lifetime::Static);
        }
        if text == self.forms.types.placeholder_lifetime {
            return self
                .elided_region(node)
                .unwrap_or(TypeExpr::Region(Lifetime::Unknown));
        }
        let name = self.data.intern(text, self.forms);
        let target = self
            .data
            .graph
            .scope_at(node.start_byte() as u32)
            .and_then(|scope| self.data.lookup(scope, name, ExportDomain::Lifetime));
        if let Some(Target::Binding(binding)) = target {
            if let Some(&(owner, index)) = self.data.traits.parameters.get(&binding) {
                return TypeExpr::SourceParameter { owner, index };
            }
            if let Some(&(owner, index)) = self.data.extension_parameters.get(&binding) {
                return TypeExpr::SourceParameter { owner, index };
            }
            if let Some(&(owner, index)) = self.parameters.get(&binding) {
                return TypeExpr::Parameter { owner, index };
            }
        }
        TypeExpr::Region(Lifetime::Unknown)
    }

    fn elided_region(&self, node: Node) -> Option<TypeExpr> {
        self.input_owner(node)
            .map(|owner| TypeExpr::InputRegion {
                owner,
                byte: node.start_byte() as u32,
            })
            .or_else(|| self.output_site(node).then_some(TypeExpr::OutputRegion))
    }

    fn input_owner(&self, node: Node) -> Option<usize> {
        if !self.forms.types.elide_input_lifetimes {
            return None;
        }
        self.signature_owner(node, self.forms.types.parameters)
            .and_then(|owner| self.slot(owner))
    }

    fn output_site(&self, node: Node) -> bool {
        self.forms.types.elide_output_lifetimes
            && self
                .signature_owner(node, self.forms.types.return_type)
                .is_some()
    }

    fn signature_owner<'t>(&self, node: Node<'t>, field: &str) -> Option<Node<'t>> {
        let mut ancestor = Some(node);
        while let Some(current) = ancestor {
            // A nested function type has its own binder, not the outer item's.
            if self.forms.types.functions.contains(&current.kind())
                || self.forms.locals.closures.contains(&current.kind())
            {
                return None;
            }
            if self.signature_item(current) {
                let region = current.child_by_field_name(field)?;
                return (region.start_byte() <= node.start_byte()
                    && node.end_byte() <= region.end_byte())
                .then_some(current);
            }
            ancestor = current.parent();
        }
        None
    }
}

pub(super) fn parameter_domain(kind: GenericParamKind) -> ExportDomain {
    match kind {
        GenericParamKind::Type => ExportDomain::Type,
        GenericParamKind::Lifetime => ExportDomain::Lifetime,
        GenericParamKind::Const => ExportDomain::Value,
    }
}

#[cfg(test)]
#[path = "namespace_types_tests.rs"]
mod tests;

#[path = "namespace_pattern_values.rs"]
pub(crate) mod patterns;
#[path = "namespace_receivers.rs"]
mod receivers;
#[path = "namespace_type_traits.rs"]
mod traits;
#[path = "namespace_value_initializers.rs"]
mod values;
