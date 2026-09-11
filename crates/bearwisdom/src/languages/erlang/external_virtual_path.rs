//! OTP application virtual identities.

pub(crate) fn for_pulled(path: &str) -> Option<String> {
    let index = path.rfind("/lib/")?;
    let after = &path[index + "/lib/".len()..];
    let app_version = after.split('/').next()?;
    let app = app_version.split('-').next()?;
    let source = after.find("/src/")?;
    let relative = &after[source + "/src/".len()..];
    Some(format!("ext:erlang:{app}/{relative}"))
}
