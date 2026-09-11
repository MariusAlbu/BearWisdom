use super::*;

#[test]
fn return_type_is_read_after_the_parameter_list() {
    assert_eq!(
        PhpPlugin.signature_return_type("function getMockBuilder(string $className): MockBuilder"),
        Some("MockBuilder".to_string())
    );
    assert_eq!(
        PhpPlugin.signature_return_type("function default(bool $value): static"),
        Some("static".to_string())
    );
    assert_eq!(
        PhpPlugin.signature_return_type(
            "function convertFileInformation(array|UploadedFile $file): array|UploadedFile|null"
        ),
        Some("array|UploadedFile|null".to_string())
    );
}

#[test]
fn return_type_ignores_colons_inside_parameter_attributes_and_defaults() {
    let signature = "function lookup(\n        array $languageTag,\n        #[LanguageAware(['8.0' => 'string'], default: '')] $locale,\n        #[ElementAvailable(from: '7.0')] $canonicalize = false\n    ): ?string";
    assert_eq!(
        PhpPlugin.signature_return_type(signature),
        Some("?string".to_string())
    );
}

#[test]
fn signature_without_result_type_yields_no_return_type() {
    assert_eq!(PhpPlugin.signature_return_type("function messages()"), None);
    assert_eq!(
        PhpPlugin.signature_return_type("function setLenient(#[LanguageAware(['8.0' => 'bool'], default: '')] $lenient)"),
        None
    );
}

#[test]
fn global_and_static_variable_signatures_carry_no_declared_type() {
    assert_eq!(PhpPlugin.signature_declared_type("static $cache"), None);
    assert_eq!(PhpPlugin.signature_declared_type("global $config"), None);
}
