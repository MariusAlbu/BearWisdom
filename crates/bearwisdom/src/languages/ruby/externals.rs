// =============================================================================
// ruby/externals.rs — Ruby runtime globals and framework-injected names
// =============================================================================

use std::collections::HashSet;

/// Ruby runtime globals that are always external.
///
/// These identifiers appear in Ruby code but are never defined in project
/// source — they are kernel-level, stdlib-level, or interpreter globals.
pub(crate) const EXTERNALS: &[&str] = &[
    // Special variables / pseudo-globals
    "__method__",
    "__dir__",
    "__callee__",
    // Kernel-injected globals
    "$stdout",
    "$stderr",
    "$stdin",
    "$0",
    "$PROGRAM_NAME",
    "$LOAD_PATH",
    "$LOADED_FEATURES",
    "$:",
    "$\"",
];

/// Dependency-gated framework globals for Ruby.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    // RSpec — test framework globals
    if deps.contains("rspec") || deps.contains("rspec-core") || deps.contains("rspec-rails") {
        globals.extend(RSPEC_GLOBALS);
    }

    // Rails / ActiveSupport
    if deps.contains("rails") || deps.contains("activerecord") || deps.contains("activesupport") {
        globals.extend(RAILS_GLOBALS);
    }

    // FactoryBot / FactoryGirl
    if deps.contains("factory_bot") || deps.contains("factory_bot_rails") || deps.contains("factory_girl") {
        globals.extend(&["create", "build", "build_stubbed", "attributes_for", "create_list", "build_list"]);
    }

    // Devise authentication helpers
    if deps.contains("devise") {
        globals.extend(&["sign_in", "sign_out", "current_user", "user_signed_in?", "authenticate_user!"]);
    }

    // Sidekiq background jobs
    if deps.contains("sidekiq") {
        globals.extend(&["perform_async", "perform_in", "perform_at", "set"]);
    }

    globals
}

const RSPEC_GLOBALS: &[&str] = &[
    "describe",
    "fdescribe",
    "xdescribe",
    "it",
    "fit",
    "xit",
    "specify",
    "fspecify",
    "xspecify",
    "context",
    "before",
    "after",
    "around",
    "let",
    "let!",
    "subject",
    "shared_examples",
    "shared_context",
    "include_examples",
    "include_context",
    "shared_examples_for",
    "expect",
    "allow",
    "receive",
    "have_received",
    "instance_double",
    "class_double",
    "object_double",
    "instance_spy",
    "class_spy",
    "double",
    "spy",
    "stub_const",
    "aggregate_failures",
    "pending",
    "skip",
];

const RAILS_GLOBALS: &[&str] = &[
    // ActionController test helpers
    "get",
    "post",
    "put",
    "patch",
    "delete",
    "head",
    "response",
    "request",
    "assigns",
    "flash",
    "session",
    "cookies",
    // Rails routing helpers (generated at runtime)
    "root_path",
    "root_url",
    // Capybara integration helpers
    "visit",
    "fill_in",
    "click_button",
    "click_link",
    "have_content",
    "have_selector",
    "within",
    "page",
];
