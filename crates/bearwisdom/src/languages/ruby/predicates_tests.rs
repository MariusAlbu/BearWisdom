use super::predicates;

#[test]
fn rails_active_record_methods_not_classified_as_ruby_builtin() {
    // Rails / ActiveRecord / ActiveSupport / RSpec leakage previously
    // short-circuited resolution by classifying these as `ruby_core`,
    // hiding them from the gem-manifest path. The RubyGems externals
    // walker (`ecosystem/rubygems.rs`) handles them when the project's
    // Gemfile declares the corresponding gems.
    for name in &[
        // ActiveSupport convenience
        "present?", "blank?", "presence", "try", "in?",
        // Rails / ActiveRecord framework constants
        "ActiveRecord", "ActiveSupport", "ActionController",
        "ApplicationRecord", "Rails", "RSpec",
        // ActiveRecord DSL
        "belongs_to", "has_many", "has_one",
        "validates", "validates_presence_of", "validates_uniqueness_of",
        "before_action", "after_action", "before_save", "after_save",
        "scope", "default_scope",
        // ActiveRecord query methods (collide with common verbs)
        "where", "find_by", "create", "update", "destroy", "save",
        "transaction", "joins", "order", "limit", "pluck",
        // RSpec DSL + matchers (collide with common names)
        "context", "before", "after", "let", "subject",
        "expect", "eq", "match", "change", "double", "mock",
        "be_valid", "be_persisted",
    ] {
        assert!(
            !predicates::is_ruby_builtin(name),
            "{name:?} should not be classified as a ruby builtin",
        );
    }
}

#[test]
fn real_ruby_builtins_still_classified() {
    // Sanity: actual Ruby stdlib / Kernel methods + core constants still
    // match.
    for name in &[
        // Kernel
        "puts", "print", "raise", "require", "lambda",
        // Object
        "nil?", "is_a?", "respond_to?", "send", "to_s", "to_i",
        // Enumerable / Array / Hash / String
        "each", "map", "select", "push", "pop", "strip", "split",
        "keys", "values", "fetch",
        // Core constants / classes
        "Array", "Hash", "Object", "Kernel",
        "NilClass", "Exception", "StandardError",
        "IO", "File", "Dir",
        "Math", "Thread", "Fiber", "GC",
        "STDOUT", "STDERR", "ENV", "ARGV",
    ] {
        assert!(
            predicates::is_ruby_builtin(name),
            "{name:?} must remain a ruby builtin",
        );
    }
}
