//! Ruby declaration callback contracts for RBI and RBS.

use crate::type_checker::profile::chain_specs::CallbackArgumentPolicy;

/// Decode the callback form admitted by one Ruby declaration parameter.
pub(crate) fn argument_policy(
    file_path: &str,
    signature: Option<&str>,
    index: usize,
) -> CallbackArgumentPolicy {
    let extension = file_path.rsplit('.').next();
    if extension.is_some_and(|extension| extension.eq_ignore_ascii_case("rbs")) {
        return CallbackArgumentPolicy::TrailingBlockOnly;
    }
    if !extension.is_some_and(|extension| extension.eq_ignore_ascii_case("rbi")) {
        return CallbackArgumentPolicy::Any;
    }
    match rbi_parameter(signature, index)
        .and_then(|parameter| parameter.trim_start().as_bytes().first().copied())
    {
        Some(b'^') => CallbackArgumentPolicy::PositionalOnly,
        Some(b'&') => CallbackArgumentPolicy::TrailingBlockOnly,
        // An unmarked, stale, or malformed RBI signature cannot authorize an
        // ordinary positional Proc callback.
        _ => CallbackArgumentPolicy::TrailingBlockOnly,
    }
}

/// Return one top-level parameter from the internal canonical RBI signature.
/// A callback's own `(A, B) -> R` parentheses remain inside its slot.
fn rbi_parameter(signature: Option<&str>, wanted: usize) -> Option<&str> {
    let signature = signature?;
    let open = signature.find('(')?;
    let mut depth = 0usize;
    let mut start = open + 1;
    let bytes = signature.as_bytes();
    let mut index = 0usize;
    for offset in open..bytes.len() {
        match bytes[offset] {
            b'(' => depth += 1,
            b')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return (index == wanted).then(|| signature[start..offset].trim());
                }
            }
            b',' if depth == 1 => {
                if index == wanted {
                    return Some(signature[start..offset].trim());
                }
                index += 1;
                start = offset + 1;
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rbi_proc_and_block_markers_remain_distinct() {
        assert_eq!(
            argument_policy("sig.rbi", Some("map(^callback: (Item) -> Result): void"), 0),
            CallbackArgumentPolicy::PositionalOnly
        );
        assert_eq!(
            argument_policy("sig.rbi", Some("map(&block: (Item) -> Result): void"), 0),
            CallbackArgumentPolicy::TrailingBlockOnly
        );
    }

    #[test]
    fn rbs_and_non_contract_sources_have_their_own_defaults() {
        assert_eq!(
            argument_policy("sig.rbs", Some("map: () { (Item) -> Result } -> void"), 0),
            CallbackArgumentPolicy::TrailingBlockOnly
        );
        assert_eq!(
            argument_policy("app.rb", None, 0),
            CallbackArgumentPolicy::Any
        );
    }
}
