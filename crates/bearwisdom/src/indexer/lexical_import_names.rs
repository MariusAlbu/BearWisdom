//! Source-bound import names. This pass captures scope barriers, not targets.
use tree_sitter::Node;

pub(crate) struct Forms {
    pub statements: &'static [(&'static str, &'static str)],
    pub paths: &'static [(&'static str, &'static str, &'static str)],
    pub groups: &'static [(&'static str, &'static str, &'static str)],
    pub lists: &'static [&'static str],
    pub renames: &'static [(&'static str, &'static str)],
    pub identifiers: &'static [&'static str],
    pub self_leaf: &'static str,
    pub wildcards: &'static [&'static str],
    pub discarded: &'static [&'static str],
    pub trivia: &'static [&'static str],
}

#[derive(Default, Debug)]
pub(crate) struct Names<'a> {
    pub exposed: Vec<&'a str>,
    pub wildcard: bool,
    pub unknown: bool,
}

pub(crate) fn capture<'a>(node: Node, source: &'a [u8], forms: &Forms) -> Option<Names<'a>> {
    let &(_, field) = forms
        .statements
        .iter()
        .find(|&&(kind, _)| kind == node.kind())?;
    let mut names = Names {
        unknown: node.has_error(),
        ..Default::default()
    };
    match node.child_by_field_name(field) {
        Some(argument) => walk(argument, None, source, forms, &mut names),
        None => names.unknown = true,
    }
    Some(names)
}

fn leaf<'a>(
    node: Node,
    parent: Option<&'a str>,
    source: &'a [u8],
    forms: &Forms,
) -> Option<&'a str> {
    if node.kind() == forms.self_leaf {
        return parent;
    }
    if let Some(&(_, field, prefix)) = forms
        .paths
        .iter()
        .find(|&&(kind, _, _)| kind == node.kind())
    {
        let name = node.child_by_field_name(field)?;
        let parent = node
            .child_by_field_name(prefix)
            .and_then(|p| leaf(p, parent, source, forms));
        return leaf(name, parent, source, forms);
    }
    forms
        .identifiers
        .contains(&node.kind())
        .then(|| node.utf8_text(source).ok())
        .flatten()
}

fn walk<'a>(
    node: Node,
    parent: Option<&'a str>,
    source: &'a [u8],
    forms: &Forms,
    out: &mut Names<'a>,
) {
    if forms.trivia.contains(&node.kind()) {
        return;
    }
    if forms.wildcards.contains(&node.kind()) {
        out.wildcard = true;
        return;
    }
    if let Some(&(_, path, list)) = forms
        .groups
        .iter()
        .find(|&&(kind, _, _)| kind == node.kind())
    {
        let parent = node
            .child_by_field_name(path)
            .and_then(|p| leaf(p, parent, source, forms));
        match node.child_by_field_name(list) {
            Some(list) => walk(list, parent, source, forms, out),
            None => out.unknown = true,
        }
        return;
    }
    if forms.lists.contains(&node.kind()) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            walk(child, parent, source, forms, out);
        }
        return;
    }
    let name =
        if let Some(&(_, alias)) = forms.renames.iter().find(|&&(kind, _)| kind == node.kind()) {
            node.child_by_field_name(alias)
                .and_then(|n| n.utf8_text(source).ok())
        } else {
            leaf(node, parent, source, forms)
        };
    match name {
        Some(name) if forms.discarded.contains(&name) => {}
        Some(name) if !name.is_empty() => out.exposed.push(name),
        _ => out.unknown = true,
    }
}

#[cfg(test)]
#[path = "lexical_import_names_tests.rs"]
mod tests;
