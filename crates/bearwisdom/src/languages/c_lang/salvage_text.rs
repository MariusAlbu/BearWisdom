// =============================================================================
// c_lang/salvage_text.rs  —  shared byte-level scanners used by salvage modules
// =============================================================================

pub(super) fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

pub(super) fn collect_balanced_parens(source: &str, open_idx: usize) -> Option<(String, usize)> {
    let bytes = source.as_bytes();
    if bytes.get(open_idx).copied() != Some(b'(') {
        return None;
    }
    let mut depth = 1usize;
    let mut i = open_idx + 1;
    let args_start = i;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((source[args_start..i].to_string(), i + 1));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}
