    use super::*;
    use crate::types::{EdgeKind, SymbolKind, Visibility};

    fn sym(source: &str) -> Vec<ExtractedSymbol> { extract(source).symbols }
    fn refs(source: &str) -> Vec<ExtractedRef>    { extract(source).refs }

    #[test]
    fn extracts_class_with_namespace() {
        let src = "namespace App { public class UserService {} }";
        let symbols = sym(src);
        let svc = symbols.iter().find(|s| s.name == "UserService").unwrap();
        assert_eq!(svc.kind, SymbolKind::Class);
        assert_eq!(svc.visibility, Some(Visibility::Public));
        assert_eq!(svc.qualified_name, "App.UserService");
    }

    #[test]
    fn extracts_interface() {
        let src = "public interface IRepo { void Save(); }";
        let symbols = sym(src);
        let iface = symbols.iter().find(|s| s.name == "IRepo").unwrap();
        assert_eq!(iface.kind, SymbolKind::Interface);
    }

    #[test]
    fn extracts_enum_and_members() {
        let src = "public enum Color { Red, Green, Blue }";
        let symbols = sym(src);
        assert!(symbols.iter().any(|s| s.name == "Color" && s.kind == SymbolKind::Enum));
        assert!(symbols.iter().any(|s| s.name == "Red" && s.kind == SymbolKind::EnumMember));
        assert!(symbols.iter().any(|s| s.name == "Blue" && s.kind == SymbolKind::EnumMember));
    }

    #[test]
    fn extracts_method_signature() {
        let src = r#"
namespace Catalog {
    class CatalogService {
        public async Task<Item> GetItem(int id) { return null; }
    }
}"#;
        let symbols = sym(src);
        let m = symbols.iter().find(|s| s.name == "GetItem").unwrap();
        assert_eq!(m.kind, SymbolKind::Method);
        assert!(m.signature.as_ref().unwrap().contains("GetItem"));
        assert_eq!(m.qualified_name, "Catalog.CatalogService.GetItem");
    }

    #[test]
    fn extracts_constructor() {
        let src = "class Svc { public Svc(string name) {} }";
        let symbols = sym(src);
        let c = symbols.iter().find(|s| s.kind == SymbolKind::Constructor).unwrap();
        assert_eq!(c.name, "Svc");
    }

    #[test]
    fn extracts_property() {
        let src = "class Foo { public string Name { get; set; } }";
        let symbols = sym(src);
        let p = symbols.iter().find(|s| s.name == "Name").unwrap();
        assert_eq!(p.kind, SymbolKind::Property);
    }

    #[test]
    fn extracts_inheritance_edges() {
        let src = "class Foo : Bar, IBaz {}";
        let r = refs(src);
        assert!(r.iter().any(|r| r.target_name == "Bar" && r.kind == EdgeKind::Inherits));
        assert!(r.iter().any(|r| r.target_name == "IBaz" && r.kind == EdgeKind::Implements));
    }

    #[test]
    fn extracts_call_edges() {
        let src = r#"class S { void Run() { Foo(); bar.Baz(); } }"#;
        let r = refs(src);
        let calls: Vec<_> = r.iter().filter(|r| r.kind == EdgeKind::Calls).collect();
        let names: Vec<&str> = calls.iter().map(|r| r.target_name.as_str()).collect();
        assert!(names.contains(&"Foo"), "Missing Foo: {names:?}");
        assert!(names.contains(&"Baz"), "Missing Baz: {names:?}");
    }

    #[test]
    fn extracts_instantiation_edges() {
        let src = "class S { void Run() { var x = new Foo(); } }";
        let r = refs(src);
        assert!(r.iter().any(|r| r.target_name == "Foo" && r.kind == EdgeKind::Instantiates));
    }

    #[test]
    fn extracts_http_get_attribute() {
        let src = r#"
class CatalogController {
    [HttpGet("/api/catalog/{id}")]
    public IResult GetById(int id) { return Results.Ok(); }
}"#;
        let result = extract(src);
        assert!(!result.routes.is_empty(), "No routes extracted");
        let route = &result.routes[0];
        assert_eq!(route.http_method, "GET");
        assert!(route.template.contains("catalog"), "Template: {}", route.template);
    }

    #[test]
    fn extracts_test_method_kind() {
        let src = r#"
class Tests {
    [Fact]
    public void ShouldWork() {}
}"#;
        let symbols = sym(src);
        let t = symbols.iter().find(|s| s.name == "ShouldWork").unwrap();
        assert_eq!(t.kind, SymbolKind::Test);
    }

    #[test]
    fn extracts_dbset_properties() {
        let src = r#"
class CatalogDbContext : DbContext {
    public DbSet<CatalogItem> CatalogItems { get; set; }
    public DbSet<CatalogBrand> CatalogBrands { get; set; }
}"#;
        let result = extract(src);
        assert!(!result.db_sets.is_empty(), "No DbSets extracted");
        assert!(result.db_sets.iter().any(|d| d.entity_type == "CatalogItem"));
        assert!(result.db_sets.iter().any(|d| d.entity_type == "CatalogBrand"));
    }

    #[test]
    fn does_not_panic_on_malformed_source() {
        let src = "public class { broken !!! @@@ ###";
        let _ = extract(src); // must not panic
    }

    #[test]
    fn extracts_type_refs_from_method_signature() {
        let src = r#"
using FamilyBudget.Api.Entities;

namespace FamilyBudget.Api.Controllers;

class CategoriesController {
    public async Task<ActionResult<Category>> GetCategories() { return null; }
    public async Task<ActionResult<Category>> CreateCategory(Category category) { return null; }
}
"#;
        let result = extract(src);
        let type_refs: Vec<_> = result
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef && r.target_name == "Category")
            .collect();
        assert!(
            type_refs.len() >= 2,
            "Expected at least 2 Category type refs (return type + parameter), got {}. All refs: {:?}",
            type_refs.len(),
            result.refs.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn extracts_using_as_namespace_import() {
        let src = "using FamilyBudget.Api.Entities;";
        let result = extract(src);
        let imports: Vec<_> = result
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .collect();
        assert!(
            !imports.is_empty(),
            "Expected using directive to produce an Imports ref. Got: {:?}",
            result.refs.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
        assert_eq!(
            imports[0].module.as_deref(),
            Some("FamilyBudget.Api.Entities")
        );
    }

    #[test]
    fn extracts_property_type_ref() {
        let src = r#"
class Transaction {
    public Category? Category { get; set; }
    public int CategoryId { get; set; }
}
"#;
        let result = extract(src);
        let cat_refs: Vec<_> = result
            .refs
            .iter()
            .filter(|r| r.target_name == "Category" && r.kind == EdgeKind::TypeRef)
            .collect();
        assert!(
            !cat_refs.is_empty(),
            "Expected Category type ref from property type. Got refs: {:?}",
            result.refs.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn file_scoped_namespace_is_handled() {
        let src = "namespace App.Catalog;\npublic class CatalogApi {}";
        let symbols = sym(src);
        let cls = symbols.iter().find(|s| s.name == "CatalogApi").unwrap();
        assert!(
            cls.qualified_name.contains("CatalogApi"),
            "qualified_name: {}",
            cls.qualified_name
        );
    }

    // WP-6: Record primary constructor parameters extracted as properties.
    #[test]
    fn record_primary_constructor_params_extracted_as_properties() {
        let src = r#"
namespace Geometry {
    public record Point(int X, int Y);
}
"#;
        let symbols = sym(src);
        // The record itself should be extracted as a Class.
        let rec = symbols.iter().find(|s| s.name == "Point").unwrap();
        assert_eq!(rec.kind, SymbolKind::Class, "record should be Class kind");

        // X and Y should be extracted as Property symbols with the record as parent.
        let x = symbols
            .iter()
            .find(|s| s.name == "X" && s.kind == SymbolKind::Property);
        assert!(x.is_some(), "Expected property X from record primary ctor");
        let x = x.unwrap();
        assert!(
            x.qualified_name.contains("Point.X"),
            "X.qualified_name should contain 'Point.X', got: {}",
            x.qualified_name
        );
        assert_eq!(
            x.scope_path.as_deref(),
            Some("Geometry.Point"),
            "X.scope_path should be 'Geometry.Point', got: {:?}",
            x.scope_path
        );
        assert_eq!(x.visibility, Some(Visibility::Public));

        let y = symbols.iter().find(|s| s.name == "Y" && s.kind == SymbolKind::Property);
        assert!(y.is_some(), "Expected property Y from record primary ctor");
    }

    // WP-6: Record with body — existing body members not duplicated.
    #[test]
    fn record_with_body_extracts_both_params_and_body_members() {
        let src = r#"
record Person(string Name) {
    public int Age { get; init; }
}
"#;
        let symbols = sym(src);
        let props: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Property).collect();
        // Should have Name (from primary ctor) and Age (from body).
        let names: Vec<&str> = props.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Name"), "Expected Name property: {names:?}");
        assert!(names.contains(&"Age"), "Expected Age property: {names:?}");
    }

    // -----------------------------------------------------------------------
    // MapGroup prefix resolution
    // -----------------------------------------------------------------------

    #[test]
    fn minimal_api_mapgroup_simple() {
        let src = r#"
class OrdersApi {
    static void MapOrdersApiV1(IEndpointRouteBuilder app) {
        var api = app.MapGroup("api/orders");
        api.MapGet("/", GetOrdersByUserAsync);
        api.MapGet("{orderId:int}", GetOrderAsync);
        api.MapPost("/", CreateOrderAsync);
        api.MapPut("/cancel", CancelOrderAsync);
    }
}
"#;
        let result = extract(src);
        let templates: Vec<&str> = result.routes.iter().map(|r| r.template.as_str()).collect();
        assert!(templates.contains(&"api/orders"), "Expected 'api/orders', got: {templates:?}");
        assert!(templates.contains(&"api/orders/{orderId:int}"), "Expected 'api/orders/{{orderId:int}}', got: {templates:?}");
        assert!(templates.contains(&"api/orders/cancel"), "Expected 'api/orders/cancel', got: {templates:?}");
        assert_eq!(result.routes.len(), 4, "Expected 4 routes, got: {templates:?}");
    }

    #[test]
    fn minimal_api_mapgroup_chained_variables() {
        let src = r#"
class CatalogApi {
    static void MapCatalogApi(IEndpointRouteBuilder app) {
        var vApi = app.MapGroup("api");
        var catalog = vApi.MapGroup("catalog");
        catalog.MapGet("/items", GetItems);
        catalog.MapGet("/items/{id}", GetItem);
    }
}
"#;
        let result = extract(src);
        let templates: Vec<&str> = result.routes.iter().map(|r| r.template.as_str()).collect();
        assert!(templates.contains(&"api/catalog/items"), "Expected 'api/catalog/items', got: {templates:?}");
        assert!(templates.contains(&"api/catalog/items/{id}"), "Expected 'api/catalog/items/{{id}}', got: {templates:?}");
    }

    #[test]
    fn minimal_api_mapgroup_fluent_chain() {
        let src = r#"
class Api {
    static void Map(IEndpointRouteBuilder app) {
        var api = app.MapGroup("api/catalog").RequireAuthorization().WithTags("Catalog");
        api.MapGet("/items", GetAllItems);
        api.MapDelete("/items/{id:int}", DeleteItem);
    }
}
"#;
        let result = extract(src);
        let templates: Vec<&str> = result.routes.iter().map(|r| r.template.as_str()).collect();
        assert!(templates.contains(&"api/catalog/items"), "Expected 'api/catalog/items', got: {templates:?}");
        assert!(templates.contains(&"api/catalog/items/{id:int}"), "Expected 'api/catalog/items/{{id:int}}', got: {templates:?}");
    }

    #[test]
    fn minimal_api_without_mapgroup_unchanged() {
        let src = r#"
class Program {
    static void Main() {
        app.MapGet("/api/items", GetItems);
        app.MapPost("/api/items", CreateItem);
    }
}
"#;
        let result = extract(src);
        let templates: Vec<&str> = result.routes.iter().map(|r| r.template.as_str()).collect();
        assert!(templates.contains(&"/api/items"), "Expected '/api/items' preserved, got: {templates:?}");
        assert_eq!(result.routes.len(), 2, "Expected 2 routes, got: {templates:?}");
    }

    // -----------------------------------------------------------------------
    // Type narrowing — is expressions and switch expressions
    // -----------------------------------------------------------------------

    #[test]
    fn is_expression_emits_type_ref() {
        let src = r#"
namespace App {
    class AuthService {
        public void Check(object user) {
            if (user is Admin) {
                System.Console.WriteLine("admin");
            }
        }
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
            "refs: {r:?}"
        );
    }

    #[test]
    fn is_pattern_expression_emits_type_ref() {
        let src = r#"
namespace App {
    class AuthService {
        public void Check(object user) {
            if (user is Admin admin) {
                admin.DoStuff();
            }
        }
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
            "refs: {r:?}"
        );
    }

    #[test]
    fn switch_expression_pattern_emits_type_ref() {
        let src = r#"
namespace App {
    class LevelService {
        public int GetLevel(object user) {
            return user switch {
                Admin a => a.Level,
                _ => 0,
            };
        }
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
            "refs: {r:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Lambda expression extraction
    // -----------------------------------------------------------------------

    #[test]
    fn lambda_single_param_extracted_as_variable() {
        let src = r#"
class S {
    void Run() {
        var names = users.Select(u => u.Name);
    }
}
"#;
        let symbols = sym(src);
        let var_u = symbols.iter().find(|s| s.name == "u" && s.kind == SymbolKind::Variable);
        assert!(var_u.is_some(), "Expected variable symbol 'u' from lambda param. Symbols: {:?}",
            symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>());
    }

    #[test]
    fn lambda_multi_param_extracted_as_variables() {
        let src = r#"
class S {
    void Run() {
        var result = pairs.Select((x, y) => Combine(x, y));
    }
}
"#;
        let symbols = sym(src);
        let names: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(names.contains(&"x"), "Expected variable 'x': {names:?}");
        assert!(names.contains(&"y"), "Expected variable 'y': {names:?}");
    }

    #[test]
    fn lambda_body_calls_extracted() {
        // Calls inside a lambda body must still be emitted.
        let src = r#"
class S {
    void Run() {
        var active = items.Where(x => Validate(x));
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Validate" && r.kind == EdgeKind::Calls),
            "Expected Validate call from inside lambda body. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    // -----------------------------------------------------------------------
    // LINQ query expression extraction
    // -----------------------------------------------------------------------

    #[test]
    fn linq_range_variable_extracted_as_variable() {
        let src = r#"
class S {
    void Run() {
        var q = from u in users where u.IsActive select u.Name;
    }
}
"#;
        let symbols = sym(src);
        let var_u = symbols.iter().find(|s| s.name == "u" && s.kind == SymbolKind::Variable);
        assert!(var_u.is_some(), "Expected variable symbol 'u' from LINQ from_clause. Symbols: {:?}",
            symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>());
    }

    // -----------------------------------------------------------------------
    // Switch expression pattern binding variables
    // -----------------------------------------------------------------------

    #[test]
    fn switch_expression_pattern_binding_variable_extracted() {
        let src = r#"
namespace App {
    class LevelService {
        public int GetLevel(object user) {
            return user switch {
                Admin a => a.Level,
                Student s => s.Grade,
                _ => 0,
            };
        }
    }
}
"#;
        let symbols = sym(src);
        let var_names: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(var_names.contains(&"a"), "Expected variable 'a' from switch pattern. Variables: {var_names:?}");
        assert!(var_names.contains(&"s"), "Expected variable 's' from switch pattern. Variables: {var_names:?}");
    }

    #[test]
    fn is_pattern_binding_variable_extracted() {
        let src = r#"
namespace App {
    class AuthService {
        public void Check(object user) {
            if (user is Admin admin) {
                admin.DoStuff();
            }
        }
    }
}
"#;
        let symbols = sym(src);
        let var_admin = symbols.iter().find(|s| s.name == "admin" && s.kind == SymbolKind::Variable);
        assert!(var_admin.is_some(), "Expected variable symbol 'admin' from is-pattern binding. Symbols: {:?}",
            symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>());
    }

    // -----------------------------------------------------------------------
    // Nullable type annotation (already handled — regression guard)
    // -----------------------------------------------------------------------

    #[test]
    fn nullable_type_in_property_emits_type_ref() {
        // `Category?` should produce a TypeRef to `Category` (not `Category?`).
        let src = r#"
class Order {
    public Category? Category { get; set; }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Category" && r.kind == EdgeKind::TypeRef),
            "Expected TypeRef to Category from nullable property type. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    // -----------------------------------------------------------------------
    // using_statement — object_creation inside using block is reachable
    // -----------------------------------------------------------------------

    #[test]
    fn using_statement_instantiation_extracted() {
        let src = r#"
class S {
    void Run() {
        using (var conn = new DbConnection()) {
            conn.Open();
        }
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "DbConnection" && r.kind == EdgeKind::Instantiates),
            "Expected Instantiates edge for DbConnection. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
        assert!(
            r.iter().any(|r| r.target_name == "Open" && r.kind == EdgeKind::Calls),
            "Expected Calls edge for Open. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn using_var_declaration_instantiation_extracted() {
        // `using var db = new Database();` — no parens, statement-level using.
        let src = r#"
class S {
    void Run() {
        using var db = new Database();
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Database" && r.kind == EdgeKind::Instantiates),
            "Expected Instantiates edge for Database. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    // -----------------------------------------------------------------------
    // Pattern matching completeness
    // -----------------------------------------------------------------------

    #[test]
    fn or_pattern_emits_type_refs_for_both_branches() {
        // `x is Admin or User` — both Admin and User should produce TypeRefs.
        let src = r#"
class S {
    void F(object x) {
        bool r = x is Admin or User;
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
            "Expected TypeRef to Admin in or_pattern. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
        assert!(
            r.iter().any(|r| r.target_name == "User" && r.kind == EdgeKind::TypeRef),
            "Expected TypeRef to User in or_pattern. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn negated_pattern_emits_type_ref() {
        // `x is not Admin` — Admin should produce a TypeRef.
        let src = r#"
class S {
    void F(object x) {
        if (x is not Admin) {}
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
            "Expected TypeRef to Admin in negated_pattern. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn var_pattern_binding_variable_extracted() {
        // `x is var v` — v should be extracted as a Variable symbol.
        let src = r#"
class S {
    void F(object x) {
        if (x is var v) {}
    }
}
"#;
        let symbols = sym(src);
        let var_v = symbols.iter().find(|s| s.name == "v" && s.kind == SymbolKind::Variable);
        assert!(
            var_v.is_some(),
            "Expected variable symbol 'v' from var pattern. Symbols: {:?}",
            symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn switch_expression_or_pattern_arm_emits_type_refs() {
        // `x switch { Admin or User => ... }` — TypeRefs for Admin and User.
        let src = r#"
class S {
    void F(object x) {
        var r = x switch {
            Admin or User => 1,
            _ => 0,
        };
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
            "Expected TypeRef to Admin in switch or_pattern arm. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
        assert!(
            r.iter().any(|r| r.target_name == "User" && r.kind == EdgeKind::TypeRef),
            "Expected TypeRef to User in switch or_pattern arm. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn switch_expression_arm_calls_extracted() {
        // Calls in switch expression arm bodies must be extracted.
        let src = r#"
class S {
    void F(int x) {
        var r = x switch {
            > 5 => Compute(),
            _ => Default(),
        };
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Compute" && r.kind == EdgeKind::Calls),
            "Expected Calls edge for Compute in switch arm. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
        assert!(
            r.iter().any(|r| r.target_name == "Default" && r.kind == EdgeKind::Calls),
            "Expected Calls edge for Default in switch arm. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    // -----------------------------------------------------------------------
    // Body constructs that must recurse into calls
    // -----------------------------------------------------------------------

    #[test]
    fn lock_statement_calls_extracted() {
        let src = r#"
class S {
    void F() {
        lock (_lock) {
            DoWork();
        }
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "DoWork" && r.kind == EdgeKind::Calls),
            "Expected Calls edge for DoWork inside lock body. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn checked_statement_calls_extracted() {
        let src = r#"
class S {
    void F() {
        checked { ProcessChecked(); }
        unchecked { ProcessUnchecked(); }
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "ProcessChecked" && r.kind == EdgeKind::Calls),
            "Expected Calls edge for ProcessChecked inside checked block. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
        assert!(
            r.iter().any(|r| r.target_name == "ProcessUnchecked" && r.kind == EdgeKind::Calls),
            "Expected Calls edge for ProcessUnchecked inside unchecked block. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn unsafe_statement_calls_extracted() {
        let src = r#"
class S {
    unsafe void F() {
        unsafe {
            ReadMemory();
        }
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "ReadMemory" && r.kind == EdgeKind::Calls),
            "Expected Calls edge for ReadMemory inside unsafe block. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn fixed_statement_calls_extracted() {
        let src = r#"
class S {
    unsafe void F() {
        int x = 1;
        fixed (int* p = &x) {
            ReadPtr(p);
        }
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "ReadPtr" && r.kind == EdgeKind::Calls),
            "Expected Calls edge for ReadPtr inside fixed block. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn yield_statement_calls_extracted() {
        let src = r#"
class S {
    System.Collections.Generic.IEnumerable<int> F() {
        yield return Calc();
        yield break;
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Calc" && r.kind == EdgeKind::Calls),
            "Expected Calls edge for Calc inside yield return. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn string_interpolation_calls_extracted() {
        let src = r#"
class S {
    void F() {
        var s = $"Hello {user.GetName()} ({GetRole()})";
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "GetName" && r.kind == EdgeKind::Calls),
            "Expected Calls edge for GetName inside string interpolation. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
        assert!(
            r.iter().any(|r| r.target_name == "GetRole" && r.kind == EdgeKind::Calls),
            "Expected Calls edge for GetRole inside string interpolation. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn with_expression_calls_extracted() {
        let src = r#"
class S {
    void F() {
        var b = a with { Name = Compute() };
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Compute" && r.kind == EdgeKind::Calls),
            "Expected Calls edge for Compute inside with_expression. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn tuple_deconstruction_calls_extracted() {
        let src = r#"
class S {
    void F() {
        var (a, b) = GetValues();
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "GetValues" && r.kind == EdgeKind::Calls),
            "Expected Calls edge for GetValues in tuple deconstruction. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn discard_assignment_calls_extracted() {
        let src = r#"
class S {
    void F() {
        _ = SomeMethod();
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "SomeMethod" && r.kind == EdgeKind::Calls),
            "Expected Calls edge for SomeMethod in discard assignment. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn block_lambda_calls_extracted() {
        // Block-bodied lambda `() => { Work(); }` — calls inside must be found.
        let src = r#"
class S {
    void F() {
        var t = Task.Run(() => { Work(); });
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Work" && r.kind == EdgeKind::Calls),
            "Expected Calls edge for Work inside block lambda body. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn event_handler_lambda_calls_extracted() {
        // `button.Click += (s, e) => HandleClick(s);` — HandleClick must be found.
        let src = r#"
class S {
    void F() {
        button.Click += (s, e) => HandleClick(s);
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "HandleClick" && r.kind == EdgeKind::Calls),
            "Expected Calls edge for HandleClick in event handler lambda. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn event_handler_lambda_params_extracted_as_variables() {
        // `(s, e) => HandleClick(s)` — s and e should be Variable symbols.
        let src = r#"
class S {
    void F() {
        button.Click += (s, e) => HandleClick(s);
    }
}
"#;
        let symbols = sym(src);
        let var_names: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .map(|s| s.name.as_str())
            .collect();
        assert!(var_names.contains(&"s"), "Expected variable 's' from event handler lambda. Variables: {var_names:?}");
        assert!(var_names.contains(&"e"), "Expected variable 'e' from event handler lambda. Variables: {var_names:?}");
    }

    #[test]
    fn linq_where_clause_calls_extracted() {
        // Calls inside LINQ where/select clauses must be extracted.
        let src = r#"
class S {
    void F() {
        var q = from u in users where IsActive(u) select Transform(u);
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "IsActive" && r.kind == EdgeKind::Calls),
            "Expected Calls edge for IsActive in LINQ where clause. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
        assert!(
            r.iter().any(|r| r.target_name == "Transform" && r.kind == EdgeKind::Calls),
            "Expected Calls edge for Transform in LINQ select clause. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn global_using_directive_emits_import() {
        // `global using System;` — must produce an Imports ref just like a normal using.
        let src = r#"global using System.Collections.Generic;"#;
        let result = extract(src);
        let imports: Vec<_> = result
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .collect();
        assert!(
            !imports.is_empty(),
            "Expected global using to produce an Imports ref. refs: {:?}",
            result.refs.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
        assert_eq!(
            imports[0].module.as_deref(),
            Some("System.Collections.Generic"),
        );
    }

    #[test]
    fn file_scoped_namespace_qualified_name() {
        // `namespace MyApp;` (file-scoped) — class should be qualified under MyApp.
        let src = r#"
namespace MyApp.Services;
public class OrderService {}
"#;
        let symbols = sym(src);
        let svc = symbols.iter().find(|s| s.name == "OrderService").unwrap();
        assert!(
            svc.qualified_name.starts_with("MyApp.Services"),
            "Expected qualified_name to start with 'MyApp.Services', got: {}",
            svc.qualified_name
        );
    }

    #[test]
    fn init_accessor_property_extracted() {
        // `public string Name { get; init; }` — property must be extracted; init does not break it.
        let src = r#"
record Person {
    public string Name { get; init; }
    public int Age { get; init; }
}
"#;
        let symbols = sym(src);
        assert!(
            symbols.iter().any(|s| s.name == "Name" && s.kind == SymbolKind::Property),
            "Expected Name property extracted. Symbols: {:?}",
            symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
        assert!(
            symbols.iter().any(|s| s.name == "Age" && s.kind == SymbolKind::Property),
            "Expected Age property extracted. Symbols: {:?}",
            symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn nullable_type_unwrapped_in_type_ref() {
        // `User?` param should emit TypeRef to `User`, not `User?`.
        let src = r#"
class S {
    public void Process(User? user) {}
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "User" && r.kind == EdgeKind::TypeRef),
            "Expected TypeRef to User (not User?) from nullable param. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
        assert!(
            !r.iter().any(|r| r.target_name == "User?" && r.kind == EdgeKind::TypeRef),
            "TypeRef target should not include the '?' suffix"
        );
    }

    #[test]
    fn record_positional_params_have_type_refs() {
        // `record UserDto(string Name, Category Category)` — TypeRef to Category.
        let src = r#"
namespace App {
    public record UserDto(string Name, Category Category);
}
"#;
        let result = extract(src);
        // The primary ctor params are extracted as Property symbols.
        let symbols = &result.symbols;
        assert!(
            symbols.iter().any(|s| s.name == "Name" && s.kind == SymbolKind::Property),
            "Expected Name property from record primary ctor. Symbols: {:?}",
            symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
        assert!(
            symbols.iter().any(|s| s.name == "Category" && s.kind == SymbolKind::Property),
            "Expected Category property from record primary ctor. Symbols: {:?}",
            symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
    }

    // -----------------------------------------------------------------------
    // New: cast_expression, typeof, nameof, destructor, operator, indexer,
    //      event declaration, local function
    // -----------------------------------------------------------------------

    #[test]
    fn cast_expression_emits_type_ref() {
        let src = r#"
class S {
    void F(object x) {
        var y = (Admin)x;
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
            "Expected TypeRef for cast type Admin. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn as_expression_emits_type_ref() {
        let src = r#"
class S {
    void F(object x) {
        var y = x as Admin;
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
            "Expected TypeRef for as-expression type Admin. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn typeof_expression_emits_type_ref() {
        let src = r#"
class S {
    void F() {
        var t = typeof(Admin);
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
            "Expected TypeRef for typeof(Admin). refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn foreach_explicit_type_emits_type_ref() {
        let src = r#"
class S {
    void F(System.Collections.Generic.List<Admin> items) {
        foreach (Admin item in items) { item.Check(); }
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
            "Expected TypeRef for foreach type Admin. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
        assert!(
            r.iter().any(|r| r.target_name == "Check" && r.kind == EdgeKind::Calls),
            "Expected Calls edge for Check inside foreach body. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn catch_clause_emits_type_ref_for_exception() {
        let src = r#"
class S {
    void F() {
        try { DoWork(); }
        catch (ArgumentException e) { Handle(e); }
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "ArgumentException" && r.kind == EdgeKind::TypeRef),
            "Expected TypeRef for catch clause exception type. refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn destructor_emits_method_symbol() {
        let src = r#"
class Resource {
    ~Resource() { }
}
"#;
        let s = sym(src);
        assert!(
            s.iter().any(|s| s.name == "~Resource" && s.kind == SymbolKind::Method),
            "Expected ~Resource method symbol. Symbols: {:?}",
            s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn operator_overload_emits_method_symbol() {
        let src = r#"
class Money {
    public static Money operator +(Money a, Money b) => new Money();
}
"#;
        let s = sym(src);
        assert!(
            s.iter().any(|s| s.name.starts_with("operator") && s.kind == SymbolKind::Method),
            "Expected operator+ method symbol. Symbols: {:?}",
            s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn conversion_operator_emits_method_symbol() {
        let src = r#"
class Celsius {
    double Value { get; set; }
    public static implicit operator double(Celsius c) => c.Value;
}
"#;
        let s = sym(src);
        assert!(
            s.iter().any(|s| s.kind == SymbolKind::Method && s.name.contains("operator")),
            "Expected conversion operator method symbol. Symbols: {:?}",
            s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn indexer_emits_property_symbol() {
        let src = r#"
class Grid {
    public int this[int x, int y] { get => 0; set {} }
}
"#;
        let s = sym(src);
        assert!(
            s.iter().any(|s| s.name == "this[]" && s.kind == SymbolKind::Property),
            "Expected this[] property symbol for indexer. Symbols: {:?}",
            s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn event_declaration_with_accessors_emits_event_symbol() {
        let src = r#"
class Button {
    public event System.EventHandler Click {
        add { }
        remove { }
    }
}
"#;
        let s = sym(src);
        assert!(
            s.iter().any(|s| s.name == "Click" && s.kind == SymbolKind::Event),
            "Expected Click event symbol. Symbols: {:?}",
            s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn local_function_emits_function_symbol() {
        let src = r#"
class S {
    void Outer() {
        int Inner(int x) => x + 1;
        var r = Inner(5);
    }
}
"#;
        let s = sym(src);
        assert!(
            s.iter().any(|s| s.name == "Inner" && s.kind == SymbolKind::Function),
            "Expected Inner local function symbol. Symbols: {:?}",
            s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
    }

    // -----------------------------------------------------------------------
    // Diagnostic dump — not a real test, just prints node tree structure.
    // -----------------------------------------------------------------------

    #[test]
    fn dump_csharp_node_kinds() {
        let snippets = [
            ("as_expression", "class S { void F(object x) { var y = x as Admin; } }"),
            ("cast_expression", "class S { void F(object x) { var y = (Admin)x; } }"),
            ("foreach", "class S { void F() { foreach (Admin item in items) {} } }"),
            ("catch", "class S { void F() { try { } catch (ArgumentException e) { } } }"),
            ("local_fn", "class S { void Outer() { int Inner(int x) => x + 1; } }"),
            ("typeof", "class S { void F() { var t = typeof(Admin); } }"),
        ];
        for (label, src) in &snippets {
            let language: tree_sitter::Language = tree_sitter_c_sharp::LANGUAGE.into();
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&language).unwrap();
            let tree = parser.parse(src, None).unwrap();
            let root = tree.root_node();
            println!("\n--- {label}: {src}");
            let mut cursor = root.walk();
            for child in root.children(&mut cursor) {
                print_csharp_node_tree(child, 0);
            }
        }
    }

    fn print_csharp_node_tree(node: tree_sitter::Node, depth: usize) {
        let indent = "  ".repeat(depth);
        println!("{}[{}] ({}:{})", indent, node.kind(), node.start_position().row, node.start_position().column);
        if depth < 8 {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                print_csharp_node_tree(child, depth + 1);
            }
        }
    }

