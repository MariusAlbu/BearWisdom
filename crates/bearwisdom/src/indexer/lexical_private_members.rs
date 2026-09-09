//! Private member tokens bind in their lexical class, not the receiver's name set.
use super::*;

pub(in crate::indexer::lexical) fn capture(
    mut token: Node,
    source: &[u8],
    syntax: &LexicalSyntax,
    symbols: &[ExtractedSymbol],
) -> Option<usize> {
    let spelling = token.utf8_text(source).ok()?;
    while let Some(parent) = token.parent() {
        token = parent;
        if !syntax.base_types.0.contains(&token.kind()) {
            continue;
        }
        let body = token.child_by_field_name("body")?;
        let mut cursor = body.walk();
        let declarations: Vec<_> = body
            .named_children(&mut cursor)
            .filter(|member| {
                member.child_by_field_name("name").is_some_and(|name| {
                    name.kind() == syntax.globals.private_member.0
                        && name.utf8_text(source).ok() == Some(spelling)
                })
            })
            .collect();
        if declarations.is_empty() {
            continue;
        }
        let [declaration] = declarations.as_slice() else {
            return None;
        };
        let mut cursor = declaration.walk();
        if declaration
            .children(&mut cursor)
            .any(|n| n.kind() == syntax.globals.private_member.1)
        {
            return None;
        }
        let point = declaration.start_position();
        let rows: Vec<_> = symbols
            .iter()
            .enumerate()
            .filter(|(_, symbol)| {
                symbol.start_line == point.row as u32 && symbol.start_col == point.column as u32
            })
            .map(|(slot, _)| slot)
            .collect();
        return match rows.as_slice() {
            [slot] => Some(*slot),
            _ => None,
        };
    }
    None
}

#[cfg(test)]
#[path = "lexical_private_members_tests.rs"]
mod tests;
