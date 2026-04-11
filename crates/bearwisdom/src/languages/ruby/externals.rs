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
    // Ruby stdlib type constants — require'd from stdlib but used as bare names
    "SecureRandom",
    "JSON",
    "URI",
    "Pathname",
    "FileUtils",
    "Tempfile",
    "StringIO",
    "OpenStruct",
    "Set",
    "Mutex",
    "Process",
    "BigDecimal",
    "Logger",
    "Date",
    "DateTime",
    "Time",
    "Regexp",
    // Additional stdlib constants used without explicit require in Rails apps
    "Base64",
    "Digest",
    "CSV",
    "YAML",
    "ERB",
    "CGI",
    "Net",
    "Socket",
    "Encoding",
    "Math",
    "Comparable",
];

/// Dependency-gated framework globals for Ruby.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    // Minitest — stdlib test framework. Often appears in Gemfile as "minitest"
    // or "minitest-rails". Also used directly without a gem declaration in
    // projects that rely on bundled minitest from Ruby stdlib.
    if deps.contains("minitest") || deps.contains("minitest-rails") || deps.contains("minitest-spec-rails") {
        globals.extend(MINITEST_GLOBALS);
    } else {
        // Minitest is part of Ruby stdlib — include the most common assertions
        // unconditionally so they are recognised even when not listed in Gemfile.
        globals.extend(MINITEST_CORE);
    }

    // RSpec — test framework globals
    if deps.contains("rspec") || deps.contains("rspec-core") || deps.contains("rspec-rails") {
        globals.extend(RSPEC_GLOBALS);
    }

    // Rails / ActiveRecord / ActiveSupport
    if deps.contains("rails")
        || deps.contains("activerecord")
        || deps.contains("activesupport")
        || deps.contains("actionpack")
    {
        globals.extend(RAILS_GLOBALS);
    }

    // FactoryBot / FactoryGirl
    if deps.contains("factory_bot")
        || deps.contains("factory_bot_rails")
        || deps.contains("factory_girl")
        || deps.contains("factory_girl_rails")
    {
        globals.extend(&[
            "create",
            "build",
            "build_stubbed",
            "attributes_for",
            "create_list",
            "build_list",
            "build_stubbed_list",
            "create_pair",
            "build_pair",
        ]);
    }

    // Devise authentication helpers
    if deps.contains("devise") {
        globals.extend(&[
            "sign_in",
            "sign_out",
            "current_user",
            "user_signed_in?",
            "authenticate_user!",
            "require_no_authentication",
            "devise_parameter_sanitizer",
            // Namespace classes referenced as base classes in User models
            // and Devise-extending controllers.
            "Devise",
            "Devise::Controllers::Helpers",
            "Devise::Test::ControllerHelpers",
            "Devise::Test::IntegrationHelpers",
            "Devise::OmniAuth::AuthCallbacksController",
            "DeviseController",
        ]);
    }

    // Sidekiq background jobs
    if deps.contains("sidekiq") {
        globals.extend(&[
            "perform_async",
            "perform_in",
            "perform_at",
            "set",
            "sidekiq_options",
            "sidekiq_retry_in",
            "sidekiq_retries_exhausted",
            "get_sidekiq_options",
        ]);
    }

    // Pundit — authorization
    if deps.contains("pundit") {
        globals.extend(&[
            "authorize",
            "policy",
            "policy_scope",
            "permitted_attributes",
            "verify_authorized",
            "verify_policy_scoped",
            "pundit_user",
        ]);
    }

    // Pagy — pagination
    if deps.contains("pagy") {
        globals.extend(&["pagy", "pagy_array", "pagy_countless", "pagy_metadata"]);
    }

    // Dry-rb ecosystem
    if deps.contains("dry-validation") || deps.contains("dry-schema") {
        globals.extend(&["params", "json", "hash", "rule", "macro", "required", "optional", "maybe"]);
    }

    // Sorbet runtime
    if deps.contains("sorbet-runtime") || deps.contains("tapioca") {
        globals.extend(&["sig", "T", "abstract!", "interface!", "sealed!", "mixes_in_class_methods"]);
    }

    globals
}

/// Minitest assertions always included (stdlib, no Gemfile dep required).
const MINITEST_CORE: &[&str] = &[
    "assert",
    "refute",
    "assert_equal",
    "refute_equal",
    "assert_nil",
    "refute_nil",
    "assert_raises",
    "assert_match",
    "refute_match",
    "assert_includes",
    "refute_includes",
    "assert_empty",
    "refute_empty",
    "skip",
    "pass",
    "flunk",
];

/// Full Minitest assertion set (when minitest is an explicit dep).
const MINITEST_GLOBALS: &[&str] = &[
    "assert",
    "refute",
    "assert_equal",
    "refute_equal",
    "assert_nil",
    "refute_nil",
    "assert_raises",
    "assert_match",
    "refute_match",
    "assert_includes",
    "refute_includes",
    "assert_empty",
    "refute_empty",
    "assert_respond_to",
    "refute_respond_to",
    "assert_kind_of",
    "refute_kind_of",
    "assert_instance_of",
    "refute_instance_of",
    "assert_operator",
    "refute_operator",
    "assert_predicate",
    "refute_predicate",
    "assert_output",
    "assert_silent",
    "assert_in_delta",
    "refute_in_delta",
    "assert_in_epsilon",
    "refute_in_epsilon",
    "assert_same",
    "refute_same",
    "assert_send",
    "assert_throws",
    "assert_raises",
    "capture_io",
    "capture_subprocess_io",
    "skip",
    "pass",
    "flunk",
    // Minitest::Spec DSL
    "describe",
    "it",
    "before",
    "after",
    "let",
    "subject",
    "must_equal",
    "must_be_nil",
    "must_include",
    "must_be_empty",
    "must_be_kind_of",
    "must_be_instance_of",
    "must_raise",
    "must_respond_to",
    "must_match",
    "wont_equal",
    "wont_be_nil",
    "wont_include",
    "wont_be_empty",
    "wont_be_kind_of",
    "wont_match",
    "wont_respond_to",
    // Minitest::Mock
    "mock",
    "expect",
    "verify",
    "stub",
];

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
    "described_class",
    "shared_examples",
    "shared_context",
    "include_examples",
    "include_context",
    "shared_examples_for",
    "it_behaves_like",
    "it_should_behave_like",
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
    // RSpec metadata helpers
    "metadata",
    "example",
    "current_example",
    "is_expected",
    "will",
];

const RAILS_GLOBALS: &[&str] = &[
    // Rails namespace classes commonly referenced as base classes or type
    // annotations in Rails apps. Each is a gem class, never project source,
    // and the Ruby resolver currently leaks them into unresolved_refs as
    // `Namespace::Class` atomic target_names.
    "ActiveRecord::Base",
    "ActiveRecord::Migration",
    "ActiveRecord::Relation",
    "ActiveRecord::Schema",
    "ActiveRecord::RecordNotFound",
    "ActiveRecord::RecordInvalid",
    "ActiveRecord::RecordNotUnique",
    "ActiveRecord::StatementInvalid",
    "ActiveRecord::Rollback",
    "ActiveSupport::Concern",
    "ActiveSupport::TestCase",
    "ActiveSupport::Notifications",
    "ActiveSupport::Configurable",
    "ActiveSupport::TimeWithZone",
    "ActiveSupport::HashWithIndifferentAccess",
    "ActionController::Base",
    "ActionController::API",
    "ActionController::Parameters",
    "ActionController::TestCase",
    "ActionController::RoutingError",
    "ActionController::UrlFor",
    "ActionDispatch::IntegrationTest",
    "ActionDispatch::TestCase",
    "ActionDispatch::Request",
    "ActionDispatch::Response",
    "ActionView::Base",
    "ActionView::Helpers",
    "ActionView::TestCase",
    "ActionMailer::Base",
    "ActionMailer::TestCase",
    "ActionCable::Channel::Base",
    "ActionCable::Connection::Base",
    "ActiveJob::Base",
    "ActiveJob::TestCase",
    "ActiveModel::Model",
    "ActiveModel::Validator",
    "ActiveModel::Errors",
    "ActiveStorage::Blob",
    "ActiveStorage::Attachment",
    "Rails",
    "Rails.application",
    "Rails.root",
    "Rails.env",
    "Rails.logger",
    "Rails.configuration",
    "Rails.cache",
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
    // ActiveRecord query interface (class-level on AR models)
    "find",
    "find_each",
    "find_in_batches",
    "find_by",
    "find_by!",
    "find_or_create_by",
    "find_or_initialize_by",
    "where",
    "order",
    "limit",
    "offset",
    "includes",
    "eager_load",
    "preload",
    "joins",
    "left_joins",
    "left_outer_joins",
    "group",
    "having",
    "select",
    "pluck",
    "pick",
    "count",
    "sum",
    "average",
    "minimum",
    "maximum",
    "exists?",
    "any?",
    "many?",
    "none?",
    "first",
    "first!",
    "last",
    "last!",
    "take",
    "take!",
    "all",
    "none",
    "create",
    "create!",
    "save",
    "save!",
    "update",
    "update!",
    "update_all",
    "update_columns",
    "update_column",
    "destroy",
    "destroy!",
    "destroy_all",
    "delete",
    "delete_all",
    "new",
    "build",
    "unscoped",
    "scoped",
    "distinct",
    "reorder",
    "except",
    "only",
    "lock",
    "transaction",
    "connection",
    "reflect_on_association",
    "column_names",
    "attribute_names",
    "reset_column_information",
    "table_name",
    // ActiveRecord DSL — class-level macros injected by ActiveRecord::Base
    "belongs_to",
    "has_many",
    "has_one",
    "has_and_belongs_to_many",
    "validates",
    "validates_presence_of",
    "validates_uniqueness_of",
    "validates_format_of",
    "validates_length_of",
    "validates_numericality_of",
    "validate",
    "scope",
    "default_scope",
    "before_save",
    "after_save",
    "before_create",
    "after_create",
    "before_update",
    "after_update",
    "before_destroy",
    "after_destroy",
    "before_validation",
    "after_validation",
    "after_commit",
    "after_rollback",
    "after_initialize",
    "after_find",
    "attr_accessor",
    "attr_reader",
    "attr_writer",
    "class_attribute",
    "delegate",
    "enum",
    "store",
    "store_accessor",
    "serialize",
    "composed_of",
    // ActionController DSL
    "before_action",
    "after_action",
    "around_action",
    "skip_before_action",
    "skip_after_action",
    "prepend_before_action",
    "helper_method",
    "rescue_from",
    "protect_from_forgery",
    "force_ssl",
    "layout",
    "http_basic_authenticate_with",
    // ActionMailer DSL
    "default",
    "mail",
    "attachments",
    // ActiveJob DSL
    "queue_as",
    "retry_on",
    "discard_on",
    // ActiveSupport helpers (mixed into all objects in Rails)
    "present?",
    "blank?",
    "presence",
    "try",
    "try!",
    "in?",
    "to_json",
    "as_json",
    "freeze",
    "deep_dup",
    "deep_merge",
    "deep_merge!",
    "symbolize_keys",
    "symbolize_keys!",
    "stringify_keys",
    "stringify_keys!",
    "with_indifferent_access",
    "constantize",
    "safe_constantize",
    "camelize",
    "underscore",
    "titleize",
    "pluralize",
    "singularize",
    "humanize",
    "parameterize",
    "dasherize",
    "classify",
    "demodulize",
    "deconstantize",
    "foreign_key",
    "tableize",
    "to_param",
    "to_query",
    // ActiveSupport::Concern
    "included",
    "extended",
    "prepended",
    "class_methods",
    // Rails helpers (ActionView)
    "render",
    "redirect_to",
    "redirect_back",
    "redirect_back_or_to",
    "send_file",
    "send_data",
    "head",
    "url_for",
    "link_to",
    "form_for",
    "form_with",
    "content_tag",
    "tag",
    "image_tag",
    "javascript_include_tag",
    "stylesheet_link_tag",
    "number_to_currency",
    "number_to_percentage",
    "number_with_delimiter",
    "truncate",
    "highlight",
    "strip_tags",
    "sanitize",
    "time_ago_in_words",
    "distance_of_time_in_words",
    "pluralize",
    // Rails test helpers
    "assert_response",
    "assert_redirected_to",
    "assert_template",
    "assert_difference",
    "assert_no_difference",
    "assert_enqueued_jobs",
    "assert_performed_jobs",
    "assert_emails",
    "assert_no_emails",
    "travel_to",
    "travel",
    "freeze_time",
];
