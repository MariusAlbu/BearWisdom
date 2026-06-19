// =============================================================================
// type_checker/profile — declarative per-language behavior
//
// LanguageProfile describes every per-language axis the engine inspects at
// resolution time.
// =============================================================================

pub mod language_profile;
pub mod registry;

pub use language_profile::{
    ArgKey, BucketContainer, ClassBuilderSpec, ClassNameSource, ConstructorPattern,
    DecoratorSyntax, DispatchAxis, KindCompatibility, KindTable, LanguageProfile, MemberShape,
    MethodBucket, SupertypeDiscovery, DEFAULT_PROFILE, PERMISSIVE_KIND_TABLE,
};
pub use registry::ProfileRegistry;
