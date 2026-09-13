//! Return-type evidence in the display signature the assembly cracker renders:
//! `Name<GP>(params): Ret`. The return slot follows the parameter list, so a
//! declaration whose result text happens to spell its own name decodes the
//! same as any other.

/// The return slot of a cracked-assembly display signature, or `None` when the
/// line has no `): Ret` tail or the tail names no result type.
pub fn return_type(signature: &str) -> Option<String> {
    let signature = signature.trim();
    let open = signature.find('(')?;
    let close = open + matching_paren(&signature[open..])?;
    let tail = signature[close + 1..]
        .trim_start()
        .strip_prefix(':')?
        .trim();
    (!tail.is_empty() && tail != "void" && tail != "System.Void").then(|| tail.to_string())
}

/// Offset of the `)` closing the `(` that `text` starts with, depth-aware so a
/// parenthesised parameter type does not end the list early.
fn matching_paren(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, ch) in text.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
#[path = "signature_tests.rs"]
mod tests;
