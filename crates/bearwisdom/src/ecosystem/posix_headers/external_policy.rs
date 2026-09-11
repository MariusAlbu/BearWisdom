/// Classify extensionless standard-library headers owned by this provider.
pub(super) fn extensionless_source_language(file_name: &str) -> Option<&'static str> {
    crate::ecosystem::posix_headers::is_extensionless_cpp_stdlib_header(file_name).then_some("cpp")
}
