//! Flutter, pub-cache, and Dart SDK virtual identities.

pub(crate) fn for_pulled(path: &str) -> Option<String> {
    if let Some(index) = path.rfind("/packages/") {
        if let Some((package, rest)) = path[index + "/packages/".len()..].split_once('/') {
            if let Some(relative) = rest.strip_prefix("lib/") {
                if !package.is_empty() && !relative.is_empty() {
                    return Some(format!("ext:flutter-sdk:{package}/{relative}"));
                }
            }
        }
    }
    if let Some(index) = path.rfind("/hosted/pub.dev/") {
        if let Some((directory, rest)) = path[index + "/hosted/pub.dev/".len()..].split_once('/') {
            if let Some(relative) = rest.strip_prefix("lib/") {
                if let Some((package, _)) = crate::ecosystem::cargo::split_crate_dir_name(directory)
                {
                    if !relative.is_empty() {
                        return Some(format!("ext:dart:{package}/{relative}"));
                    }
                }
            }
        }
    }
    let mut search = 0;
    while let Some(index) = path[search..].find("/lib/") {
        let start = search + index + "/lib/".len();
        if let Some((library, relative)) = path[start..].split_once('/') {
            if crate::ecosystem::dart_sdk::DART_SDK_LIBS.contains(&library) && !relative.is_empty()
            {
                return Some(format!("ext:dart-sdk:{library}/{relative}"));
            }
        }
        search = start;
    }
    None
}
