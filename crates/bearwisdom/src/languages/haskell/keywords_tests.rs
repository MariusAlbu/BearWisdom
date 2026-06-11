use super::keywords::KEYWORDS;

/// The rule-legal subset is Prelude functions/constructors/primitive types and
/// operators only. Library types and typeclasses (text/containers/aeson/mtl/
/// vector) must NOT appear — they stay honest-unresolved/external.
#[test]
fn prelude_and_operators_decline_library_types_do_not() {
    // Prelude functions and operators decline as primitives.
    for name in ["map", "fmap", "$", "return"] {
        assert!(
            KEYWORDS.contains(&name),
            "`{name}` is a Prelude function/operator and must decline as a primitive"
        );
    }

    // Library types/typeclasses are NOT language primitives — they must resolve
    // to indexed externals, not be declined.
    for name in ["Text", "Map", "ToJSON"] {
        assert!(
            !KEYWORDS.contains(&name),
            "`{name}` is library API (text/containers/aeson); it must NOT be a primitive"
        );
    }
}
