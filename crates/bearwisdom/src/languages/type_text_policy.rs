//! What a language admits about its own type text.
//!
//! The parser beside this module reads spellings; this module declares WHICH
//! spellings a plugin owns. Keeping the declaration separate is what lets a
//! plugin opt into one surface form without inheriting another language's.

use crate::type_checker::core::types::Intrinsic;

/// Explicit source-syntax features a language elects to parse.
///
/// The default is deliberately opaque.  A plugin must opt into every surface
/// form it owns; this prevents a type annotation from one language being
/// interpreted according to another language's grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeTextPolicy {
    pub reference_sigil: bool,
    pub pointer_sigil: bool,
    pub opaque_existential_prefix: bool,
    pub python_callable: bool,
    pub dart_function: bool,
    pub go_function: bool,
    pub fat_arrow_function: bool,
    pub thin_arrow_function: bool,
    pub bare_arrow_parameter: bool,
    pub readonly_modifier: bool,
    pub nullable_prefix: bool,
    pub nullable_suffix: bool,
    pub array_suffix: bool,
    pub union_intersection: bool,
    pub parenthesized_tuple: bool,
    pub bracket_tuple: bool,
    pub bracket_array: bool,
    pub rust_array_or_slice: bool,
    pub angle_application: bool,
    pub bracket_application: bool,
    pub lifetime_arguments: bool,
    /// Whole spellings this language uses for its atomic types, paired with the
    /// semantic atom each denotes. A spelling listed here interns as that atom
    /// instead of as a nominal named after the spelling — the arena stores
    /// `Intrinsic::Null`, not a class called `null` that no declaration answers
    /// to. Matched against the complete trimmed text, so an identifier that
    /// merely starts with one is untouched.
    pub atoms: &'static [(&'static str, Intrinsic)],
}

impl TypeTextPolicy {
    /// Accept no source grammar; preserve the complete spelling as an opaque
    /// nominal. This is the `LanguagePlugin` default.
    pub const OPAQUE: Self = Self {
        reference_sigil: false,
        pointer_sigil: false,
        opaque_existential_prefix: false,
        python_callable: false,
        dart_function: false,
        go_function: false,
        fat_arrow_function: false,
        thin_arrow_function: false,
        bare_arrow_parameter: false,
        readonly_modifier: false,
        nullable_prefix: false,
        nullable_suffix: false,
        array_suffix: false,
        union_intersection: false,
        parenthesized_tuple: false,
        bracket_tuple: false,
        bracket_array: false,
        rust_array_or_slice: false,
        angle_application: false,
        bracket_application: false,
        lifetime_arguments: false,
        atoms: &[],
    };

    /// Compatibility policy for a plugin that explicitly owns every source
    /// spelling supported by the former generic parser. It is intentionally
    /// not the default. It names no atoms: an atom is a spelling, and spellings
    /// belong to one language rather than to a shared syntax set.
    pub const ALL_LEGACY_FORMS: Self = Self {
        reference_sigil: true,
        pointer_sigil: true,
        opaque_existential_prefix: true,
        python_callable: true,
        dart_function: true,
        go_function: true,
        fat_arrow_function: true,
        thin_arrow_function: true,
        bare_arrow_parameter: true,
        readonly_modifier: true,
        nullable_prefix: true,
        nullable_suffix: true,
        array_suffix: true,
        union_intersection: true,
        parenthesized_tuple: true,
        bracket_tuple: true,
        bracket_array: true,
        rust_array_or_slice: true,
        angle_application: true,
        bracket_application: true,
        lifetime_arguments: true,
        atoms: &[],
    };

    /// The semantic atom `text` denotes, when this language spells one that way.
    pub(crate) fn atom(&self, text: &str) -> Option<Intrinsic> {
        self.atoms
            .iter()
            .find(|(spelling, _)| *spelling == text)
            .map(|(_, atom)| *atom)
    }
}
