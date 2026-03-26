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
