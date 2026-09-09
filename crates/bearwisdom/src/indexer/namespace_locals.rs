//! Ordered value declarations and source-addressed initializers over an existing
//! namespace scope arena. Language differences are supplied as syntax data.
use super::*;
use crate::types::{SourceSpan, SymbolKind};
use tree_sitter::Node;

pub(crate) struct Forms {
    pub preserve_non_union_type: bool,
    pub declarations: &'static [(&'static str, &'static str, &'static str)],
    pub parameters: &'static [(&'static str, &'static str)],
    pub identifiers: &'static [&'static str],
    pub pattern_wrappers: &'static [&'static str],
    pub pattern_fields: &'static [(&'static str, &'static str)],
    pub annotation: &'static str,
    pub assignments: &'static [(&'static str, &'static str, &'static str)],
    pub calls: &'static [(&'static str, &'static str)],
    pub selectors: &'static [(&'static str, &'static str)],
    pub receivers: &'static [(&'static str, &'static str, &'static str)],
    pub direct_patterns: &'static [&'static str],
    pub callable_wrappers: &'static [(&'static str, &'static str)],
    pub value_wrappers: &'static [&'static str],
    pub closures: &'static [&'static str],
    pub barriers: &'static [&'static str],
}

struct Capture<'a, 'tree> {
    data: &'a mut NamespaceData,
    source: &'a [u8],
    forms: &'a super::Forms,
    declarations: Vec<(Node<'tree>, BindingId, SymbolKind)>,
    initializers: Vec<(BindingId, Node<'tree>)>,
    assignments: Vec<(Node<'tree>, Node<'tree>)>,
}

pub(super) fn capture(
    data: &mut NamespaceData,
    root: Node,
    source: &[u8],
    forms: &super::Forms,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &[ExtractedRef],
    policy: super::super::flow::BindingSymbols,
) {
    let mut capture = Capture {
        data,
        source,
        forms,
        declarations: Vec::new(),
        initializers: Vec::new(),
        assignments: Vec::new(),
    };
    capture.data.graph.preserve_non_union_type = forms.locals.preserve_non_union_type;
    capture.walk(root);
    // Some extractors already emit parameters as value rows. Adopt only the
    // exact coordinate's row kind; synthesizing a second row leaves a namesake
    // outside the lexical visibility fence. No spelling-based correlation.
    let anchors: HashMap<_, _> = symbols
        .iter()
        .filter(|s| matches!(s.kind, SymbolKind::Variable | SymbolKind::Parameter))
        .map(|s| ((s.start_line, s.start_col), s.kind))
        .collect();
    for (node, _, kind) in &mut capture.declarations {
        if *kind == SymbolKind::Parameter {
            let point = node.start_position();
            if let Some(row_kind) = anchors.get(&(point.row as u32, point.column as u32)) {
                *kind = *row_kind;
            }
        }
    }
    super::super::lexical::symbol_rows::reconcile(
        &mut capture.data.graph,
        &capture.declarations,
        source,
        symbols,
        policy,
    );
    capture
        .data
        .graph
        .source_owned_types
        .extend(capture.data.graph.symbols.keys().copied());
    capture.occurrences(root, refs);
    let mut calls: HashMap<_, Vec<usize>> = HashMap::new();
    for (index, reference) in refs.iter().enumerate().filter(|(_, r)| {
        matches!(
            r.kind,
            crate::types::EdgeKind::Calls | crate::types::EdgeKind::Instantiates
        )
    }) {
        let end = reference
            .chain
            .as_ref()
            .and_then(|c| c.segments.last())
            .map(|s| s.byte_offset)
            .unwrap_or(reference.byte_offset);
        calls
            .entry((reference.byte_offset, end))
            .or_default()
            .push(index);
    }
    for (binding, rhs) in std::mem::take(&mut capture.initializers) {
        capture.write(binding, rhs, &calls, true);
    }
    for (lhs, rhs) in std::mem::take(&mut capture.assignments) {
        if let Some(binding) = capture.value_binding(lhs) {
            capture.write(binding, rhs, &calls, false);
        }
    }
}

impl<'tree> Capture<'_, 'tree> {
    fn walk(&mut self, node: Node<'tree>) {
        let Some(scope) = self.data.graph.scope_at(node.start_byte() as u32) else {
            return;
        };
        let forms = self.forms.locals;
        if forms.barriers.contains(&node.kind()) {
            let barrier = if node.kind() == self.forms.module {
                node.child_by_field_name(self.forms.body)
                    .and_then(|body| self.data.graph.scope_at(body.start_byte() as u32))
            } else {
                Some(scope)
            };
            if let Some(barrier) = barrier {
                self.data.graph.capture_barriers.insert(barrier);
            }
        }
        if let Some(&(_, field)) = self
            .forms
            .patterns
            .arms
            .iter()
            .find(|&&(kind, _)| kind == node.kind())
        {
            if let Some(pattern) = node.child_by_field_name(field) {
                let available = pattern
                    .child_by_field_name(self.forms.patterns.condition)
                    .map(|n| n.start_byte())
                    .unwrap_or(pattern.end_byte());
                self.pattern(pattern, scope, available as u32, SymbolKind::Variable);
            }
        }
        if let Some(&(_, lhs, rhs)) = forms
            .declarations
            .iter()
            .find(|&&(kind, _, _)| kind == node.kind())
        {
            if let Some(pattern) = node.child_by_field_name(lhs) {
                let before = self.declarations.len();
                self.pattern(pattern, scope, node.end_byte() as u32, SymbolKind::Variable);
                if self.declarations.len() == before + 1
                    && forms.direct_patterns.contains(&pattern.kind())
                {
                    let binding = self.declarations[before].1;
                    if let Some(ty) = node.child_by_field_name(forms.annotation) {
                        self.annotation(binding, ty);
                    }
                    if let Some(rhs) = node.child_by_field_name(rhs) {
                        self.initializers.push((binding, rhs));
                    }
                }
            }
        }
        if let Some(&(_, field)) = forms
            .parameters
            .iter()
            .find(|&&(kind, _)| kind == node.kind())
        {
            let before = self.declarations.len();
            if field.is_empty() {
                let mut cursor = node.walk();
                for child in node
                    .named_children(&mut cursor)
                    .filter(|n| forms.identifiers.contains(&n.kind()))
                {
                    self.pattern(child, scope, node.end_byte() as u32, SymbolKind::Parameter);
                }
            } else if let Some(pattern) = node.child_by_field_name(field) {
                self.pattern(
                    pattern,
                    scope,
                    node.end_byte() as u32,
                    SymbolKind::Parameter,
                );
            }
            if self.declarations.len() == before + 1 {
                if let Some(ty) = node.child_by_field_name(forms.annotation) {
                    self.annotation(self.declarations[before].1, ty);
                }
            }
        }
        if let Some(&(_, lhs, rhs)) = forms
            .assignments
            .iter()
            .find(|&&(kind, _, _)| kind == node.kind())
        {
            if let (Some(lhs), Some(rhs)) =
                (node.child_by_field_name(lhs), node.child_by_field_name(rhs))
            {
                self.assignments.push((lhs, rhs));
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child);
        }
    }

    fn pattern(&mut self, node: Node<'tree>, scope: ScopeId, available: u32, kind: SymbolKind) {
        let forms = self.forms.locals;
        if forms.identifiers.contains(&node.kind()) {
            let Ok(text) = node.utf8_text(self.source) else {
                return;
            };
            let name = self.data.intern(text, self.forms);
            let binding = self
                .data
                .graph
                .declare_ordered(scope, name, available, None);
            self.data.graph.kinds.insert(binding, kind);
            self.data.graph.lexical_only.insert(binding);
            self.data.graph.declarations.insert(span(node), binding);
            self.data
                .graph
                .declaration_starts
                .insert(node.start_byte() as u32, binding);
            self.declarations.push((node, binding, kind));
            return;
        }
        if let Some(&(_, field)) = forms
            .pattern_fields
            .iter()
            .find(|&&(kind, _)| kind == node.kind())
        {
            if !field.is_empty() {
                if let Some(child) = node.child_by_field_name(field).or_else(|| {
                    self.forms
                        .patterns
                        .fields
                        .iter()
                        .find(|&&(form, _, _)| form == node.kind())
                        .and_then(|&(_, name, _)| node.child_by_field_name(name))
                }) {
                    self.pattern(child, scope, available, kind);
                }
                return;
            }
        } else if !forms.pattern_wrappers.contains(&node.kind()) {
            return;
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            // Constructor/type names in a pattern are not value declarations.
            if node.child_by_field_name("type") == Some(child)
                || node.child_by_field_name("name") == Some(child)
                || node.child_by_field_name(self.forms.patterns.condition) == Some(child)
            {
                continue;
            }
            self.pattern(child, scope, available, kind);
        }
    }

    fn annotation(&mut self, binding: BindingId, node: Node) {
        self.data.graph.bindings[binding.0].annotation =
            node.utf8_text(self.source).ok().map(str::to_owned);
    }

    fn value_binding(&self, node: Node) -> Option<BindingId> {
        if !self.forms.locals.identifiers.contains(&node.kind()) {
            return None;
        }
        let name = node
            .utf8_text(self.source)
            .ok()?
            .strip_prefix(self.forms.raw_prefix)
            .unwrap_or(node.utf8_text(self.source).ok()?);
        self.data
            .graph
            .binding_at(node.start_byte() as u32, self.data.graph.name_id(name)?)
    }

    fn occurrences(&mut self, root: Node, refs: &[ExtractedRef]) {
        let mut reads = Vec::new();
        for args in self.data.call_arguments.values().flatten() {
            for arg in args {
                arg.visit_identifiers(&mut |span| reads.push(span));
            }
        }
        for span in reads {
            if let Some(binding) = root
                .named_descendant_for_byte_range(span.start as usize, span.end as usize)
                .filter(|node| {
                    node.start_byte() == span.start as usize && node.end_byte() == span.end as usize
                })
                .and_then(|node| self.value_binding(node))
            {
                self.data.graph.argument_reads.insert(span, binding);
            }
        }
        for reference in refs {
            let byte = reference.byte_offset;
            if let Some(node) =
                root.named_descendant_for_byte_range(byte as usize, byte as usize + 1)
            {
                // Qualified paths choose a type namespace; a value namesake is irrelevant.
                if !node.parent().is_some_and(|p| {
                    self.forms
                        .path_nodes
                        .iter()
                        .any(|&(kind, _, _)| kind == p.kind())
                }) {
                    if let Some(binding) = self.value_binding(node) {
                        self.data.graph.references.insert(byte, binding);
                    }
                }
            }
        }
    }

    fn write(
        &mut self,
        binding: BindingId,
        mut rhs: Node,
        calls: &HashMap<(u32, u32), Vec<usize>>,
        initializer: bool,
    ) {
        let forms = self.forms.locals;
        while forms.value_wrappers.contains(&rhs.kind()) {
            let Some(child) = rhs.named_child(0) else {
                return;
            };
            rhs = child;
        }
        let Some(&(_, field)) = forms.calls.iter().find(|&&(kind, _)| kind == rhs.kind()) else {
            return;
        };
        let Some(mut callee) = rhs.child_by_field_name(field) else {
            return;
        };
        while let Some(&(_, field)) = forms
            .callable_wrappers
            .iter()
            .find(|&&(kind, _)| kind == callee.kind())
        {
            let Some(child) = callee.child_by_field_name(field) else {
                return;
            };
            callee = child;
        }
        let selector = forms
            .selectors
            .iter()
            .find(|&&(kind, _)| kind == callee.kind())
            .and_then(|&(_, field)| callee.child_by_field_name(field))
            .unwrap_or(callee)
            .start_byte() as u32;
        let address = (rhs.start_byte() as u32, selector);
        for &index in calls.get(&address).into_iter().flatten() {
            self.data.graph.writes.insert(index, binding);
            if initializer {
                self.data.graph.initializers.insert(index);
                if let Some(slot) = self
                    .data
                    .graph
                    .symbol_slots
                    .get(&binding)
                    .copied()
                    .flatten()
                {
                    self.data
                        .graph
                        .types
                        .call_initializers
                        .insert(slot, address);
                }
            }
        }
    }
}

fn span(node: Node) -> SourceSpan {
    SourceSpan {
        start: node.start_byte() as u32,
        end: node.end_byte() as u32,
    }
}

#[cfg(test)]
#[path = "namespace_locals_tests.rs"]
mod tests;
