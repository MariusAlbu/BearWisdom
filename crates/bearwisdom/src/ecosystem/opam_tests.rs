use super::*;

#[test]
fn ecosystem_identity() {
    assert_eq!(OpamEcosystem.id(), ID);
    assert_eq!(Ecosystem::languages(&OpamEcosystem), &["ocaml"]);
}

#[test]
fn parse_opam_deps() {
    let content = r#"
depends: [
  "dune" {>= "2.8.0"}
  "ocaml" {>= "4.08.1"}
  "conf-libpcre"
  "cohttp-lwt-unix"
  "core"
  "lwt"
]
"#;
    let deps = parse_opam_depends(content);
    assert!(deps.contains(&"cohttp-lwt-unix".to_string()));
    assert!(deps.contains(&"core".to_string()));
    assert!(deps.contains(&"lwt".to_string()));
    assert!(!deps.contains(&"ocaml".to_string()));
    assert!(!deps.contains(&"conf-libpcre".to_string()));
}

#[test]
fn parse_opam_deps_union_two_files() {
    // sub-package A declares cmdliner; sub-package B declares ctypes.
    // Union must include both without duplicates.
    let file_a = r#"
depends: [
  "ocaml"
  "cmdliner" {>= "1.3.0"}
  "fmt"
]
"#;
    let file_b = r#"
depends: [
  "ocaml"
  "ctypes" {>= "0.19"}
  "ctypes-foreign" {>= "0.18"}
  "fmt"
]
"#;
    let mut union: Vec<String> = Vec::new();
    for dep in parse_opam_depends(file_a) {
        if !union.contains(&dep) { union.push(dep); }
    }
    for dep in parse_opam_depends(file_b) {
        if !union.contains(&dep) { union.push(dep); }
    }
    assert!(union.contains(&"cmdliner".to_string()));
    assert!(union.contains(&"ctypes".to_string()));
    assert!(union.contains(&"ctypes-foreign".to_string()));
    assert!(union.contains(&"fmt".to_string()));
    // fmt appears in both files but must appear only once
    assert_eq!(union.iter().filter(|s| s.as_str() == "fmt").count(), 1);
}

#[allow(dead_code)]
fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
    shared_locator()
}

#[test]
fn ocaml_extracts_open_and_dotted() {
    let mut out = std::collections::HashSet::new();
    extract_ocaml_modules(
        "open Core\nopen Lwt.Infix\nlet x = Cohttp_lwt_unix.Client.get url\n",
        &mut out,
    );
    assert!(out.contains("Core"));
    assert!(out.contains("Lwt"));
    assert!(out.contains("Cohttp_lwt_unix"));
}

#[test]
fn ocaml_module_path_tail_is_lowercase_ml() {
    assert_eq!(ocaml_module_to_path_tail("Core"), Some("core.ml".to_string()));
    assert_eq!(ocaml_module_to_path_tail("Cohttp_lwt_unix"), Some("cohttp_lwt_unix.ml".to_string()));
}
