// =============================================================================
// parser/extractors/cpp.rs  —  C++ extractor stub
//
// Delegates to c_lang for now; a dedicated C++ extractor can be added later.
// =============================================================================

pub fn extract(source: &str) -> super::ExtractionResult {
    super::c_lang::extract(source, "cpp")
}
