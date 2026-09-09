//! Generic module declaration and qualified-occurrence capture over one CST.
use super::*;
use tree_sitter::Node;

struct PendingImport {
    binding: BindingId,
    scope: ScopeId,
    path: Vec<paths::Token>,
    domain: ExportDomain,
}
struct PendingExtension {
    binding: BindingId,
    scope: ScopeId,
    path: Option<Vec<paths::Token>>,
    parameters: Vec<(NameId, ExportDomain)>,
    arguments: Vec<(NameId, ExportDomain)>,
}
struct Builder<'a> {
    source: &'a [u8],
    forms: &'a Forms,
    symbols: &'a [ExtractedSymbol],
    data: NamespaceData,
    anchors: HashMap<(u32, u32), Option<usize>>,
    imports: Vec<PendingImport>,
    extensions: Vec<PendingExtension>,
}

pub(super) fn capture(
    root: Node,
    source: &[u8],
    forms: &Forms,
    symbols: &[ExtractedSymbol],
    refs: &[ExtractedRef],
) -> NamespaceData {
    let mut builder = Builder {
        source,
        forms,
        symbols,
        data: NamespaceData::default(),
        anchors: HashMap::new(),
        imports: Vec::new(),
        extensions: Vec::new(),
    };
    for (slot, symbol) in symbols.iter().enumerate() {
        builder
            .anchors
            .entry((symbol.start_line, symbol.start_col))
            .and_modify(|s| *s = None)
            .or_insert(Some(slot));
    }
    let scope = builder
        .data
        .graph
        .add_scope(None, 0, root.end_byte() as u32, true);
    builder.data.file_layout = Some((forms.file_extension, forms.directory_entry));
    builder.data.units.push(Unit {
        parent: None,
        scope,
        name: None,
        path: None,
        range: (0, root.end_byte() as u32),
    });
    builder.data.scope_units.insert(scope, SourceModuleId(0));
    builder.walk(root, scope);
    for import in &builder.imports {
        let target = paths::target(
            &builder.data,
            forms,
            import.scope,
            &import.path,
            import.domain,
        );
        builder.data.bindings[import.binding.0].targets.push(target);
    }
    for extension in &builder.extensions {
        let parameters = extension
            .parameters
            .iter()
            .map(|&(name, domain)| {
                builder
                    .data
                    .entries
                    .get(&(extension.scope, name, domain))
                    .copied()
            })
            .collect::<Option<std::collections::HashSet<_>>>();
        let arguments = extension
            .arguments
            .iter()
            .map(|&(name, domain)| {
                builder
                    .data
                    .entries
                    .get(&(extension.scope, name, domain))
                    .copied()
            })
            .collect::<Option<Vec<_>>>();
        let agreed = parameters.zip(arguments).filter(|(params, args)| {
            extension.path.is_some()
                && params.len() == extension.parameters.len()
                && params.len() == args.len()
                && args
                    .iter()
                    .copied()
                    .collect::<std::collections::HashSet<_>>()
                    == *params
        });
        let target = extension
            .path
            .as_ref()
            .filter(|_| agreed.is_some())
            .map(|path| {
                paths::target(
                    &builder.data,
                    forms,
                    extension.scope,
                    path,
                    ExportDomain::Type,
                )
            })
            .unwrap_or(Target::Missing);
        if let Some((_, args)) = agreed {
            for (index, binding) in args.into_iter().enumerate() {
                builder.data.extension_parameters.insert(
                    binding,
                    (
                        Use {
                            binding: extension.binding,
                            domain: ExportDomain::Type,
                            local: true,
                        },
                        index,
                    ),
                );
            }
        }
        builder.data.bindings[extension.binding.0].targets = vec![target];
    }
    for reference in refs
        .iter()
        .filter(|r| r.kind == crate::types::EdgeKind::Calls)
    {
        builder.reference(root, reference);
    }
    builder.trait_availability();
    builder.data
}

impl Builder<'_> {
    fn slot(&self, node: Node) -> Option<usize> {
        let point = node.start_position();
        self.anchors
            .get(&(point.row as u32, point.column as u32))
            .copied()
            .flatten()
    }
    fn access(&mut self, node: Node, unit: SourceModuleId) -> Option<Target> {
        let mut cursor = node.walk();
        let visibility = node
            .named_children(&mut cursor)
            .find(|n| n.kind() == self.forms.visibility);
        let Some(visibility) = visibility else {
            return Some(Target::Module(unit));
        };
        if visibility.utf8_text(self.source).ok() == Some(self.forms.public) {
            return None;
        }
        let path = visibility
            .named_child(0)
            .and_then(|n| paths::tokens(n, self.source, self.forms, &mut self.data));
        Some(
            path.as_deref()
                .map(|path| paths::restriction(&self.data, unit, path))
                .unwrap_or(Target::Missing),
        )
    }
    fn member_access(&mut self, node: Node, unit: SourceModuleId) -> Option<Target> {
        let mut ancestor = Some(node);
        while let Some(container) = ancestor {
            if self.conditional(container) {
                return Some(Target::Missing);
            }
            if container.kind() == self.forms.module {
                break;
            }
            ancestor = container.parent();
        }
        let mut cursor = node.walk();
        if node
            .named_children(&mut cursor)
            .any(|n| n.kind() == self.forms.visibility)
        {
            return self.access(node, unit);
        }
        let mut parent = node.parent();
        while let Some(container) = parent {
            if let Some(&(_, field)) = self
                .forms
                .public_member_containers
                .iter()
                .find(|&&(kind, _)| kind == container.kind())
            {
                if self.conditional(container) {
                    return Some(Target::Missing);
                }
                return if field.is_empty() || container.child_by_field_name(field).is_some() {
                    None
                } else {
                    Some(Target::Module(unit))
                };
            }
            if self.forms.scopes.contains(&container.kind())
                || container.kind() == self.forms.module
            {
                break;
            }
            parent = container.parent();
        }
        Some(Target::Module(unit))
    }
    fn export(
        &mut self,
        node: Node,
        scope: ScopeId,
        name: NameId,
        binding: BindingId,
        domain: ExportDomain,
    ) {
        let unit = self.data.scope_units[&scope];
        if scope == self.data.units[unit.0 as usize].scope {
            let access = self.access(node, unit);
            let declaration = !self
                .forms
                .imports
                .statements
                .iter()
                .any(|&(kind, _)| kind == node.kind());
            self.data.exports.push(Export {
                unit,
                name,
                binding,
                domain,
                access,
                declaration,
            });
        }
    }
    fn conditional(&self, node: Node) -> bool {
        let mut previous = node.prev_named_sibling();
        while let Some(attribute) = previous {
            previous = attribute.prev_named_sibling();
            if self.forms.imports.trivia.contains(&attribute.kind()) {
                continue;
            }
            if !self.forms.attributes.contains(&attribute.kind()) {
                break;
            }
            let mut cursor = attribute.walk();
            if attribute
                .named_children(&mut cursor)
                .filter(|n| n.kind() == self.forms.attribute_body)
                .filter_map(|n| n.named_child(0))
                .filter_map(|n| n.utf8_text(self.source).ok())
                .any(|name| self.forms.conditional_attributes.contains(&name))
            {
                return true;
            }
        }
        false
    }
    fn walk(&mut self, node: Node, inherited: ScopeId) {
        super::arguments::capture(node, self.source, self.forms, &mut self.data.call_arguments);
        let conditional = self.conditional(node);
        if let Some((span, mutable, owner)) = super::occurrences::borrow_site(node, self.forms) {
            if let Some(slot) = self
                .slot(owner)
                .filter(|_| !conditional && !self.conditional(owner))
            {
                self.data.borrow_sites.insert(span, (slot, mutable));
            }
        }
        if let Some((byte, owner)) = super::occurrences::method_site(node, self.forms) {
            if let Some(slot) = self
                .slot(owner)
                .filter(|_| !conditional && !self.conditional(owner))
            {
                self.data.method_calls.insert(byte, slot);
            }
        }
        if let Some(&(_, field)) = self
            .forms
            .imports
            .statements
            .iter()
            .find(|&&(kind, _)| kind == node.kind())
        {
            if crate::indexer::lexical::import_names::capture(node, self.source, self.forms.imports)
                .is_some_and(|names| names.wildcard || names.unknown)
            {
                self.data.opaque_scopes.insert(inherited);
            }
            let mut imports = Vec::new();
            if let Some(argument) = node.child_by_field_name(field) {
                paths::imports(
                    argument,
                    &[],
                    self.source,
                    self.forms,
                    &mut self.data,
                    &mut imports,
                );
            }
            for import in imports {
                if import.anonymous {
                    let domain = ExportDomain::Type;
                    let binding = self.data.query(domain, Target::Missing);
                    self.data
                        .traits
                        .anonymous_imports
                        .entry(inherited)
                        .or_default()
                        .push(binding);
                    if !conditional {
                        self.data.bindings[binding.0].targets.clear();
                        self.imports.push(PendingImport {
                            binding,
                            scope: inherited,
                            path: import.path,
                            domain,
                        });
                    }
                    continue;
                }
                for domain in [ExportDomain::Type, ExportDomain::Value, ExportDomain::Macro] {
                    let binding =
                        self.data
                            .declare(inherited, import.name, domain, Target::Missing);
                    // Reserve the binding before lowering any paths, including forward imports.
                    if !conditional {
                        self.data.bindings[binding.0].targets.pop();
                        self.imports.push(PendingImport {
                            binding,
                            scope: inherited,
                            path: import.path.clone(),
                            domain,
                        });
                    }
                    self.export(node, inherited, import.name, binding, domain);
                }
            }
            return;
        }
        let owner = self.data.scope_units[&inherited];
        if self.forms.access_items.contains(&node.kind())
            || self.forms.patterns.positional_field(node)
        {
            if let Some(slot) = self.slot(node) {
                let scope = self.member_access(node, owner);
                self.data.declaration_access.push(DeclarationAccess {
                    slot,
                    unit: owner,
                    scope,
                });
            }
        }
        if node.kind() == self.forms.module {
            let Some(name) = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(self.source).ok())
            else {
                return;
            };
            let name = self.data.intern(name, self.forms);
            let body = node.child_by_field_name(self.forms.body);
            let path = super::source_files::path_attribute(node, self.source, self.forms);
            let target = if let Some(body) = body.filter(|_| !conditional && path.is_ok()) {
                let id = SourceModuleId(self.data.units.len() as u32);
                let scope = self.data.graph.add_scope(
                    Some(inherited),
                    body.start_byte() as u32,
                    body.end_byte() as u32,
                    false,
                );
                self.data.units.push(Unit {
                    parent: Some(owner),
                    scope,
                    name: Some(name),
                    path: path.unwrap(),
                    range: (body.start_byte() as u32, body.end_byte() as u32),
                });
                self.data.scope_units.insert(scope, id);
                let binding =
                    self.data
                        .declare(inherited, name, ExportDomain::Type, Target::Module(id));
                self.export(node, inherited, name, binding, ExportDomain::Type);
                self.walk(body, scope);
                return;
            } else if !conditional && body.is_none() {
                match path {
                    Ok(path) => {
                        let id = self.data.source_files.len();
                        self.data
                            .source_files
                            .push(SourceFile { owner, name, path });
                        Target::SourceFile(id)
                    }
                    Err(()) => Target::Missing,
                }
            } else {
                Target::Missing
            };
            let binding = self
                .data
                .declare(inherited, name, ExportDomain::Type, target);
            self.export(node, inherited, name, binding, ExportDomain::Type);
            return;
        }
        if let Some(&(_, domains)) = self
            .forms
            .declarations
            .iter()
            .find(|&&(kind, _)| kind == node.kind())
        {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(self.source).ok())
            {
                let name = self.data.intern(name, self.forms);
                let target = self
                    .slot(node)
                    .filter(|_| !conditional)
                    .map(Target::Declaration)
                    .unwrap_or(Target::Missing);
                for &domain in domains {
                    let binding = self.data.declare(inherited, name, domain, target.clone());
                    self.export(node, inherited, name, binding, domain);
                }
            }
        }
        let scope = if self.forms.scopes.contains(&node.kind()) {
            let scope = self.data.graph.add_scope(
                Some(inherited),
                node.start_byte() as u32,
                node.end_byte() as u32,
                node.kind() == self.forms.function
                    || self.forms.locals.closures.contains(&node.kind()),
            );
            self.data.scope_units.insert(scope, owner);
            scope
        } else {
            inherited
        };
        if node.kind() == self.forms.extensions.extension {
            self.extension(node, scope, owner, conditional);
        }
        self.trait_header(node, scope, owner);
        if node.kind() == self.forms.function {
            if let Some(slot) = self.slot(node) {
                if let Some(parent) = self.symbols[slot].parent_index.filter(|&p| {
                    self.symbols.get(p).is_some_and(|s| {
                        matches!(
                            s.kind,
                            crate::types::SymbolKind::Struct
                                | crate::types::SymbolKind::Enum
                                | crate::types::SymbolKind::Class
                        )
                    })
                }) {
                    let name = self.data.intern(self.forms.self_type, self.forms);
                    if self.data.lookup(scope, name, ExportDomain::Type).is_none() {
                        self.data.declare(
                            scope,
                            name,
                            ExportDomain::Type,
                            Target::Declaration(parent),
                        );
                    }
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, scope);
        }
    }

    fn reference(&mut self, root: Node, reference: &ExtractedRef) {
        let byte = reference.byte_offset;
        let Some(mut node) = root.named_descendant_for_byte_range(byte as usize, byte as usize + 1)
        else {
            return;
        };
        while let Some(parent) = node.parent().filter(|p| {
            self.forms
                .path_nodes
                .iter()
                .any(|&(kind, _, _)| kind == p.kind())
        }) {
            node = parent;
        }
        let qualified = self
            .forms
            .path_nodes
            .iter()
            .any(|&(kind, _, _)| kind == node.kind());
        let mut callee = node;
        while let Some(parent) = callee.parent().filter(|parent| {
            self.forms
                .locals
                .callable_wrappers
                .iter()
                .any(|&(kind, field)| {
                    kind == parent.kind() && parent.child_by_field_name(field) == Some(callee)
                })
        }) {
            callee = parent;
        }
        let bare_call = self.forms.identifiers.contains(&node.kind())
            && callee.parent().is_some_and(|parent| {
                self.forms.locals.calls.iter().any(|&(kind, field)| {
                    kind == parent.kind() && parent.child_by_field_name(field) == Some(callee)
                })
            });
        if !qualified && !bare_call {
            return;
        }
        let Some(path) = paths::tokens(node, self.source, self.forms, &mut self.data) else {
            return;
        };
        let Some(scope) = self.data.graph.scope_at(byte) else {
            return;
        };
        for index in 0..path.len() {
            let domain = if index + 1 == path.len() {
                ExportDomain::Value
            } else {
                ExportDomain::Type
            };
            let target = paths::target(&self.data, self.forms, scope, &path[..=index], domain);
            // A captured bare-value lookup is authoritative even when provider
            // evidence is unknown. Dropping it reopens unrestricted name search.
            let local = bare_call || self.data.locally_attested(&target);
            let binding = self.data.query(domain, target);
            let usage = Use {
                binding,
                domain,
                local,
            };
            if index == 0 {
                self.data.roots.insert(byte, usage);
            } else {
                self.data.selectors.insert(path[index].1, usage);
            }
        }
    }
}

#[path = "namespace_extensions.rs"]
mod extensions;
#[cfg(test)]
#[path = "namespace_ingest_tests.rs"]
mod tests;
#[path = "namespace_ingest_traits.rs"]
mod trait_headers;
