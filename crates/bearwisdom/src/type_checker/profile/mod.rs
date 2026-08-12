// =============================================================================
// type_checker/profile — declarative per-language behavior
//
// LanguageProfile describes every per-language axis the engine inspects at
// resolution time.
// =============================================================================

pub mod chain_specs;
pub mod default_profile;
pub mod import_axes;
pub mod import_specs;
pub mod language_profile;
pub mod registry;
pub mod syntax_specs;

pub use language_profile::{
    ArgKey, BucketContainer, ClassBuilderSpec, ClassNameSource, ConstructorPattern,
    DecoratorSyntax, DispatchAxis, ImportAxes, KindCompatibility, KindTable, LanguageProfile,
    MemberShape, MethodBucket, SupertypeDiscovery, DEFAULT_PROFILE, PERMISSIVE_KIND_TABLE,
};
pub use registry::ProfileRegistry;
