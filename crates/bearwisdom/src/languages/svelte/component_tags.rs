//! Svelte component tag and component-file spelling.

pub(crate) fn component_tag_head(target: &str) -> Option<&str> {
    let head = target
        .split(['.', ':', '/'])
        .next()
        .unwrap_or(target)
        .trim();
    head.chars()
        .next()
        .filter(|ch| ch.is_ascii_uppercase())
        .map(|_| head)
}

pub(crate) fn is_component_file(path: &str) -> bool {
    path.ends_with(".svelte")
}
