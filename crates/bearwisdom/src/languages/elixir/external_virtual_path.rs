//! Mix dependency and OTP standard-library virtual identities.

pub(crate) fn for_pulled(path: &str) -> Option<String> {
    if let Some(index) = path.rfind("/deps/") {
        if let Some((package, relative)) = path[index + "/deps/".len()..].split_once('/') {
            if !package.is_empty() && !relative.is_empty() {
                return Some(format!("ext:elixir:{package}/{relative}"));
            }
        }
    }
    let mut hit = None;
    let mut search = 0;
    while let Some(index) = path[search..].find("/lib/") {
        let start = search + index + "/lib/".len();
        let Some((app, rest)) = path[start..].split_once('/') else {
            break;
        };
        if let Some(relative) = rest.strip_prefix("lib/") {
            if !app.is_empty() && !relative.is_empty() {
                hit = Some(format!("ext:elixir-stdlib:{app}/{relative}"));
            }
        }
        search = start;
    }
    hit
}
