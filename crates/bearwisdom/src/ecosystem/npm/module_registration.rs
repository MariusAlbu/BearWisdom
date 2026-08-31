// =============================================================================
// npm/module_registration.rs — module-entry key registration.
//
// Registers every specifier → entry-file key the demand pass can materialize:
// package `.` entries, published subpath entries, and ambient
// `declare module '<name>'` names. Ordering is load-bearing: ambient names
// register last so an augmentation naming a real package never claims that
// package's keys — first writer wins on both axes.
// =============================================================================

use std::collections::HashMap;
use std::path::PathBuf;

use crate::ecosystem::SymbolLocationIndex;

use super::ts_scan::FileExports;

/// Insert module-entry keys for package entries, subpath entries, and ambient
/// declared modules into `index`.
pub(super) fn register_module_entries(
    index: &mut SymbolLocationIndex,
    pkg_entry: &HashMap<String, PathBuf>,
    subpath_entry: &HashMap<String, PathBuf>,
    scanned: &[(String, PathBuf, FileExports)],
) {
    // Expose each package's `.` entry. A barrel package re-exports its names from
    // other packages (`vue` → `@vue/runtime-dom` → @vue/runtime-core), so the leaf
    // is qnamed under the DEFINING package, not the imported one. Materializing the
    // package entry brings in its `export *` chain (whose re-export refs carry the
    // source module, so the demand pass pulls each hop) and lets re-export-following
    // bind `import { computed } from 'vue'`.
    for (module, entry) in pkg_entry {
        index.insert_module_entry(module.clone(), entry.clone());
    }
    // Subpath entries (`pkg/sub` published in `exports`): key each full deep
    // specifier to its own entry file, so a ref tagged with the deep module
    // materializes the subpath's declaration entry rather than only the
    // package's `.` barrel.
    for (module, entry) in subpath_entry {
        index.insert_module_entry(module.clone(), entry.clone());
    }
    // Ambient `declare module '<name>'` blocks: the declared literal is its
    // own module key — that is the specifier users import, not the declaring
    // package's name. Inner names locate under the declared key; the module
    // entry lets demand materialize the declaring file.
    for (_, file, exports) in scanned {
        for (declared, inner_names) in &exports.ambient_modules {
            for inner in inner_names {
                index.insert(declared, inner.clone(), file.clone());
            }
            index.insert_module_entry(declared.clone(), file.clone());
        }
    }
}
