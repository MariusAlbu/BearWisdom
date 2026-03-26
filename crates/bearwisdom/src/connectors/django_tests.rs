use super::*;
use crate::db::Database;
use std::io::Write;
use tempfile::NamedTempFile;

fn make_py_file(content: &str) -> NamedTempFile {
    let mut f = tempfile::Builder::new()
        .suffix(".py")
        .tempfile()
        .unwrap();
    write!(f, "{}", content).unwrap();
    f
}

fn insert_py_file(conn: &Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES (?1, 'h', 'python', 0)",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

// -----------------------------------------------------------------------

#[test]
fn model_regex_matches_models_model() {
    let re = build_model_regex();
    assert!(re.is_match("class Product(models.Model):"));
    let cap = re.captures("class Order(models.Model):").unwrap();
    assert_eq!(&cap[1], "Order");
}

#[test]
fn model_regex_does_not_match_plain_class() {
    let re = build_model_regex();
    assert!(!re.is_match("class MyForm(forms.Form):"));
}

#[test]
fn url_regex_extracts_route_and_view() {
    let re = build_url_path_regex();
    let line = r#"    path('products/', views.ProductListView.as_view(), name='product-list'),"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[1], "products/");
    // The regex captures up to the opening paren of as_view(), so the last
    // dotted component before `()` is included in the capture.
    assert_eq!(&cap[2], "views.ProductListView.as_view");
}

#[test]
fn cbv_regex_matches_class_based_view() {
    let re = build_cbv_regex();
    assert!(re.is_match("class ProductListView(ListView):"));
    let cap = re.captures("class OrderDetailView(DetailView):").unwrap();
    assert_eq!(&cap[1], "OrderDetailView");
}

#[test]
fn fbv_regex_matches_function_view() {
    let re = build_fbv_regex();
    assert!(re.is_match("def my_view(request):"));
    let cap = re.captures("def product_detail(request, pk):").unwrap();
    assert_eq!(&cap[1], "product_detail");
}

#[test]
fn detect_models_inserts_flow_edges() {
    let db = Database::open_in_memory().unwrap();
    let conn = &db.conn;

    let py_file = make_py_file("class Product(models.Model):\n    name = models.CharField()\n");
    let root = py_file.path().parent().unwrap();
    let file_name = py_file.path().file_name().unwrap().to_str().unwrap();

    insert_py_file(conn, file_name);

    let count = detect_django_models(conn, root).unwrap();
    assert_eq!(count, 1, "Expected one django_model flow_edge");

    let edge_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM flow_edges WHERE edge_type = 'django_model'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(edge_count, 1);
}

#[test]
fn detect_urls_inserts_route() {
    let db = Database::open_in_memory().unwrap();
    let conn = &db.conn;

    let py_file = tempfile::Builder::new()
        .prefix("urls")
        .suffix(".py")
        .tempfile()
        .unwrap();
    {
        let mut f = std::fs::File::create(py_file.path()).unwrap();
        write!(f, "urlpatterns = [\n    path('items/', views.item_list),\n]\n").unwrap();
    }
    let root = py_file.path().parent().unwrap();

    // Register as urls.py-equivalent by making the path end with urls.py.
    // The NamedTempFile name won't end in urls.py so we wire the DB path directly.
    let fake_name = "myapp/urls.py";
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES (?1, 'h', 'python', 0)",
        [fake_name],
    )
    .unwrap();

    // Write the actual file at the path the connector will look for.
    let target = root.join("myapp");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(
        target.join("urls.py"),
        "urlpatterns = [\n    path('items/', views.item_list),\n]\n",
    )
    .unwrap();

    let count = detect_django_urls(conn, root).unwrap();
    assert_eq!(count, 1, "Expected one route inserted");

    let route_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM routes", [], |r| r.get(0))
        .unwrap();
    assert_eq!(route_count, 1);
}

#[test]
fn detect_views_inserts_cbv_and_fbv_edges() {
    let db = Database::open_in_memory().unwrap();
    let conn = &db.conn;

    let source = "class ProductListView(ListView):\n    pass\n\ndef order_detail(request, pk):\n    pass\n";
    let py_file = make_py_file(source);
    let root = py_file.path().parent().unwrap();
    let file_name = py_file.path().file_name().unwrap().to_str().unwrap();

    insert_py_file(conn, file_name);

    let count = detect_django_views(conn, root).unwrap();
    assert_eq!(count, 2, "Expected one CBV + one FBV edge");
}

#[test]
fn connect_runs_all_passes_without_error() {
    let db = Database::open_in_memory().unwrap();
    let dir = tempfile::TempDir::new().unwrap();
    // Empty project — should succeed with zero detections.
    connect(&db, dir.path()).unwrap();
}
