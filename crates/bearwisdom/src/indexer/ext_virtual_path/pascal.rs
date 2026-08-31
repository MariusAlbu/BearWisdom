// =============================================================================
// indexer/ext_virtual_path/pascal — FPC/Lazarus virtual paths for pulled files
// =============================================================================

use std::path::Path;

/// FPC/Lazarus fragments spliced into `rtl/inc/` under a unit OTHER than
/// System, so they keep the non-ambient shape even though they sit in the
/// same directory as System's own fragments: `Dos` (`dos.inc`/`dosh.inc`),
/// `Strings` (`genstr.inc`/`genstrs.inc`/`stringsi.inc`), `SysUtils`
/// (`fexpand.inc`/`varerror.inc`), `Types`/Windows struct decls
/// (`typshrd.inc`/`typshrdh.inc`), `ObjC` (`objc1.inc`/`objcnf.inc`),
/// `FpExtRes`/`FpIntRes` (`extres.inc`/`intres.inc`) — confirmed by tracing
/// `{$I}` splice targets from every platform's `system.pp`/`system.pas`
/// against FPC 3.2.2's `rtl/inc/` fragment set. `makefile.inc` is FPC's own
/// build-system fragment list, not Pascal source.
const NON_SYSTEM_RTL_INC_FRAGMENTS: &[&str] = &[
    "dos.inc",
    "dosh.inc",
    "fexpand.inc",
    "extres.inc",
    "intres.inc",
    "genstr.inc",
    "genstrs.inc",
    "stringsi.inc",
    "objc1.inc",
    "objcnf.inc",
    "typshrd.inc",
    "typshrdh.inc",
    "varerror.inc",
    "makefile.inc",
];

/// Free Pascal RTL / Lazarus layout, discovered eagerly by
/// `freepascal_runtime.rs` but never walked (`uses_demand_driven_parse`) —
/// every pascal external file reaches the index through this function.
/// Layout: `<lazarus>/lcl/`, `<lazarus>/components/`,
/// `<lazarus>/fpc/<ver>/source/rtl/<target|inc|objpas>/`,
/// `<lazarus>/fpc/<ver>/source/packages/<pkg>/src/`.
///
/// System-unit membership is path-derived: `system.pp`/`system.pas` itself,
/// plus every `.inc` fragment directly under `rtl/inc/` EXCEPT the
/// confirmed non-System fragments in `NON_SYSTEM_RTL_INC_FRAGMENTS`. Every
/// `.pp`/`.pas` file under `rtl/inc/` (Strings, CTypes, CMem, GetOpts,
/// HeapTrc, LineInfo, ...) declares its own `unit` header and is never
/// spliced, so the extension split excludes those too. The `fpc-stdlib`
/// ecosystem segment is what `is_ambient_global_lib_path` classifies as
/// ambient scope: System is implicit in every Pascal file — no
/// `uses System;` ever appears — so its declarations (`TObject`,
/// `Exception`, ...) must resolve without a `uses`-clause wildcard rung to
/// open them.
pub(super) fn virtual_path_for_pulled_pascal(s: &str, abs: &Path) -> Option<String> {
    if let Some(inc_idx) = s.rfind("/rtl/inc/") {
        let rel = &s[inc_idx + "/rtl/inc/".len()..];
        if rel.is_empty() {
            return None;
        }
        let rel_lower = rel.to_ascii_lowercase();
        let is_system_fragment = rel_lower.ends_with(".inc")
            && !NON_SYSTEM_RTL_INC_FRAGMENTS.contains(&rel_lower.as_str());
        return Some(if is_system_fragment {
            format!("ext:fpc-stdlib:system/{rel}")
        } else {
            format!("ext:fpc:fpc-rtl-inc/{rel}")
        });
    }
    if let Some(name) = abs.file_name().and_then(|n| n.to_str()) {
        if name.eq_ignore_ascii_case("system.pp") || name.eq_ignore_ascii_case("system.pas") {
            return Some(format!("ext:fpc-stdlib:system/{name}"));
        }
    }
    if let Some(i) = s.rfind("/lcl/") {
        let rel = &s[i + "/lcl/".len()..];
        if !rel.is_empty() {
            return Some(format!("ext:fpc:lcl/{rel}"));
        }
    }
    if let Some(i) = s.rfind("/components/") {
        let rel = &s[i + "/components/".len()..];
        if !rel.is_empty() {
            return Some(format!("ext:fpc:lazarus-components/{rel}"));
        }
    }
    if let Some(i) = s.rfind("/rtl/objpas/") {
        let rel = &s[i + "/rtl/objpas/".len()..];
        if !rel.is_empty() {
            return Some(format!("ext:fpc:fpc-rtl-objpas/{rel}"));
        }
    }
    if let Some(i) = s.rfind("/source/packages/") {
        let after = &s[i + "/source/packages/".len()..];
        if let Some((pkg, rest)) = after.split_once('/') {
            if let Some(rel) = rest.strip_prefix("src/") {
                if !pkg.is_empty() && !rel.is_empty() {
                    return Some(format!("ext:fpc:fpc-pkg-{pkg}/{rel}"));
                }
            }
        }
    }
    if let Some(i) = s.rfind("/source/rtl/") {
        let after = &s[i + "/source/rtl/".len()..];
        if let Some((target, rel)) = after.split_once('/') {
            if !target.is_empty() && !rel.is_empty() {
                return Some(format!("ext:fpc:fpc-rtl-{target}/{rel}"));
            }
        }
    }
    None
}
