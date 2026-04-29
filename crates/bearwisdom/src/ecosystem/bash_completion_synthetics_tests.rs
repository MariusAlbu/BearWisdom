// Tests for bash_completion_synthetics — in sibling file per
// `feedback_tests_in_separate_files.md`.

use super::*;

#[test]
fn synthesized_file_parallel_vecs_consistent() {
    let pf = synthesize_file();
    assert_eq!(pf.symbols.len(), pf.symbol_origin_languages.len());
    assert_eq!(pf.symbols.len(), pf.symbol_from_snippet.len());
}

#[test]
fn core_completion_helpers_present() {
    let pf = synthesize_file();
    let names: Vec<&str> = pf.symbols.iter().map(|s| s.name.as_str()).collect();

    for expected in [
        "_filedir",
        "_init_completion",
        "_count_args",
        "_command_offset",
        "_completion_loader",
        "_get_comp_words_by_ref",
        "_known_hosts",
        "_pids",
        "_users",
        "_groups",
    ] {
        assert!(names.contains(&expected), "{expected} must be synthesized");
    }
}

#[test]
fn git_completion_helpers_present() {
    let pf = synthesize_file();
    let names: Vec<&str> = pf.symbols.iter().map(|s| s.name.as_str()).collect();

    assert!(names.contains(&"__git_main"));
    assert!(names.contains(&"__git_complete"));
    assert!(names.contains(&"__git_list_all_commands_without_hub"));
    assert!(names.contains(&"__gitcomp"));
    assert!(names.contains(&"__gitcomp_nl"));
    assert!(names.contains(&"__git_refs"));
    assert!(names.contains(&"__git_branches"));
}

#[test]
fn synthesized_file_uses_bash_language() {
    let pf = synthesize_file();
    assert_eq!(pf.language, "bash");
    assert!(pf.path.starts_with("ext:bash-completion-synthetics:"));
}

#[test]
fn locate_roots_skips_non_completion_projects() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    // Plain bash file with no completion markers — should NOT activate.
    std::fs::write(
        tmp.path().join("script.sh"),
        "#!/bin/bash\necho hello\nls -la\n",
    )
    .unwrap();

    let eco = BashCompletionSyntheticsEcosystem;
    let roots = ExternalSourceLocator::locate_roots(&eco, tmp.path());
    assert!(
        roots.is_empty(),
        "non-completion project should not activate the synthetics ecosystem"
    );
}

#[test]
fn locate_roots_activates_on_filedir_reference() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    std::fs::write(
        tmp.path().join("docker.completion.sh"),
        "_docker() { _filedir; }\ncomplete -F _docker docker\n",
    )
    .unwrap();

    let eco = BashCompletionSyntheticsEcosystem;
    let roots = ExternalSourceLocator::locate_roots(&eco, tmp.path());
    assert_eq!(
        roots.len(),
        1,
        "completion project must produce exactly one synthetic dep root"
    );
    assert_eq!(roots[0].module_path, "bash-completion-synthetics");
}

#[test]
fn locate_roots_activates_on_complete_directive() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    std::fs::write(
        tmp.path().join("init.sh"),
        "_my_completion() { COMPREPLY=(); }\ncomplete -F _my_completion mycmd\n",
    )
    .unwrap();

    let eco = BashCompletionSyntheticsEcosystem;
    let roots = ExternalSourceLocator::locate_roots(&eco, tmp.path());
    assert_eq!(roots.len(), 1);
}
