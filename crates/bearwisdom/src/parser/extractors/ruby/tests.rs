    use super::*;
    use crate::types::{EdgeKind, SymbolKind, Visibility};

    #[test]
    fn extracts_class_and_method() {
        let source = r#"
class Animal
  def initialize(name)
    @name = name
  end

  def speak
    "..."
  end
end
"#;
        let r = extract(source);
        assert!(!r.has_errors);
        let cls = r.symbols.iter().find(|s| s.name == "Animal").expect("Animal class");
        assert_eq!(cls.kind, SymbolKind::Class);

        let init = r.symbols.iter().find(|s| s.name == "initialize").expect("initialize");
        assert_eq!(init.kind, SymbolKind::Constructor);
        assert_eq!(init.qualified_name, "Animal::initialize");

        let speak = r.symbols.iter().find(|s| s.name == "speak").expect("speak");
        assert_eq!(speak.kind, SymbolKind::Method);
    }

    #[test]
    fn extracts_module() {
        let source = "module Greetable\n  def greet\n    puts 'hi'\n  end\nend\n";
        let r = extract(source);
        let m = r.symbols.iter().find(|s| s.name == "Greetable").expect("Greetable");
        assert_eq!(m.kind, SymbolKind::Interface);
    }

    #[test]
    fn require_produces_import_ref() {
        let source = "require 'net/http'\n";
        let r = extract(source);
        let imp = r.refs.iter().find(|r| r.kind == EdgeKind::Imports).expect("import ref");
        assert_eq!(imp.target_name, "http");
        assert_eq!(imp.module.as_deref(), Some("net"));
    }

    #[test]
    fn require_relative_produces_import_ref() {
        let source = "require_relative '../models/user'\n";
        let r = extract(source);
        let imp = r.refs.iter().find(|r| r.kind == EdgeKind::Imports).expect("import ref");
        assert_eq!(imp.target_name, "user");
    }

    #[test]
    fn inheritance_produces_inherits_edge() {
        let source = "class Dog < Animal\nend\n";
        let r = extract(source);
        let inh = r.refs.iter().find(|r| r.kind == EdgeKind::Inherits).expect("inherits ref");
        assert_eq!(inh.target_name, "Animal");
    }

    #[test]
    fn attr_accessor_produces_property_symbols() {
        let source = r#"
class Person
  attr_accessor :name, :age
end
"#;
        let r = extract(source);
        let props: Vec<_> = r.symbols.iter().filter(|s| s.kind == SymbolKind::Property).collect();
        let names: Vec<&str> = props.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"name"), "name missing: {names:?}");
        assert!(names.contains(&"age"), "age missing: {names:?}");
    }

    #[test]
    fn underscore_method_is_private() {
        let source = "def _helper\nend\n";
        let r = extract(source);
        let sym = r.symbols.iter().find(|s| s.name == "_helper").expect("_helper");
        assert_eq!(sym.visibility, Some(Visibility::Private));
    }

    #[test]
    fn class_new_produces_instantiates_edge() {
        let source = r#"
class Foo
  def build
    Bar.new
  end
end
"#;
        let r = extract(source);
        let inst = r.refs.iter().find(|r| r.kind == EdgeKind::Instantiates);
        assert!(inst.is_some(), "Expected Instantiates edge for Bar.new");
        assert_eq!(inst.unwrap().target_name, "Bar");
    }

    #[test]
    fn handles_parse_errors_gracefully() {
        let source = "class Broken\ndef bad(\nend\n{{{";
        let result = std::panic::catch_unwind(|| extract(source));
        assert!(result.is_ok(), "extractor panicked on malformed input");
    }

    #[test]
    fn calls_inside_brace_block_are_extracted() {
        let source = r#"
class Order
  def process
    items.each { |item| item.save }
  end
end
"#;
        let r = extract(source);
        let calls: Vec<&str> = r.refs.iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"each"),  "Missing 'each': {calls:?}");
        assert!(calls.contains(&"save"),  "Missing 'save' (inside block): {calls:?}");
    }

    #[test]
    fn calls_inside_do_block_are_extracted() {
        let source = r#"
class Repo
  def run
    items.map do |item|
      item.process
    end
  end
end
"#;
        let r = extract(source);
        let calls: Vec<&str> = r.refs.iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"map"),     "Missing 'map': {calls:?}");
        assert!(calls.contains(&"process"), "Missing 'process' (inside do block): {calls:?}");
    }

    #[test]
    fn block_parameters_emitted_as_variable_symbols() {
        let source = r#"
class Svc
  def run
    items.each { |item| item.name }
  end
end
"#;
        let r = extract(source);
        let vars: Vec<&str> = r.symbols.iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(vars.contains(&"item"), "Missing block param 'item': {vars:?}");
    }

    #[test]
    fn method_keyword_params_emitted_as_variables() {
        let source = r#"
class UserService
  def create(name:, email: nil, &block)
    User.new
  end
end
"#;
        let r = extract(source);
        let vars: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(vars.contains(&"name"),  "Missing keyword param 'name': {vars:?}");
        assert!(vars.contains(&"email"), "Missing optional param 'email': {vars:?}");
        assert!(vars.contains(&"block"), "Missing block param 'block': {vars:?}");
    }

    #[test]
    fn method_splat_params_emitted_as_variables() {
        let source = r#"
def log(*args, **opts)
end
"#;
        let r = extract(source);
        let vars: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(vars.contains(&"args"), "Missing splat param 'args': {vars:?}");
        assert!(vars.contains(&"opts"), "Missing hash splat 'opts': {vars:?}");
    }

    #[test]
    fn rescue_exception_type_emits_typeref() {
        let source = r#"
class Repo
  def find(id)
    User.find(id)
  rescue ActiveRecord::RecordNotFound => e
    nil
  rescue StandardError => e
    raise
  end
end
"#;
        let r = extract(source);
        let typerefs: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            typerefs.iter().any(|n| n.contains("RecordNotFound") || n.contains("ActiveRecord")),
            "Expected TypeRef for ActiveRecord::RecordNotFound: {typerefs:?}"
        );
        assert!(
            typerefs.contains(&"StandardError"),
            "Expected TypeRef for StandardError: {typerefs:?}"
        );
    }

    #[test]
    fn rescue_variable_emitted_as_variable_symbol() {
        let source = r#"
def run
  do_work
rescue => e
  log(e)
end
"#;
        let r = extract(source);
        let vars: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(vars.contains(&"e"), "Expected rescue variable 'e': {vars:?}");
    }

    // -----------------------------------------------------------------------
    // String interpolation
    // -----------------------------------------------------------------------

    #[test]
    fn string_interpolation_calls_extracted() {
        let source = r#"
class Greeter
  def greet(user)
    "Hello #{user.get_name()}"
  end
end
"#;
        let r = extract(source);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            calls.contains(&"get_name"),
            "expected get_name() from string interpolation, got: {calls:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Case / when
    // -----------------------------------------------------------------------

    #[test]
    fn case_when_body_calls_extracted() {
        let source = r#"
class Handler
  def handle(command)
    case command
    when :create
      create_record()
    when :delete
      delete_record()
    else
      log_unknown()
    end
  end
end
"#;
        let r = extract(source);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"create_record"), "expected create_record: {calls:?}");
        assert!(calls.contains(&"delete_record"), "expected delete_record: {calls:?}");
        assert!(calls.contains(&"log_unknown"), "expected log_unknown: {calls:?}");
    }

    // -----------------------------------------------------------------------
    // Begin / ensure
    // -----------------------------------------------------------------------

    #[test]
    fn begin_block_calls_extracted() {
        let source = r#"
def run
  begin
    do_work()
  rescue => e
    handle_error(e)
  ensure
    cleanup()
  end
end
"#;
        let r = extract(source);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"do_work"), "expected do_work from begin: {calls:?}");
        assert!(calls.contains(&"cleanup"), "expected cleanup from ensure: {calls:?}");
    }

    // -----------------------------------------------------------------------
    // Hash / array expressions with calls
    // -----------------------------------------------------------------------

    #[test]
    fn hash_value_calls_extracted() {
        let source = r#"
def build_options
  { name: compute_name(), count: fetch_count() }
end
"#;
        let r = extract(source);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"compute_name"), "expected compute_name: {calls:?}");
        assert!(calls.contains(&"fetch_count"), "expected fetch_count: {calls:?}");
    }

    #[test]
    fn array_element_calls_extracted() {
        let source = r#"
def build_list
  [fetch_first(), fetch_second()]
end
"#;
        let r = extract(source);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"fetch_first"), "expected fetch_first: {calls:?}");
        assert!(calls.contains(&"fetch_second"), "expected fetch_second: {calls:?}");
    }

    // -----------------------------------------------------------------------
    // Singleton method (def self.foo)
    // -----------------------------------------------------------------------

    #[test]
    fn singleton_method_emitted_as_method() {
        let source = r#"
class Repo
  def self.find(id)
    where(id: id).first
  end
end
"#;
        let r = extract(source);
        let m = r.symbols.iter().find(|s| s.name == "find").expect("find method");
        assert_eq!(m.kind, SymbolKind::Method);
    }

    // -----------------------------------------------------------------------
    // Multiple exception types in rescue
    // -----------------------------------------------------------------------

    #[test]
    fn rescue_multiple_exceptions_emits_all_typerefs() {
        let source = r#"
def run
  do_work
rescue ArgumentError, TypeError => e
  handle(e)
end
"#;
        let r = extract(source);
        let typerefs: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(typerefs.contains(&"ArgumentError"), "expected ArgumentError TypeRef: {typerefs:?}");
        assert!(typerefs.contains(&"TypeError"), "expected TypeError TypeRef: {typerefs:?}");
    }

    // -----------------------------------------------------------------------
    // Singleton class (class << self)
    // -----------------------------------------------------------------------

    #[test]
    fn singleton_class_methods_are_extracted() {
        let source = r#"
class Repo
  class << self
    def find(id)
      where(id: id).first
    end

    def all
      order(:name)
    end
  end
end
"#;
        let r = extract(source);
        let method_names: Vec<&str> = r
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method || s.kind == SymbolKind::Function)
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            method_names.contains(&"find"),
            "expected 'find' from singleton class body: {method_names:?}"
        );
        assert!(
            method_names.contains(&"all"),
            "expected 'all' from singleton class body: {method_names:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Command call (method call without parens)
    // -----------------------------------------------------------------------

    #[test]
    fn command_call_emits_calls_edge() {
        let source = r#"
class Greeter
  def greet(user)
    puts "Hello #{user.name}"
  end
end
"#;
        let r = extract(source);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"puts"), "expected 'puts' Calls edge: {calls:?}");
    }

    #[test]
    fn command_call_with_receiver_emits_calls_edge() {
        let source = r#"
class Logger
  def log(msg)
    Rails.logger.info msg
  end
end
"#;
        let r = extract(source);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"info"), "expected 'info' Calls edge: {calls:?}");
    }

