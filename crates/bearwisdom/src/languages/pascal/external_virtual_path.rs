//! Free Pascal and Lazarus virtual identities.

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

pub(crate) fn for_pulled(path: &str) -> Option<String> {
    if let Some(index) = path.rfind("/rtl/inc/") {
        let relative = &path[index + "/rtl/inc/".len()..];
        if relative.is_empty() {
            return None;
        }
        let lower = relative.to_ascii_lowercase();
        let system =
            lower.ends_with(".inc") && !NON_SYSTEM_RTL_INC_FRAGMENTS.contains(&lower.as_str());
        return Some(if system {
            format!("ext:fpc-stdlib:system/{relative}")
        } else {
            format!("ext:fpc:fpc-rtl-inc/{relative}")
        });
    }
    if let Some(name) = path.rsplit('/').next() {
        if name.eq_ignore_ascii_case("system.pp") || name.eq_ignore_ascii_case("system.pas") {
            return Some(format!("ext:fpc-stdlib:system/{name}"));
        }
    }
    if let Some(index) = path.rfind("/lcl/") {
        let relative = &path[index + "/lcl/".len()..];
        if !relative.is_empty() {
            return Some(format!("ext:fpc:lcl/{relative}"));
        }
    }
    if let Some(index) = path.rfind("/components/") {
        let relative = &path[index + "/components/".len()..];
        if !relative.is_empty() {
            return Some(format!("ext:fpc:lazarus-components/{relative}"));
        }
    }
    if let Some(index) = path.rfind("/rtl/objpas/") {
        let relative = &path[index + "/rtl/objpas/".len()..];
        if !relative.is_empty() {
            return Some(format!("ext:fpc:fpc-rtl-objpas/{relative}"));
        }
    }
    if let Some(index) = path.rfind("/source/packages/") {
        if let Some((package, rest)) = path[index + "/source/packages/".len()..].split_once('/') {
            if let Some(relative) = rest.strip_prefix("src/") {
                if !package.is_empty() && !relative.is_empty() {
                    return Some(format!("ext:fpc:fpc-pkg-{package}/{relative}"));
                }
            }
        }
    }
    if let Some(index) = path.rfind("/source/rtl/") {
        let (target, relative) = path[index + "/source/rtl/".len()..].split_once('/')?;
        if !target.is_empty() && !relative.is_empty() {
            return Some(format!("ext:fpc:fpc-rtl-{target}/{relative}"));
        }
    }
    None
}
