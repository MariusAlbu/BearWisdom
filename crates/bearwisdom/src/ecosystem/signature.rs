//! Ecosystem-owned signature evidence adapters.
//!
//! The resolver supplies a source language and an opaque stored signature. This
//! registry selects only the ecosystem decoder that owns that language's
//! signature grammar, so generic resolution never interprets a foreign
//! descriptor spelling.

type ReturnTypeParser = fn(&str) -> Option<String>;

struct Adapter {
    language_ids: &'static [&'static str],
    return_type: ReturnTypeParser,
}

const ADAPTERS: &[Adapter] = &[
    Adapter {
        language_ids: &["java", "kotlin", "scala", "groovy", "clojure"],
        return_type: super::maven::signature::return_type,
    },
    Adapter {
        language_ids: &["csharp", "fsharp", "vbnet", "powershell"],
        return_type: super::nuget::signature::return_type,
    },
];

/// Decode ecosystem-owned return-type evidence for `language`.
pub(crate) fn return_type_for_language(language: &str, signature: &str) -> Option<String> {
    ADAPTERS
        .iter()
        .filter(|adapter| adapter.language_ids.contains(&language))
        .find_map(|adapter| (adapter.return_type)(signature))
}

#[cfg(test)]
mod tests {
    use super::return_type_for_language;

    #[test]
    fn decodes_descriptors_only_for_the_owning_ecosystem_languages() {
        let descriptor = "(Ljava/lang/String;)Lcom/example/Result;";
        assert_eq!(
            return_type_for_language("java", descriptor),
            Some("com.example.Result".into())
        );
        assert_eq!(return_type_for_language("typescript", descriptor), None);
    }

    #[test]
    fn decodes_the_cracked_assembly_shape_only_for_clr_languages() {
        let display = "Greeter(string): FakeExt.Greeter";
        assert_eq!(
            return_type_for_language("csharp", display),
            Some("FakeExt.Greeter".into())
        );
        assert_eq!(
            return_type_for_language("fsharp", display),
            Some("FakeExt.Greeter".into())
        );
        assert_eq!(return_type_for_language("java", display), None);
        assert_eq!(return_type_for_language("typescript", display), None);
    }
}
