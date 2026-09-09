//! Decode source paths once. Relations after the initial lookup carry IDs.
use super::*;
use tree_sitter::Node;

pub(super) type Token = (NameId, u32);
pub(super) fn tokens(
    node: Node,
    source: &[u8],
    forms: &Forms,
    data: &mut NamespaceData,
) -> Option<Vec<Token>> {
    if let Some(&(_, path, name)) = forms
        .path_nodes
        .iter()
        .find(|&&(kind, _, _)| kind == node.kind())
    {
        let mut out = tokens(node.child_by_field_name(path)?, source, forms, data)?;
        out.extend(tokens(
            node.child_by_field_name(name)?,
            source,
            forms,
            data,
        )?);
        return Some(out);
    }
    if !forms.identifiers.contains(&node.kind()) {
        return None;
    }
    Some(vec![(
        data.intern(node.utf8_text(source).ok()?, forms),
        node.start_byte() as u32,
    )])
}

pub(super) fn target(
    data: &NamespaceData,
    _forms: &Forms,
    scope: ScopeId,
    path: &[Token],
    domain: ExportDomain,
) -> Target {
    let Some(&(head, _)) = path.first() else {
        return Target::Missing;
    };
    let origin = data.scope_units[&scope];
    if let Some((base, consumed)) = rooted(data, origin, path) {
        return selected(base, &path[consumed..], domain, origin);
    }
    let head_domain = if path.len() == 1 {
        domain
    } else {
        ExportDomain::Type
    };
    let base = data
        .lookup(scope, head, head_domain)
        .unwrap_or(Target::External(head));
    selected(base, &path[1..], domain, origin)
}

/// Explicit roots never enter the lexical/external-name fallback ladder.
pub(super) fn rooted(
    data: &NamespaceData,
    origin: SourceModuleId,
    path: &[Token],
) -> Option<(Target, usize)> {
    let head = data.path_keywords.get(&path.first()?.0)?;
    if *head == PathKeyword::Crate {
        return Some((Target::CrateRoot, 1));
    }
    let start = usize::from(*head == PathKeyword::Current);
    let parents = path[start..]
        .iter()
        .take_while(|(name, _)| data.path_keywords.get(name) == Some(&PathKeyword::Parent))
        .count();
    let base = if parents == 0 {
        Target::Module(origin)
    } else {
        Target::Parent(origin, parents as u32)
    };
    Some((base, start + parents))
}

pub(super) fn restriction(data: &NamespaceData, origin: SourceModuleId, path: &[Token]) -> Target {
    let Some((base, consumed)) = rooted(data, origin, path) else {
        return Target::Missing;
    };
    let names: Vec<_> = path[consumed..].iter().map(|&(name, _)| name).collect();
    if names
        .iter()
        .any(|name| data.path_keywords.contains_key(name))
    {
        return Target::Missing;
    }
    if names.is_empty() {
        base
    } else {
        Target::DeclarationPath(Box::new(base), names)
    }
}

fn selected(base: Target, path: &[Token], domain: ExportDomain, origin: SourceModuleId) -> Target {
    let selectors: Vec<_> = path
        .iter()
        .enumerate()
        .map(|(index, &(name, _))| {
            (
                name,
                if index + 1 == path.len() {
                    domain
                } else {
                    ExportDomain::Type
                },
            )
        })
        .collect();
    if selectors.is_empty() {
        base
    } else {
        Target::Select(Box::new(base), selectors, origin)
    }
}

pub(super) struct Imported {
    pub name: NameId,
    pub path: Vec<Token>,
    pub anonymous: bool,
}
pub(super) fn imports(
    node: Node,
    prefix: &[Token],
    source: &[u8],
    forms: &Forms,
    data: &mut NamespaceData,
    out: &mut Vec<Imported>,
) {
    let shapes = forms.imports;
    if shapes.trivia.contains(&node.kind()) || shapes.wildcards.contains(&node.kind()) {
        return;
    }
    if let Some(&(_, path, list)) = shapes
        .groups
        .iter()
        .find(|&&(kind, _, _)| kind == node.kind())
    {
        let Some(path) = node
            .child_by_field_name(path)
            .and_then(|n| tokens(n, source, forms, data))
        else {
            return;
        };
        let combined: Vec<_> = prefix.iter().chain(&path).copied().collect();
        if let Some(list) = node.child_by_field_name(list) {
            imports(list, &combined, source, forms, data, out);
        }
        return;
    }
    if shapes.lists.contains(&node.kind()) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            imports(child, prefix, source, forms, data, out);
        }
        return;
    }
    let (alias, path, anonymous) = if let Some(&(_, alias)) = shapes
        .renames
        .iter()
        .find(|&&(kind, _)| kind == node.kind())
    {
        let Some(alias) = node
            .child_by_field_name(alias)
            .and_then(|n| n.utf8_text(source).ok())
        else {
            return;
        };
        (
            Some(data.intern(alias, forms)),
            node.child_by_field_name(forms.rename_path),
            shapes.discarded.contains(&alias),
        )
    } else {
        (None, Some(node), false)
    };
    let Some(mut path) = path.and_then(|n| tokens(n, source, forms, data)) else {
        return;
    };
    if path.len() == 1
        && data.path_keywords.get(&path[0].0) == Some(&PathKeyword::Current)
        && !prefix.is_empty()
    {
        path.clear();
    }
    let path: Vec<_> = prefix.iter().chain(&path).copied().collect();
    let Some(name) = alias.or_else(|| path.last().map(|&(name, _)| name)) else {
        return;
    };
    out.push(Imported {
        name,
        path,
        anonymous,
    });
}

#[cfg(test)]
#[path = "namespace_paths_tests.rs"]
mod tests;
