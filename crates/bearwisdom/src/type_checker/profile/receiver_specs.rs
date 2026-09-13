// =============================================================================
// type_checker/profile/receiver_specs.rs — merge reach and receiver spellings
//
// A receiver spelling is source grammar; resolver rules consume only the
// language-neutral `ReceiverRole` it selects. `language_profile.rs` re-exports
// this module, so every profile literal keeps one import path.
// =============================================================================

/// How far apart two same-qname type declarations may sit and still be ONE
/// logical type (Roslyn-style declaration merging).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeScope {
    /// No declaration merging — same-qname declarations are distinct types.
    None,
    /// Merge only within one file (TS `interface Foo` + `namespace Foo`;
    /// module scoping makes same-name declarations in other files distinct).
    SameFile,
    /// Merge across files within one package (C# partial classes — qnames
    /// are namespace-qualified, so one qname is one type per package).
    SamePackage,
}

/// The semantic declaration selected by a receiver spelling. Resolver rules
/// consume only this language-neutral meaning; source spellings and member
/// separators live in each language profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverRole {
    /// The type enclosing the reference's source symbol.
    EnclosingType,
    /// The enclosing type's recorded direct parent.
    DirectParent,
}

/// One source spelling that is meaningful at a receiver position.
///
/// `role: None` permits a language-owned qualification prefix such as HCL's
/// `var.name` to normalize for bare-name rules without granting it receiver
/// semantics. The separator is source syntax, never a generic resolver
/// assumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiverSpelling {
    pub spelling: &'static str,
    pub role: Option<ReceiverRole>,
    pub member_separator: &'static str,
}

impl ReceiverSpelling {
    pub const fn enclosing(spelling: &'static str, member_separator: &'static str) -> Self {
        Self {
            spelling,
            role: Some(ReceiverRole::EnclosingType),
            member_separator,
        }
    }

    pub const fn parent(spelling: &'static str, member_separator: &'static str) -> Self {
        Self {
            spelling,
            role: Some(ReceiverRole::DirectParent),
            member_separator,
        }
    }

    pub const fn prefix(spelling: &'static str, member_separator: &'static str) -> Self {
        Self {
            spelling,
            role: None,
            member_separator,
        }
    }
}

impl PartialEq<&str> for ReceiverSpelling {
    fn eq(&self, other: &&str) -> bool {
        self.spelling == *other
    }
}
