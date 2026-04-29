use super::*;

fn resolve(spec: &str, from: &str, files: &[&str]) -> Option<String> {
    RubyModuleResolver.resolve_to_file(spec, from, files)
}

#[test]
fn require_relative_sibling() {
    let files = &["test/helper.rb", "test/retry_test.rb"];
    assert_eq!(
        resolve("./helper", "test/retry_test.rb", files),
        Some("test/helper.rb".into())
    );
}

#[test]
fn require_relative_parent() {
    let files = &["models/user.rb", "controllers/users_controller.rb"];
    assert_eq!(
        resolve("../models/user", "controllers/users_controller.rb", files),
        Some("models/user.rb".into())
    );
}

#[test]
fn bare_require_under_lib() {
    let files = &["lib/sidekiq/api.rb", "lib/sidekiq.rb"];
    assert_eq!(
        resolve("sidekiq/api", "test/api_test.rb", files),
        Some("lib/sidekiq/api.rb".into())
    );
}

#[test]
fn bare_require_falls_back_to_root() {
    let files = &["loader.rb"];
    assert_eq!(
        resolve("loader", "main.rb", files),
        Some("loader.rb".into())
    );
}

#[test]
fn unknown_returns_none() {
    let files: &[&str] = &["lib/sidekiq.rb"];
    assert!(resolve("nonexistent/gem", "test/foo.rb", files).is_none());
}

#[test]
fn relative_outside_root_returns_none() {
    let files: &[&str] = &["test/helper.rb"];
    assert!(resolve("../../outside", "test/foo.rb", files).is_none());
}
