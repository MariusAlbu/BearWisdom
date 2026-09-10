//! JVM bytecode signature evidence owned by the Maven ecosystem.
//!
//! Resolution only supplies a stored signature string.  This decoder accepts
//! the bytecode descriptor grammar itself, so callers do not need to know
//! whether the source language was Java, Kotlin, Scala, Groovy, or Clojure.

/// Decode a JVM bytecode descriptor to the chainable element type.
///
/// Method descriptors contribute their result; field descriptors are decoded
/// directly. Arrays are projected to their element and primitives/void have no
/// member-bearing result.
pub fn return_type(signature: &str) -> Option<String> {
    let signature = signature.trim();
    let descriptor = if let Some(rest) = signature.strip_prefix('(') {
        let close = rest.find(')')?;
        &rest[close + 1..]
    } else {
        signature
    };
    let descriptor = descriptor.trim_start_matches('[');
    let object = descriptor.strip_prefix('L')?;
    let slashed = object.strip_suffix(';').unwrap_or(object);
    (!slashed.is_empty()).then(|| slashed.replace('/', "."))
}

#[cfg(test)]
mod tests {
    use super::return_type;

    #[test]
    fn decodes_method_and_field_descriptors() {
        assert_eq!(
            return_type("(Ljava/lang/String;)Lcom/example/Result;"),
            Some("com.example.Result".into())
        );
        assert_eq!(
            return_type("[Lcom/example/Item;"),
            Some("com.example.Item".into())
        );
        assert_eq!(return_type("I"), None);
    }
}
