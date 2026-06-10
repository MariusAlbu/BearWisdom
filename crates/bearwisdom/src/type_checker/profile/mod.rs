// =============================================================================
// type_checker/profile — declarative per-language behavior
//
// LanguageProfile describes every per-language axis the engine inspects at
// resolution time. LanguageEngineHooks is the escape hatch for behavior that
// can't be expressed as data (preprocessing refs, synthesizing members from
// decorators, enriching external types, special dispatch, custom flow
// emission).
// =============================================================================

pub mod hooks;
pub mod language_profile;
pub mod registry;

pub use hooks::{DispatchContext, LanguageEngineHooks, NoOpHooks, RefContext};
pub use language_profile::{
    ArgKey, BucketContainer, ClassBuilderSpec, ClassNameSource, ConstructorPattern,
    DecoratorSyntax, DispatchAxis, KindCompatibility, KindTable, LanguageProfile, MemberShape,
    MethodBucket, SupertypeDiscovery, DEFAULT_PROFILE, PERMISSIVE_KIND_TABLE,
};
pub use registry::ProfileRegistry;
