// =============================================================================
// engine/module_identity — cross-language symbol container identity
//
// A symbol's resolution identity is `(ModuleId, qname)`, not the bare qname, so
// same-named module-level symbols in different modules (monorepo packages,
// files, assemblies) don't collide. This module owns the source-symbol
// derivation seam: given a language's container granularity, the discriminator
// that pairs with the qname. External/metadata containers (a .NET assembly, a
// jar) are stamped by the ecosystem walker, not derived here.
//
// See MODULE-IDENTITY.md.
// =============================================================================

/// How a language roots its module-level symbols — the container that
/// disambiguates same-named symbols across modules.
///
/// `File`/`Package`-class rules are pure (derivable from the symbol's file);
/// `Namespace`/`Crate`/`Assembly` need the hook or the ecosystem walker and are
/// not derived here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleGranularity {
    /// One container for the whole project — same-named module-level symbols
    /// still share a key. The conservative default: resolution keys exactly as
    /// it did before module identity (every symbol shares the empty
    /// discriminator), so flipping a language off this rung is opt-in.
    Project,
    /// The module is the source file — ES-module / Python-module semantics
    /// (`import { x } from './m'` binds to a specific file).
    File,
}

/// The container discriminator for a source symbol declared in `file_path`,
/// under `granularity`. Paired with the symbol's qname to form its resolution
/// identity `(discriminator, qname)`.
///
/// `Project` yields the empty discriminator so every symbol shares one container
/// — byte-identical to pre-module-identity keying. `File` yields the declaring
/// file path, so two files exporting the same name resolve as distinct symbols.
pub fn module_discriminator(granularity: ModuleGranularity, file_path: &str) -> &str {
    match granularity {
        ModuleGranularity::Project => "",
        ModuleGranularity::File => file_path,
    }
}

#[cfg(test)]
#[path = "module_identity_tests.rs"]
mod tests;
