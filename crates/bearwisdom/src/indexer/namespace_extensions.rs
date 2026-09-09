//! Detached owner and blanket parameter correspondence, captured from syntax.
use super::*;
use crate::type_checker::core::types::GenericParamKind;

impl Builder<'_> {
    pub(super) fn extension(
        &mut self,
        node: Node,
        scope: ScopeId,
        owner: SourceModuleId,
        conditional: bool,
    ) {
        let forms = self.forms.extensions;
        let types = self.forms.types;
        let mut cursor = node.walk();
        let mut unsupported = conditional
            || forms
                .excluded_fields
                .iter()
                .any(|field| node.child_by_field_name(field).is_some())
            || node
                .named_children(&mut cursor)
                .any(|n| self.forms.extension_constraints.contains(&n.kind()));
        let mut parameters = Vec::new();
        if let Some(params) = node.child_by_field_name(types.generic_parameters) {
            let mut cursor = params.walk();
            for param in params.named_children(&mut cursor).filter(|n| !n.is_extra()) {
                // Only a plain parameter is an unconditional blanket. Bounds,
                // defaults and const parameters need applicability rules.
                let name = param.child_by_field_name("name");
                let kind = types
                    .generic_parameter_forms
                    .iter()
                    .find(|&&(syntax, _)| syntax == param.kind())
                    .map(|&(_, kind)| kind);
                let Some(kind) = kind.filter(|kind| {
                    *kind != GenericParamKind::Const
                        && param.named_child_count() == 1
                        && name.is_some()
                }) else {
                    unsupported = true;
                    continue;
                };
                if let Some(name) = name.and_then(|n| n.utf8_text(self.source).ok()) {
                    parameters.push((
                        self.data.intern(name, self.forms),
                        super::types::parameter_domain(kind),
                    ));
                } else {
                    unsupported = true;
                }
            }
        }
        let mut arguments = Vec::new();
        let mut kinds = Vec::new();
        let mut target = node.child_by_field_name(forms.target);
        if let Some((application, &(_, head, args))) = target.and_then(|n| {
            types
                .applications
                .iter()
                .find(|&&(kind, _, _)| kind == n.kind())
                .map(|form| (n, form))
        }) {
            target = application.child_by_field_name(head);
            if let Some(args) = application.child_by_field_name(args) {
                let mut cursor = args.walk();
                for arg in args.named_children(&mut cursor).filter(|n| !n.is_extra()) {
                    if arg.kind() == types.lifetime_node {
                        if let Ok(name) = arg.utf8_text(self.source) {
                            arguments
                                .push((self.data.intern(name, self.forms), ExportDomain::Lifetime));
                            kinds.push(GenericParamKind::Lifetime);
                            continue;
                        }
                    }
                    let path = paths::tokens(arg, self.source, self.forms, &mut self.data);
                    if let Some(path) = path.filter(|path| path.len() == 1) {
                        arguments.push((path[0].0, ExportDomain::Type));
                        kinds.push(GenericParamKind::Type);
                    } else {
                        unsupported = true;
                    }
                }
            } else {
                unsupported = true;
            }
        }
        let path = target
            .filter(|_| !unsupported)
            .and_then(|target| paths::tokens(target, self.source, self.forms, &mut self.data));
        let name = self.data.intern(self.forms.self_type, self.forms);
        let binding = self
            .data
            .declare(scope, name, ExportDomain::Type, Target::Missing);
        let members = node
            .child_by_field_name(self.forms.body)
            .map(|body| {
                let mut cursor = body.walk();
                body.named_children(&mut cursor)
                    .filter(|n| forms.members.contains(&n.kind()))
                    .filter_map(|n| self.slot(n))
                    .collect()
            })
            .unwrap_or_default();
        self.data.extensions.push(Extension {
            owner: binding,
            unit: owner,
            members,
            arity: arguments.len(),
            kinds,
        });
        self.extensions.push(PendingExtension {
            binding,
            scope,
            path,
            parameters,
            arguments,
        });
    }
}

#[cfg(test)]
#[path = "namespace_extensions_tests.rs"]
mod tests;
