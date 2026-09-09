//! Syntax ingestion only. Node kinds/field names and source spelling are decoded
//! here; the resulting graph contains scope/binding IDs, not qualified names.
use super::{BindingId, LexicalBindings, ScopeId};
use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

#[cfg(test)]
#[path = "lexical_ingest_tests.rs"]
mod tests;

#[path = "lexical_reference_roots.rs"]
mod reference_roots;
use reference_roots::reference_root;

pub(crate) struct LexicalSyntax {
    pub declaration_modifiers: &'static [&'static str],
    pub globals: super::globals::Forms,
    pub base_types: (
        &'static [&'static str],
        &'static str,
        &'static str,
        &'static str,
    ),
    pub base_head: (&'static str, &'static str, &'static str),
    pub type_bases: (&'static [&'static str], &'static str, &'static str),
    pub modules: &'static super::modules::ModuleForms,
    pub functions: &'static [&'static str],
    pub named_declarations: &'static [(&'static str, SymbolKind)],
    pub overload_declarations: &'static [&'static str],
    pub named_expressions: &'static [(&'static str, SymbolKind)],
    pub type_declarations: &'static [(&'static str, SymbolKind)],
    pub dual_declarations: &'static [SymbolKind],
    pub merge_declarations: &'static [(SymbolKind, SymbolKind)],
    pub type_scopes: &'static [&'static str],
    pub type_forms: &'static [(&'static str, super::type_syntax::TypeForm)],
    pub alias_values: &'static [(&'static str, &'static str)],
    pub compiler_intrinsics: &'static super::type_syntax::compiler_intrinsics::Forms,
    pub call_roots: &'static [(&'static str, &'static str)],
    pub call_selectors: &'static [(&'static str, &'static str)],
    pub call_identifiers: &'static [&'static str],
    pub initializer_forms: &'static super::type_syntax::initializers::Forms,
    pub callback_forms: &'static super::globals::call_arguments::callbacks::Forms,
    pub reference_roots: &'static [(&'static str, &'static str)],
    pub reference_wrappers: &'static [&'static str],
    pub blocks: &'static [&'static str],
    pub variables: &'static [&'static str],
    pub assignments: &'static [&'static str],
    pub function_scoped_declaration: &'static str,
    pub literal_types: &'static [(&'static str, &'static str)],
    pub preserve_non_union_type: bool,
}

pub(crate) fn syntax_for(prefix: &str) -> Option<&'static LexicalSyntax> {
    match prefix {
        "ts" | "js" => Some(&crate::languages::typescript::flow::TS_LEXICAL_SYNTAX),
        _ => None,
    }
}

pub(crate) fn capture(
    root: Node,
    source: &[u8],
    prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &[ExtractedRef],
    policy: crate::indexer::flow_bindings::BindingSymbols,
) -> Option<LexicalBindings> {
    let syntax = syntax_for(prefix)?;
    let mut builder = Builder {
        graph: LexicalBindings::default(),
        source,
        syntax,
        declarations: Vec::new(),
        assignments: Vec::new(),
        expression_initializers: Vec::new(),
    };
    let scope =
        builder
            .graph
            .add_scope(None, root.start_byte() as u32, root.end_byte() as u32, true);
    builder.graph.preserve_non_union_type = syntax.preserve_non_union_type;
    builder.walk(root, scope);
    builder.graph.module =
        super::modules::capture(root, source, syntax.modules, &mut builder.graph);
    super::selections::capture(root, source, syntax.modules, &mut builder.graph);
    for (name, expression) in &builder.expression_initializers {
        let variable = name
            .utf8_text(source)
            .ok()
            .and_then(|name| builder.graph.name_id(name))
            .and_then(|id| builder.graph.binding_at(name.start_byte() as u32, id));
        let value = expression.child_by_field_name("name").and_then(|name| {
            builder
                .graph
                .declarations
                .get(&crate::types::SourceSpan {
                    start: name.start_byte() as u32,
                    end: name.end_byte() as u32,
                })
                .copied()
        });
        if let (Some(variable), Some(value)) = (variable, value) {
            builder.graph.initial_values.insert(variable, value);
        }
    }
    // All declarations exist before assignment references are bound. A nearer
    // lexical declaration shadows an outer one even before initialization.
    for (lhs, rhs, initializer) in &builder.assignments {
        if let Ok(name) = lhs.utf8_text(source) {
            let binding = builder
                .graph
                .name_id(name)
                .and_then(|name| builder.graph.binding_at(lhs.start_byte() as u32, name));
            if *initializer {
                let value = reference_root(*rhs, rhs.start_byte() as u32, syntax)
                    .filter(|token| token.byte_range() == rhs.byte_range())
                    .and_then(|token| token.utf8_text(source).ok())
                    .and_then(|name| builder.graph.name_id(name))
                    .and_then(|name| builder.graph.binding_at(rhs.start_byte() as u32, name))
                    .filter(|id| {
                        matches!(
                            builder.graph.kinds.get(id),
                            Some(SymbolKind::Class | SymbolKind::Function)
                        )
                    });
                if let (Some(binding), Some(value)) = (binding, value) {
                    builder.graph.initial_values.insert(binding, value);
                }
            }
            if let (Some(binding), Some(reference)) = (
                binding,
                crate::indexer::flow_bindings::correlate_rhs_ref(refs, rhs, prefix),
            ) {
                builder.graph.writes.insert(reference, binding);
                if *initializer {
                    builder.graph.initializers.insert(reference);
                }
            }
        }
    }
    super::declaration_sites::normalize(&mut builder.graph, &builder.declarations, symbols, syntax);
    super::symbol_rows::reconcile(
        &mut builder.graph,
        &builder.declarations,
        source,
        symbols,
        policy,
    );
    super::merges::capture(&mut builder.graph, symbols, syntax);
    super::type_syntax::capture(root, source, syntax, symbols, &mut builder.graph);
    builder.graph.globals = Some(super::globals::capture(
        root,
        source,
        syntax,
        symbols,
        &mut builder.graph,
    ));
    // Decode spelling once at ingestion, including the root of a member chain.
    // Use the reference's expression anchor: legacy root segments may still
    // carry a zero placeholder byte, which is not a declaration-use location.
    for reference in refs {
        for segment in reference
            .chain
            .iter()
            .flat_map(|c| c.segments.iter().skip(1))
        {
            let byte = segment.byte_offset as usize;
            if let Some(node) = root
                .named_descendant_for_byte_range(byte, byte + 1)
                .filter(|n| n.kind() == syntax.globals.private_member.0)
            {
                let declaration =
                    super::globals::private_members::capture(node, source, syntax, symbols);
                builder
                    .graph
                    .globals
                    .as_mut()
                    .unwrap()
                    .private_selectors
                    .insert(segment.byte_offset, declaration);
            }
            if let Some(node) = root
                .named_descendant_for_byte_range(byte, byte + 1)
                .filter(|n| syntax.globals.member_names.contains(&n.kind()))
            {
                if let Ok(spelling) = node.utf8_text(source) {
                    let name = builder.graph.intern(spelling);
                    builder
                        .graph
                        .globals
                        .as_mut()
                        .unwrap()
                        .selectors
                        .insert(segment.byte_offset, name);
                }
            }
        }
        let segment_args = reference
            .chain
            .iter()
            .flat_map(|c| &c.segments)
            .flat_map(|s| &s.call_args);
        for arg in reference.call_args.iter().chain(segment_args) {
            arg.visit_identifiers(&mut |span| {
                let name = source
                    .get(span.start as usize..span.end as usize)
                    .and_then(|s| std::str::from_utf8(s).ok())
                    .map(|name| builder.graph.intern(name));
                let binding =
                    name.and_then(|name| builder.graph.reference_binding_at(span.start, name));
                if let Some(binding) = binding {
                    builder.graph.argument_reads.insert(span, binding);
                }
                if let Some(name) =
                    name.filter(|&name| builder.graph.binding_at(span.start, name).is_none())
                {
                    builder
                        .graph
                        .globals
                        .as_mut()
                        .unwrap()
                        .arguments
                        .insert(span, name);
                }
            });
        }
        let byte = reference.byte_offset;
        let token = reference_root(root, byte, syntax);
        let name = token
            .filter(|n| syntax.globals.names.contains(&n.kind()))
            .and_then(|n| n.utf8_text(source).ok())
            .map(|name| builder.graph.intern(name));
        if let Some(binding) = name.and_then(|name| builder.graph.reference_binding_at(byte, name))
        {
            builder.graph.references.insert(byte, binding);
        }
        if let Some(name) = name.filter(|&name| builder.graph.binding_at(byte, name).is_none()) {
            builder
                .graph
                .globals
                .as_mut()
                .unwrap()
                .values
                .insert(byte, name);
        }
    }
    Some(builder.graph)
}

struct Builder<'a, 'tree> {
    graph: LexicalBindings,
    source: &'a [u8],
    syntax: &'a LexicalSyntax,
    declarations: Vec<(Node<'tree>, BindingId, SymbolKind)>,
    assignments: Vec<(Node<'tree>, Node<'tree>, bool)>,
    expression_initializers: Vec<(Node<'tree>, Node<'tree>)>,
}

impl<'a, 'tree> Builder<'a, 'tree> {
    fn walk(&mut self, node: Node<'tree>, inherited: ScopeId) {
        self.named_declaration(node, inherited);
        self.call_type_args(node);
        let inherited = if let Some(&(_, kind)) =
            self.syntax.named_expressions.iter().find(|&&(syntax, _)| {
                syntax == node.kind() && node.child_by_field_name("name").is_some()
            }) {
            // The self-name's environment is outside the parameter environment.
            let private = self.graph.add_scope(
                Some(inherited),
                node.start_byte() as u32,
                node.end_byte() as u32,
                false,
            );
            if let Some(binding) = self.declaration(node, private, kind) {
                self.graph.lexical_only.insert(binding);
            }
            private
        } else {
            inherited
        };
        let function = self.syntax.functions.contains(&node.kind());
        let module = super::modules::scopes::kind(node, self.syntax.modules).is_some();
        let scoped = function
            || module
            || self.syntax.blocks.contains(&node.kind())
            || self.syntax.type_scopes.contains(&node.kind());
        let scope = if scoped {
            self.graph.add_scope(
                Some(inherited),
                node.start_byte() as u32,
                node.end_byte() as u32,
                function || module,
            )
        } else {
            inherited
        };
        let scope = super::type_syntax::structural::scope(
            node,
            self.source,
            self.syntax,
            &mut self.graph,
            scope,
        );
        self.type_parameters(node, scope);
        if function {
            if let Some(params) = node.child_by_field_name("parameters") {
                let mut cursor = params.walk();
                for param in params.named_children(&mut cursor) {
                    self.pattern(
                        param,
                        scope,
                        param.end_byte() as u32,
                        annotation(param, self.source),
                        SymbolKind::Parameter,
                    );
                }
            } else if let Some(param) = node.child_by_field_name("parameter") {
                self.pattern(
                    param,
                    scope,
                    param.end_byte() as u32,
                    None,
                    SymbolKind::Parameter,
                );
            }
        }
        if self.syntax.variables.contains(&node.kind()) {
            if let Some(name) = node.child_by_field_name("name") {
                let owner = if node
                    .parent()
                    .is_some_and(|p| p.kind() == self.syntax.function_scoped_declaration)
                {
                    self.graph.scopes[scope.0].function
                } else {
                    scope
                };
                let ty = annotation(node, self.source).or_else(|| {
                    let value = node.child_by_field_name("value")?;
                    self.syntax
                        .literal_types
                        .iter()
                        .find(|(kind, _)| *kind == value.kind())
                        .map(|(_, wrapper)| (*wrapper).to_owned())
                });
                self.pattern(
                    name,
                    owner,
                    node.end_byte() as u32,
                    ty,
                    SymbolKind::Variable,
                );
                if let Some(value) = node.child_by_field_name("value") {
                    if name.kind() == "identifier" {
                        if self
                            .syntax
                            .named_expressions
                            .iter()
                            .any(|&(kind, _)| kind == value.kind())
                        {
                            self.expression_initializers.push((name, value));
                        } else {
                            self.assignments.push((name, value, true));
                        }
                    }
                }
            }
        }
        if self.syntax.assignments.contains(&node.kind()) {
            if let (Some(lhs), Some(rhs)) = (
                node.child_by_field_name("left"),
                node.child_by_field_name("right"),
            ) {
                if lhs.kind() == "identifier" {
                    self.assignments.push((lhs, rhs, false));
                }
            }
        }
        if node.kind() == "catch_clause" {
            if let Some(param) = node.child_by_field_name("parameter") {
                self.pattern(
                    param,
                    scope,
                    param.end_byte() as u32,
                    None,
                    SymbolKind::Variable,
                );
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            let child_scope = super::type_syntax::inference::child_scope(
                node,
                child,
                self.source,
                self.syntax,
                &mut self.graph,
                scope,
            );
            self.walk(child, child_scope);
        }
    }

    fn call_type_args(&mut self, node: Node<'tree>) {
        let Some(&(_, field)) = self
            .syntax
            .call_roots
            .iter()
            .find(|&&(kind, _)| kind == node.kind())
        else {
            return;
        };
        let Some(callee) = node.child_by_field_name(field) else {
            return;
        };
        // Member calls belong to their own segment, not to the lexical root.
        if !matches!(callee.kind(), "identifier" | "type_identifier") {
            return;
        }
        let Some(args) = node.child_by_field_name("type_arguments") else {
            return;
        };
        let mut cursor = args.walk();
        let args = args
            .named_children(&mut cursor)
            .filter(|arg| !arg.is_extra())
            .filter_map(|arg| arg.utf8_text(self.source).ok().map(str::to_owned))
            .collect();
        self.graph
            .call_type_args
            .insert(node.start_byte() as u32, args);
    }

    fn named_declaration(&mut self, node: Node<'tree>, scope: ScopeId) {
        let Some(&(_, kind)) = self
            .syntax
            .named_declarations
            .iter()
            .chain(self.syntax.type_declarations)
            .find(|&&(syntax, _)| syntax == node.kind())
        else {
            return;
        };
        self.declaration(node, scope, kind);
    }

    fn declaration(
        &mut self,
        node: Node<'tree>,
        scope: ScopeId,
        kind: SymbolKind,
    ) -> Option<BindingId> {
        let name_node = node.child_by_field_name("name")?;
        let name = name_node.utf8_text(self.source).ok()?;
        let name = self.graph.intern(name);
        // Identity exists throughout the declaring scope, even when runtime
        // initialization/TDZ diagnostics forbid evaluating the declaration.
        let id = if self
            .syntax
            .type_declarations
            .iter()
            .any(|&(_, k)| k == kind)
        {
            self.graph.declare_type(scope, name)
        } else {
            self.graph
                .declare(scope, name, self.graph.scopes[scope.0].start, None)
        };
        if self.syntax.dual_declarations.contains(&kind) {
            let ty = self.graph.declare_type(scope, name);
            self.graph.dual_types.insert(id, ty);
        }
        self.graph.kinds.insert(id, kind);
        if self.syntax.overload_declarations.contains(&node.kind()) {
            self.graph.overload_bindings.insert(id);
        }
        self.graph.declarations.insert(
            crate::types::SourceSpan {
                start: name_node.start_byte() as u32,
                end: name_node.end_byte() as u32,
            },
            id,
        );
        // Extractors anchor named declarations at the whole declaration node.
        self.declarations.push((node, id, kind));
        Some(id)
    }

    fn type_parameters(&mut self, node: Node<'tree>, scope: ScopeId) {
        let Some(parameters) = node.child_by_field_name("type_parameters") else {
            return;
        };
        let owner = node.start_position();
        let mut cursor = parameters.walk();
        for (index, parameter) in parameters
            .named_children(&mut cursor)
            .filter(|n| !n.is_extra())
            .enumerate()
        {
            let Some(name) = parameter.child_by_field_name("name") else {
                continue;
            };
            let Ok(name) = name.utf8_text(self.source) else {
                continue;
            };
            let name = self.graph.intern(name);
            let binding = self.graph.declare_type(scope, name);
            self.graph
                .type_parameters
                .insert(binding, (owner.row as u32, owner.column as u32, index));
            self.graph.type_parameter_sites.insert(
                binding,
                super::type_syntax::signatures::SignatureId(crate::types::SourceSpan {
                    start: node.start_byte() as u32,
                    end: node.end_byte() as u32,
                }),
            );
        }
    }

    fn pattern(
        &mut self,
        node: Node<'tree>,
        scope: ScopeId,
        available_from: u32,
        ty: Option<String>,
        kind: SymbolKind,
    ) {
        match node.kind() {
            "identifier" | "shorthand_property_identifier_pattern" => {
                let Ok(name) = node.utf8_text(self.source) else {
                    return;
                };
                let name = self.graph.intern(name);
                let id = self.graph.declare(scope, name, available_from, ty);
                self.graph.kinds.entry(id).or_insert(kind);
                self.graph.declarations.insert(
                    crate::types::SourceSpan {
                        start: node.start_byte() as u32,
                        end: node.end_byte() as u32,
                    },
                    id,
                );
                self.declarations.push((node, id, kind));
            }
            "required_parameter" | "optional_parameter" => {
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    self.pattern(pattern, scope, available_from, ty, kind);
                }
            }
            "assignment_pattern" | "object_assignment_pattern" => {
                if let Some(left) = node.child_by_field_name("left") {
                    self.pattern(left, scope, available_from, ty, kind);
                }
            }
            "pair_pattern" => {
                if let Some(value) = node.child_by_field_name("value") {
                    self.pattern(value, scope, available_from, None, kind);
                }
            }
            "rest_pattern" | "object_pattern" | "array_pattern" => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    self.pattern(child, scope, available_from, None, kind);
                }
            }
            _ => {}
        }
    }
}

fn annotation(node: Node, source: &[u8]) -> Option<String> {
    let ty = node.child_by_field_name("type")?;
    ty.named_child(0)?.utf8_text(source).ok().map(str::to_owned)
}
