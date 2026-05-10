use super::{spec_for_body, AdaResolver, _test_probe_package_of_type, _test_walk_field_chain};
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, SymbolInfo, SymbolLookup,
};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
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

    fn with_field_type(mut self, qname: &str, ty: &str) -> Self {
        self.field_types.insert(qname.to_string(), ty.to_string());
        self
    }

    fn with_member_sig(mut self, parent: &str, name: &str, qname: &str, kind: &str, sig: &str) -> Self {
        let mut sym = self.sym(name, qname, kind);
        sym.signature = Some(sig.to_string());
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
// Ancestor-package rename for dotted targets (Path 3)
// ---------------------------------------------------------------------------

fn make_extracted_sym(name: &str, qname: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: SymbolKind::Function,
        visibility: None,
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

fn make_extracted_ref(target: &str) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 1,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
    }
}

/// `Alr.Commands.Run.Execute` calls `Trace.Detail`. `Alr` has member
/// `Trace` with `signature = "renames Simple_Logging"`. The resolver must
/// chain through to `Simple_Logging.Detail`.
#[test]
fn ancestor_pkg_rename_resolves_dotted_target() {
    let fix = AdaFixture::new()
        // Ancestor `Alr` exposes `Trace renames Simple_Logging`.
        .with_member_sig("Alr", "Trace", "Alr.Trace", "namespace", "renames Simple_Logging")
        // Simple_Logging has `Detail` as a member.
        .with_member("Simple_Logging", "Detail", "Simple_Logging.Detail", "function");

    let file_ctx = FileContext {
        file_path: "src/alr-commands-run.adb".to_string(),
        language: "ada".to_string(),
        imports: Vec::new(),
        file_namespace: Some("Alr.Commands.Run".to_string()),
    };

    let source_sym = make_extracted_sym("Execute", "Alr.Commands.Run.Execute");
    let extracted = make_extracted_ref("Trace.Detail");
    let ref_ctx = RefContext {
        extracted_ref: &extracted,
        source_symbol: &source_sym,
        scope_chain: vec!["Alr.Commands.Run.Execute".to_string()],
        file_package_id: None,
    };

    let resolver = AdaResolver;
    let res = resolver.resolve(&file_ctx, &ref_ctx, &fix);
    assert!(
        res.is_some(),
        "expected ancestor-pkg rename to resolve Trace.Detail via Simple_Logging.Detail"
    );
    assert_eq!(res.unwrap().strategy, "ada_ancestor_pkg_rename");
}

/// Same pattern but the ancestor is two levels up (`Alr` for a file in
/// `Alr.Commands.Run`). Verifies the depth loop walks past `Alr.Commands`.
#[test]
fn ancestor_pkg_rename_walks_multiple_levels() {
    let fix = AdaFixture::new()
        // Only `Alr` (two levels up) has the rename, not `Alr.Commands`.
        .with_member_sig("Alr", "TTY", "Alr.TTY", "namespace", "renames CLIC.TTY")
        .with_member("CLIC.TTY", "Warn", "CLIC.TTY.Warn", "function");

    let file_ctx = FileContext {
        file_path: "src/alr-commands-run.adb".to_string(),
        language: "ada".to_string(),
        imports: Vec::new(),
        file_namespace: Some("Alr.Commands.Run".to_string()),
    };

    let source_sym = make_extracted_sym("Execute", "Alr.Commands.Run.Execute");
    let extracted = make_extracted_ref("TTY.Warn");
    let ref_ctx = RefContext {
        extracted_ref: &extracted,
        source_symbol: &source_sym,
        scope_chain: Vec::new(),
        file_package_id: None,
    };

    let resolver = AdaResolver;
    let res = resolver.resolve(&file_ctx, &ref_ctx, &fix);
    assert!(
        res.is_some(),
        "expected multi-level ancestor walk to resolve TTY.Warn"
    );
    assert_eq!(res.unwrap().strategy, "ada_ancestor_pkg_rename");
}

/// `Sub_Cmd.Register(...)` where `Sub_Cmd` is a local namespace with
/// `signature = "instantiates CLIC.Subcommand.Instance"`. The resolver must
/// look up `Register` in the members of `CLIC.Subcommand.Instance`.
#[test]
fn local_instantiation_dispatch_resolves_dotted_call() {
    let fix = AdaFixture::new()
        .with_member("CLIC.Subcommand.Instance", "Register", "CLIC.Subcommand.Instance.Register", "function");

    // In-file: Sub_Cmd is a namespace with instantiates signature.
    // The fixture's `in_file` returns `empty`; we need to wire it up.
    // Use a custom fixture that has in_file support.
    struct WithInFile {
        inner: AdaFixture,
        file_syms: Vec<SymbolInfo>,
        empty: Vec<SymbolInfo>,
        empty_reexports: Vec<(String, String)>,
    }
    impl SymbolLookup for WithInFile {
        fn by_name(&self, _: &str) -> &[SymbolInfo] { &self.empty }
        fn by_qualified_name(&self, _: &str) -> Option<&SymbolInfo> { None }
        fn members_of(&self, p: &str) -> &[SymbolInfo] { self.inner.members_of(p) }
        fn types_by_name(&self, _: &str) -> &[SymbolInfo] { &self.empty }
        fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> { Vec::new() }
        fn has_in_namespace(&self, _: &str) -> bool { false }
        fn in_file(&self, _: &str) -> &[SymbolInfo] { &self.file_syms }
        fn field_type_name(&self, _: &str) -> Option<&str> { None }
        fn return_type_name(&self, _: &str) -> Option<&str> { None }
        fn field_type_args(&self, _: &str) -> Option<&[String]> { None }
        fn generic_params(&self, _: &str) -> Option<&[String]> { None }
        fn reexports_from(&self, _: &str) -> &[(String, String)] { &self.empty_reexports }
        fn is_external_name(&self, _: &str, _: &str) -> bool { false }
    }

    let id = std::cell::Cell::new(10i64);
    let next = || { let v = id.get(); id.set(v + 1); v };
    let sub_cmd_sym = SymbolInfo {
        id: next(),
        name: "Sub_Cmd".to_string(),
        qualified_name: "Alr.Commands.Sub_Cmd".to_string(),
        kind: "namespace".to_string(),
        visibility: Some("public".to_string()),
        file_path: Arc::from("src/alr-commands.ads"),
        scope_path: None,
        package_id: None,
        signature: Some("instantiates CLIC.Subcommand.Instance".to_string()),
    };

    let wif = WithInFile {
        inner: fix,
        file_syms: vec![sub_cmd_sym],
        empty: Vec::new(),
        empty_reexports: Vec::new(),
    };

    let file_ctx = FileContext {
        file_path: "src/alr-commands.ads".to_string(),
        language: "ada".to_string(),
        imports: Vec::new(),
        file_namespace: Some("Alr.Commands".to_string()),
    };

    let source_sym = make_extracted_sym("Alr_Main", "Alr.Commands.Alr_Main");
    let extracted = make_extracted_ref("Sub_Cmd.Register");
    let ref_ctx = RefContext {
        extracted_ref: &extracted,
        source_symbol: &source_sym,
        scope_chain: Vec::new(),
        file_package_id: None,
    };

    let resolver = AdaResolver;
    let res = resolver.resolve(&file_ctx, &ref_ctx, &wif);
    assert!(
        res.is_some(),
        "expected local instantiation to resolve Sub_Cmd.Register"
    );
    assert_eq!(res.unwrap().strategy, "ada_local_instantiation");
}

/// Partial qualification: `Alire.Index.Search` calls `Utils.TTY.Name` where
/// the full path is `Alire.Utils.TTY.Name`. The resolver must expand the
/// partial call by prepending the common ancestor prefix `Alire.`.
#[test]
fn partial_qualification_expands_ancestor_prefix() {
    let fix = AdaFixture::new()
        .with_member("Alire.Utils.TTY", "Name", "Alire.Utils.TTY.Name", "function");

    let file_ctx = FileContext {
        file_path: "src/alire-index-search.adb".to_string(),
        language: "ada".to_string(),
        imports: Vec::new(),
        file_namespace: Some("Alire.Index.Search".to_string()),
    };

    let source_sym = make_extracted_sym("Print_Dependents", "Alire.Index.Search.Print_Dependents");
    let extracted = make_extracted_ref("Utils.TTY.Name");
    let ref_ctx = RefContext {
        extracted_ref: &extracted,
        source_symbol: &source_sym,
        scope_chain: Vec::new(),
        file_package_id: None,
    };

    let res = AdaResolver.resolve(&file_ctx, &ref_ctx, &fix);
    assert!(
        res.is_some(),
        "expected partial-qualification expansion to resolve Utils.TTY.Name → Alire.Utils.TTY.Name"
    );
    assert_eq!(res.unwrap().strategy, "ada_partial_qualification");
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

// ---------------------------------------------------------------------------
// walk_field_chain (Fix #4)
// ---------------------------------------------------------------------------

#[test]
fn walk_field_chain_single_hop_finds_method() {
    // `This.Port.Mem_Read`: Device.Port field has type Port_Type;
    // Mem_Read lives as a member of Port_Type.
    let fix = AdaFixture::new()
        .with_field_type("Drivers.Device.Port", "Drivers.Port_Type")
        .with_member("Drivers.Port_Type", "Mem_Read", "Drivers.Port_Type.Mem_Read", "function");
    let res = _test_walk_field_chain(
        "Drivers.Device",
        &["Port", "Mem_Read"],
        EdgeKind::Calls,
        &fix,
    );
    assert!(res.is_some(), "expected field-chain hit");
}

#[test]
fn walk_field_chain_returns_none_when_field_type_unknown() {
    // No field_type registered — chain must give up at the first hop.
    let fix = AdaFixture::new();
    assert!(
        _test_walk_field_chain("Device", &["Port", "Mem_Read"], EdgeKind::Calls, &fix).is_none()
    );
}

#[test]
fn walk_field_chain_respects_depth_cap() {
    // 7 segments exceeds the cap of 6 — must return None immediately.
    let fix = AdaFixture::new();
    let segs = ["A", "B", "C", "D", "E", "F", "G"];
    assert!(
        _test_walk_field_chain("Root", &segs, EdgeKind::Calls, &fix).is_none()
    );
}

// ---------------------------------------------------------------------------
// Ancestor-package instantiation dispatch (Fix #6)
// ---------------------------------------------------------------------------

/// `Version_Outcomes.Outcome_Failure` called from a file in
/// `Alire.Origins.Deployers.System.Apt`. The instantiation
/// `Version_Outcomes` is declared in the parent package
/// `Alire.Origins.Deployers.System` with a partial generic name
/// (`Outcomes.Definite`, not the full `Alire.Outcomes.Definite`).
/// The resolver must find it via ancestor walk and expand the partial name.
#[test]
fn ancestor_pkg_instantiation_dispatch_resolves_child_call() {
    use std::sync::Arc as A;
    // Custom fixture that supports both by_name and members_of.
    struct WithByName {
        members: HashMap<String, Vec<SymbolInfo>>,
        by_name_map: HashMap<String, Vec<SymbolInfo>>,
        empty: Vec<SymbolInfo>,
        empty_reexports: Vec<(String, String)>,
    }
    impl SymbolLookup for WithByName {
        fn by_name(&self, name: &str) -> &[SymbolInfo] {
            self.by_name_map.get(name).map(|v| v.as_slice()).unwrap_or(&self.empty)
        }
        fn by_qualified_name(&self, _: &str) -> Option<&SymbolInfo> { None }
        fn members_of(&self, p: &str) -> &[SymbolInfo] {
            self.members.get(p).map(|v| v.as_slice()).unwrap_or(&self.empty)
        }
        fn types_by_name(&self, _: &str) -> &[SymbolInfo] { &self.empty }
        fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> { Vec::new() }
        fn has_in_namespace(&self, _: &str) -> bool { false }
        fn in_file(&self, _: &str) -> &[SymbolInfo] { &self.empty }
        fn field_type_name(&self, _: &str) -> Option<&str> { None }
        fn return_type_name(&self, _: &str) -> Option<&str> { None }
        fn field_type_args(&self, _: &str) -> Option<&[String]> { None }
        fn generic_params(&self, _: &str) -> Option<&[String]> { None }
        fn reexports_from(&self, _: &str) -> &[(String, String)] { &self.empty_reexports }
        fn is_external_name(&self, _: &str, _: &str) -> bool { false }
    }

    let mut members: HashMap<String, Vec<SymbolInfo>> = HashMap::new();
    let mut by_name_map: HashMap<String, Vec<SymbolInfo>> = HashMap::new();
    let mut id = 1i64;

    // Parent package has Version_Outcomes as a namespace with PARTIAL generic name.
    let vo = SymbolInfo {
        id: { id += 1; id },
        name: "Version_Outcomes".to_string(),
        qualified_name: "Alire.Origins.Deployers.System.Version_Outcomes".to_string(),
        kind: "namespace".to_string(),
        visibility: Some("public".to_string()),
        file_path: A::from("src/alire-origins-deployers-system.ads"),
        scope_path: None,
        package_id: None,
        signature: Some("instantiates Outcomes.Definite".to_string()), // partial name, as in real DB
    };
    members.entry("Alire.Origins.Deployers.System".to_string()).or_default().push(vo);

    // The fully-qualified generic namespace is indexable by name "Definite".
    let definite_ns = SymbolInfo {
        id: { id += 1; id },
        name: "Definite".to_string(),
        qualified_name: "Alire.Outcomes.Definite".to_string(),
        kind: "namespace".to_string(),
        visibility: Some("public".to_string()),
        file_path: A::from("src/alire-outcomes-definite.ads"),
        scope_path: None,
        package_id: None,
        signature: None,
    };
    by_name_map.entry("Definite".to_string()).or_default().push(definite_ns);

    // Outcome_Failure lives as a member of Alire.Outcomes.Definite.
    let outcome_failure = SymbolInfo {
        id: { id += 1; id },
        name: "Outcome_Failure".to_string(),
        qualified_name: "Alire.Outcomes.Definite.Outcome_Failure".to_string(),
        kind: "function".to_string(),
        visibility: Some("public".to_string()),
        file_path: A::from("src/alire-outcomes-definite.ads"),
        scope_path: None,
        package_id: None,
        signature: None,
    };
    members.entry("Alire.Outcomes.Definite".to_string()).or_default().push(outcome_failure);

    let fix = WithByName { members, by_name_map, empty: Vec::new(), empty_reexports: Vec::new() };

    let file_ctx = FileContext {
        file_path: "src/alire-origins-deployers-system-apt.adb".to_string(),
        language: "ada".to_string(),
        imports: Vec::new(),
        file_namespace: Some("Alire.Origins.Deployers.System.Apt".to_string()),
    };

    let source_sym = make_extracted_sym("Detect", "Alire.Origins.Deployers.System.Apt.Detect");
    let extracted = make_extracted_ref("Version_Outcomes.Outcome_Failure");
    let ref_ctx = RefContext {
        extracted_ref: &extracted,
        source_symbol: &source_sym,
        scope_chain: Vec::new(),
        file_package_id: None,
    };

    let res = AdaResolver.resolve(&file_ctx, &ref_ctx, &fix);
    assert!(
        res.is_some(),
        "expected ancestor-package instantiation to resolve Version_Outcomes.Outcome_Failure"
    );
    assert_eq!(res.unwrap().strategy, "ada_local_instantiation");
}

// ---------------------------------------------------------------------------
// Use'd-package variable-type dispatch
//
// `use STM32.Board;` brings `Display` (typed Frame_Buffer) into scope.
// `Display.Hidden_Buffer` should resolve via variable-type dispatch against
// the imported package's variable, not just in_file variables.
// ---------------------------------------------------------------------------

#[test]
fn used_package_variable_type_dispatch_resolves_dotted_call() {
    use std::sync::Arc as A;

    struct WithInFile {
        members: HashMap<String, Vec<SymbolInfo>>,
        in_file_syms: Vec<SymbolInfo>,
        empty: Vec<SymbolInfo>,
        empty_reexports: Vec<(String, String)>,
    }

    impl SymbolLookup for WithInFile {
        fn by_name(&self, _: &str) -> &[SymbolInfo] { &self.empty }
        fn by_qualified_name(&self, _: &str) -> Option<&SymbolInfo> { None }
        fn members_of(&self, p: &str) -> &[SymbolInfo] {
            self.members.get(p).map(|v| v.as_slice()).unwrap_or(&self.empty)
        }
        fn types_by_name(&self, _: &str) -> &[SymbolInfo] { &self.empty }
        fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> { Vec::new() }
        fn has_in_namespace(&self, _: &str) -> bool { false }
        fn in_file(&self, _: &str) -> &[SymbolInfo] { &self.in_file_syms }
        fn field_type_name(&self, _: &str) -> Option<&str> { None }
        fn return_type_name(&self, _: &str) -> Option<&str> { None }
        fn field_type_args(&self, _: &str) -> Option<&[String]> { None }
        fn generic_params(&self, _: &str) -> Option<&[String]> { None }
        fn reexports_from(&self, _: &str) -> &[(String, String)] { &self.empty_reexports }
        fn is_external_name(&self, _: &str, _: &str) -> bool { false }
    }

    let mut members: HashMap<String, Vec<SymbolInfo>> = HashMap::new();
    let mut id = 1i64;

    // STM32.Board has `Display` as a package-level variable typed `Frame_Buffer`.
    let display_var = SymbolInfo {
        id: { id += 1; id },
        name: "Display".to_string(),
        qualified_name: "STM32.Board.Display".to_string(),
        kind: "variable".to_string(),
        visibility: Some("public".to_string()),
        file_path: A::from("boards/stm32/src/stm32-board.ads"),
        scope_path: None,
        package_id: None,
        signature: Some("type: Framebuffer_OTM8009A.Frame_Buffer".to_string()),
    };
    members.entry("STM32.Board".to_string()).or_default().push(display_var);

    // Framebuffer_OTM8009A has `Hidden_Buffer` as a function.
    let hidden_buf = SymbolInfo {
        id: { id += 1; id },
        name: "Hidden_Buffer".to_string(),
        qualified_name: "Framebuffer_OTM8009A.Hidden_Buffer".to_string(),
        kind: "function".to_string(),
        visibility: Some("public".to_string()),
        file_path: A::from("src/framebuffer_otm8009a.ads"),
        scope_path: None,
        package_id: None,
        signature: None,
    };
    members.entry("Framebuffer_OTM8009A.Frame_Buffer".to_string()).or_default().push(hidden_buf);

    let fix = WithInFile {
        members,
        in_file_syms: Vec::new(), // no local variables — must come from use'd package
        empty: Vec::new(),
        empty_reexports: Vec::new(),
    };

    let file_ctx = FileContext {
        file_path: "examples/shared/common/gui/lcd_std_out.adb".to_string(),
        language: "ada".to_string(),
        imports: vec![
            ImportEntry {
                imported_name: "STM32.Board".to_string(),
                module_path: Some("STM32.Board".to_string()),
                alias: None,
                is_wildcard: true, // `use STM32.Board;`
            },
        ],
        file_namespace: Some("LCD_Std_Out".to_string()),
    };

    let source_sym = make_extracted_sym("Clear_Screen", "LCD_Std_Out.Clear_Screen");
    let extracted = make_extracted_ref("Display.Hidden_Buffer");
    let ref_ctx = RefContext {
        extracted_ref: &extracted,
        source_symbol: &source_sym,
        scope_chain: Vec::new(),
        file_package_id: None,
    };

    let res = AdaResolver.resolve(&file_ctx, &ref_ctx, &fix);
    assert!(
        res.is_some(),
        "expected use'd-package variable type dispatch to resolve Display.Hidden_Buffer"
    );
}

