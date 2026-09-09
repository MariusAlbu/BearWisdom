//! Attribute values decoded at the source-ingestion boundary, never in lookup.
use super::Forms;
use tree_sitter::Node;

pub(super) fn path_attribute(
    node: Node,
    source: &[u8],
    forms: &Forms,
) -> Result<Option<String>, ()> {
    let mut previous = node.prev_named_sibling();
    let mut result = None;
    while let Some(attribute) = previous {
        previous = attribute.prev_named_sibling();
        if forms.imports.trivia.contains(&attribute.kind()) {
            continue;
        }
        if !forms.attributes.contains(&attribute.kind()) {
            break;
        }
        let mut cursor = attribute.walk();
        for body in attribute
            .named_children(&mut cursor)
            .filter(|n| n.kind() == forms.attribute_body)
        {
            if body.named_child(0).and_then(|n| n.utf8_text(source).ok())
                != Some(forms.path_attribute)
            {
                continue;
            }
            if result.is_some() {
                return Err(());
            }
            let value = body
                .child_by_field_name("value")
                .ok_or(())?
                .utf8_text(source)
                .map_err(|_| ())?;
            // JSON-compatible quoted literals are a conservative subset. Unsupported
            // escape/raw/macro forms block the binding instead of using a default path.
            result = Some(serde_json::from_str::<String>(value).map_err(|_| ())?);
        }
    }
    Ok(result)
}

#[cfg(test)]
#[path = "namespace_source_files_tests.rs"]
mod tests;
