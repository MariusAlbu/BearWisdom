use super::*;
use crate::db::Database;

// ---------------------------------------------------------------------------
// Unit tests — regex / source scanning helpers
// ---------------------------------------------------------------------------

#[test]
fn detects_injectable_class() {
    let re_injectable = build_injectable_regex();
    let re_class = build_class_regex();
    let re_provided_in = build_provided_in_regex();

    let source = r#"
@Injectable()
export class UserService {
  constructor() {}
}
"#;

    let mut out: HashMap<String, InjectableService> = HashMap::new();
    collect_injectables(source, 1, &re_injectable, &re_class, &re_provided_in, &mut out);

    assert_eq!(out.len(), 1);
    assert!(out.contains_key("UserService"));
    assert!(!out["UserService"].provided_in_root);
}

#[test]
fn detects_provided_in_root() {
    let re_injectable = build_injectable_regex();
    let re_class = build_class_regex();
    let re_provided_in = build_provided_in_regex();

    let source = r#"
@Injectable({ providedIn: 'root' })
export class AuthService {
  constructor() {}
}
"#;

    let mut out: HashMap<String, InjectableService> = HashMap::new();
    collect_injectables(source, 1, &re_injectable, &re_class, &re_provided_in, &mut out);

    assert_eq!(out.len(), 1);
    assert!(out.contains_key("AuthService"));
    assert!(out["AuthService"].provided_in_root);
}

#[test]
fn detects_constructor_injection() {
    let re_injectable = build_injectable_regex();
    let re_class = build_class_regex();
    let re_param = build_constructor_param_regex();
    let re_provided_in = build_provided_in_regex();

    let service_source = r#"
@Injectable()
export class UserService {}
"#;

    let component_source = r#"
export class DashboardComponent {
  constructor(private userService: UserService) {}
}
"#;

    let mut injectables: HashMap<String, InjectableService> = HashMap::new();
    collect_injectables(
        service_source,
        1,
        &re_injectable,
        &re_class,
        &re_provided_in,
        &mut injectables,
    );

    let mut sites: Vec<InjectionSite> = Vec::new();
    collect_injection_sites(
        component_source,
        2,
        &re_param,
        &re_class,
        &injectables,
        &mut sites,
    );

    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0].injected_type, "UserService");
    assert_eq!(sites[0].consumer_class, "DashboardComponent");
    assert_eq!(sites[0].consumer_file_id, 2);
}

#[test]
fn no_injectables_produces_no_sites() {
    let re_param = build_constructor_param_regex();
    let re_class = build_class_regex();

    let source = r#"
export class AppComponent {
  constructor(private http: HttpClient) {}
}
"#;

    // Empty injectable map — HttpClient is not in it.
    let injectables: HashMap<String, InjectableService> = HashMap::new();
    let mut sites: Vec<InjectionSite> = Vec::new();

    collect_injection_sites(source, 1, &re_param, &re_class, &injectables, &mut sites);

    assert!(sites.is_empty());
}

#[test]
fn multiple_params_in_one_constructor() {
    let re_injectable = build_injectable_regex();
    let re_class = build_class_regex();
    let re_param = build_constructor_param_regex();
    let re_provided_in = build_provided_in_regex();

    let services_source = r#"
@Injectable()
export class AuthService {}

@Injectable({ providedIn: 'root' })
export class LogService {}
"#;

    let component_source = r#"
export class AdminComponent {
  constructor(
    private authService: AuthService,
    public logService: LogService,
  ) {}
}
"#;

    let mut injectables: HashMap<String, InjectableService> = HashMap::new();
    collect_injectables(
        services_source,
        1,
        &re_injectable,
        &re_class,
        &re_provided_in,
        &mut injectables,
    );

    let mut sites: Vec<InjectionSite> = Vec::new();
    collect_injection_sites(
        component_source,
        2,
        &re_param,
        &re_class,
        &injectables,
        &mut sites,
    );

    assert_eq!(sites.len(), 2);
    let types: Vec<&str> = sites.iter().map(|s| s.injected_type.as_str()).collect();
    assert!(types.contains(&"AuthService"));
    assert!(types.contains(&"LogService"));
}

#[test]
fn non_injectable_class_not_collected() {
    let re_injectable = build_injectable_regex();
    let re_class = build_class_regex();
    let re_provided_in = build_provided_in_regex();

    let source = r#"
export class PlainClass {
  constructor() {}
}
"#;

    let mut out: HashMap<String, InjectableService> = HashMap::new();
    collect_injectables(source, 1, &re_injectable, &re_class, &re_provided_in, &mut out);

    assert!(out.is_empty());
}

// ---------------------------------------------------------------------------
// Integration tests — in-memory DB
// ---------------------------------------------------------------------------

fn seed_db(db: &Database) -> (i64, i64) {
    let conn = &db.conn;

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('src/auth.service.ts', 'h1', 'typescript', 0)",
        [],
    )
    .unwrap();
    let service_file_id: i64 = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('src/dashboard.component.ts', 'h2', 'typescript', 0)",
        [],
    )
    .unwrap();
    let component_file_id: i64 = conn.last_insert_rowid();

    (service_file_id, component_file_id)
}

#[test]
fn injectable_to_constructor_creates_flow_edge() {
    let db = Database::open_in_memory().unwrap();
    let (service_file_id, component_file_id) = seed_db(&db);

    // Manually build the data structures that Pass 1 and 2 produce.
    let mut injectables: HashMap<String, InjectableService> = HashMap::new();
    injectables.insert(
        "AuthService".to_string(),
        InjectableService {
            file_id: service_file_id,
            line: 3,
            name: "AuthService".to_string(),
            provided_in_root: true,
        },
    );

    let sites = vec![InjectionSite {
        consumer_file_id: component_file_id,
        line: 7,
        injected_type: "AuthService".to_string(),
        consumer_class: "DashboardComponent".to_string(),
    }];

    let created = insert_flow_edges(&db.conn, &sites, &injectables).unwrap();
    assert_eq!(created, 1);

    let count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM flow_edges WHERE edge_type = 'di_binding'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);

    // Verify edge direction: source = component, target = service.
    let (src_file, tgt_file): (i64, i64) = db
        .conn
        .query_row(
            "SELECT source_file_id, target_file_id FROM flow_edges WHERE edge_type = 'di_binding'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(src_file, component_file_id);
    assert_eq!(tgt_file, service_file_id);
}

#[test]
fn no_injectables_produces_zero_edges() {
    let db = Database::open_in_memory().unwrap();
    seed_db(&db);

    let injectables: HashMap<String, InjectableService> = HashMap::new();
    let sites: Vec<InjectionSite> = Vec::new();

    let created = insert_flow_edges(&db.conn, &sites, &injectables).unwrap();
    assert_eq!(created, 0);

    let count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM flow_edges WHERE edge_type = 'di_binding'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn duplicate_injection_site_not_double_counted() {
    let db = Database::open_in_memory().unwrap();
    let (service_file_id, component_file_id) = seed_db(&db);

    let mut injectables: HashMap<String, InjectableService> = HashMap::new();
    injectables.insert(
        "AuthService".to_string(),
        InjectableService {
            file_id: service_file_id,
            line: 3,
            name: "AuthService".to_string(),
            provided_in_root: false,
        },
    );

    let site = InjectionSite {
        consumer_file_id: component_file_id,
        line: 7,
        injected_type: "AuthService".to_string(),
        consumer_class: "DashboardComponent".to_string(),
    };

    insert_flow_edges(&db.conn, &[site.clone()], &injectables).unwrap();
    let created = insert_flow_edges(&db.conn, &[site], &injectables).unwrap();

    // OR IGNORE should suppress the second insert.
    assert_eq!(created, 0, "Duplicate edge should be ignored");
}

#[test]
fn confidence_is_0_85() {
    let db = Database::open_in_memory().unwrap();
    let (service_file_id, component_file_id) = seed_db(&db);

    let mut injectables: HashMap<String, InjectableService> = HashMap::new();
    injectables.insert(
        "AuthService".to_string(),
        InjectableService {
            file_id: service_file_id,
            line: 3,
            name: "AuthService".to_string(),
            provided_in_root: false,
        },
    );

    let sites = vec![InjectionSite {
        consumer_file_id: component_file_id,
        line: 7,
        injected_type: "AuthService".to_string(),
        consumer_class: "DashboardComponent".to_string(),
    }];

    insert_flow_edges(&db.conn, &sites, &injectables).unwrap();

    let confidence: f64 = db
        .conn
        .query_row(
            "SELECT confidence FROM flow_edges WHERE edge_type = 'di_binding'",
            [],
            |r| r.get(0),
        )
        .unwrap();

    assert!((confidence - 0.85).abs() < f64::EPSILON);
}
