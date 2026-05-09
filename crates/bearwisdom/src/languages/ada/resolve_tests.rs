use super::{spec_for_body, _test_probe_package_of_type};
use crate::indexer::resolve::engine::{SymbolInfo, SymbolLookup};
use crate::types::EdgeKind;
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Minimal SymbolLookup fixture for Ada resolver unit tests
// ---------------------------------------------------------------------------

struct AdaFixture {
    members: HashMap<String, Vec<SymbolInfo>>,
    field_types: HashMap<String, String>,
    types_by_name: HashMap<String, Vec<SymbolInfo>>,
    empty: Vec<SymbolInfo>,
    empty_reexports: Vec<(String, String)>,
    next_id: std::cell::Cell<i64>,
}

impl AdaFixture {
    fn new() -> Self {
        Self {
            members: HashMap::new(),
            field_types: HashMap::new(),
            types_by_name: HashMap::new(),
            empty: Vec::new(),
            empty_reexports: Vec::new(),
            next_id: std::cell::Cell::new(1),
        }
    }

    fn sym(&self, name: &str, qname: &str, kind: &str) -> SymbolInfo {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        SymbolInfo {
            id,
            name: name.to_string(),
            qualified_name: qname.to_string(),
            kind: kind.to_string(),
            visibility: Some("public".to_string()),
            file_path: Arc::from("test.adb"),
            scope_path: None,
            package_id: None,
            signature: None,
        }
    }

    fn with_member(mut self, parent: &str, name: &str, qname: &str, kind: &str) -> Self {
        let sym = self.sym(name, qname, kind);
        self.members.entry(parent.to_string()).or_default().push(sym);
        self
    }
}

impl SymbolLookup for AdaFixture {
    fn by_name(&self, _: &str) -> &[SymbolInfo] { &self.empty }
    fn by_qualified_name(&self, _: &str) -> Option<&SymbolInfo> { None }
    fn members_of(&self, parent: &str) -> &[SymbolInfo] {
        self.members.get(parent).map(|v| v.as_slice()).unwrap_or(&self.empty)
    }
    fn types_by_name(&self, _: &str) -> &[SymbolInfo] { &self.empty }
    fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> { Vec::new() }
    fn has_in_namespace(&self, _: &str) -> bool { false }
    fn in_file(&self, _: &str) -> &[SymbolInfo] { &self.empty }
    fn field_type_name(&self, qname: &str) -> Option<&str> {
        self.field_types.get(qname).map(|s| s.as_str())
    }
    fn return_type_name(&self, _: &str) -> Option<&str> { None }
    fn field_type_args(&self, _: &str) -> Option<&[String]> { None }
    fn generic_params(&self, _: &str) -> Option<&[String]> { None }
    fn reexports_from(&self, _: &str) -> &[(String, String)] { &self.empty_reexports }
    fn is_external_name(&self, _: &str, _: &str) -> bool { false }
}

// ---------------------------------------------------------------------------
// spec_for_body
// ---------------------------------------------------------------------------

#[test]
fn spec_for_body_returns_ads_for_adb() {
    assert_eq!(
        spec_for_body("src/bmp280.adb"),
        Some("src/bmp280.ads".to_string())
    );
}

#[test]
fn spec_for_body_handles_unix_path() {
    assert_eq!(
        spec_for_body("drivers/sensors/bmp280.adb"),
        Some("drivers/sensors/bmp280.ads".to_string())
    );
}

#[test]
fn spec_for_body_handles_windows_separators() {
    assert_eq!(
        spec_for_body("drivers\\sensors\\bmp280.adb"),
        Some("drivers/sensors/bmp280.ads".to_string())
    );
}

#[test]
fn spec_for_body_returns_none_for_ads() {
    assert_eq!(spec_for_body("src/bmp280.ads"), None);
}

#[test]
fn spec_for_body_returns_none_for_other_extension() {
    assert_eq!(spec_for_body("src/main.rs"), None);
    assert_eq!(spec_for_body("src/foo.py"), None);
}

#[test]
fn spec_for_body_bare_filename() {
    assert_eq!(spec_for_body("bmp280.adb"), Some("bmp280.ads".to_string()));
}

// ---------------------------------------------------------------------------
// probe_package_of_type (Fix #3)
// ---------------------------------------------------------------------------

#[test]
fn probe_package_of_type_finds_method_at_package_scope() {
    // `Ada.Containers.Vectors.Vector.Append` — method lives at
    // `Ada.Containers.Vectors.Append`, not nested under the type.
    let fix = AdaFixture::new()
        .with_member("Ada.Containers.Vectors", "Append", "Ada.Containers.Vectors.Append", "function");
    let res = _test_probe_package_of_type(
        "Ada.Containers.Vectors.Vector.Append",
        EdgeKind::Calls,
        &fix,
    );
    assert!(res.is_some(), "expected package-of-type hit");
    assert_eq!(res.unwrap().strategy, "ada_pkg_of_type");
}

#[test]
fn probe_package_of_type_returns_none_for_short_target() {
    // Fewer than 3 segments — no package component to strip.
    let fix = AdaFixture::new();
    assert!(
        _test_probe_package_of_type("Vector.Append", EdgeKind::Calls, &fix).is_none()
    );
}

#[test]
fn probe_package_of_type_returns_none_when_no_match() {
    let fix = AdaFixture::new()
        .with_member("Ada.Containers.Vectors", "Clear", "Ada.Containers.Vectors.Clear", "function");
    // Searching for Append but only Clear exists.
    assert!(
        _test_probe_package_of_type(
            "Ada.Containers.Vectors.Vector.Append",
            EdgeKind::Calls,
            &fix,
        )
        .is_none()
    );
}
