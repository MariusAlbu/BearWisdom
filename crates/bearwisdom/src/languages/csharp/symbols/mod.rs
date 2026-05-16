// =============================================================================
// csharp/symbols/mod.rs  —  facade for symbol-pusher submodules
//
// One submodule per declaration-kind cluster. Re-exports `pub(super)` so the
// rest of the csharp plugin (extract.rs, calls_symbols.rs) keeps calling the
// pushers as `symbols::push_*` exactly as before.
// =============================================================================

mod members;
mod specials;
mod types_decls;

pub(super) use members::*;
pub(super) use specials::*;
pub(super) use types_decls::*;
