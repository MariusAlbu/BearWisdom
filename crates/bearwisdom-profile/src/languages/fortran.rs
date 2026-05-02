use crate::types::*;

pub static FORTRAN: LanguageDescriptor = LanguageDescriptor {
    id: "fortran",
    display_name: "Fortran",
    // .fypp is a Fortran preprocessor (preceeds compilation, like CPP for C).
    // Files contain Fortran source with `#:if` / `#:for` / `${...}$`
    // directives. The non-directive parts parse as valid Fortran and the
    // tree-sitter grammar emits the recognisable constructs (`module`,
    // `interface`, `public` declarations, etc.). Stdlib-shape projects
    // (fortran-stdlib, datetime-fortran) keep most of their public API
    // surface in `.fypp`; without indexing these files every test that
    // calls `var`/`moment`/`corr`/etc. lands in unresolved_refs.
    file_extensions: &[".f90", ".f95", ".f03", ".f08", ".fypp"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("!"),
    block_comment: None,
};
