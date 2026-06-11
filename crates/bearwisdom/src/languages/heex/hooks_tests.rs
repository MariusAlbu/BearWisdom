use super::_test_colocated_view_file as colocated_view_file;

#[test]
fn single_level_context_maps_to_view() {
    assert_eq!(
        colocated_view_file("lib/plausible_web/templates/sso/login_form.html.heex").as_deref(),
        Some("lib/plausible_web/views/sso_view.ex"),
    );
}

#[test]
fn nested_context_mirrors_path_and_names_deepest_dir() {
    // `templates/admin/episode/edit.html.heex` is rendered by
    // `Admin.EpisodeView` at `views/admin/episode_view.ex` — the deepest
    // directory names the view, intervening dirs are mirrored.
    assert_eq!(
        colocated_view_file("lib/changelog_web/templates/admin/episode/edit.html.heex").as_deref(),
        Some("lib/changelog_web/views/admin/episode_view.ex"),
    );
}

#[test]
fn deeply_nested_context_preserves_all_parents() {
    assert_eq!(
        colocated_view_file("lib/web/templates/a/b/c/page.html.heex").as_deref(),
        Some("lib/web/views/a/b/c_view.ex"),
    );
}

#[test]
fn template_directly_under_templates_has_no_view() {
    // No context directory to name a view after.
    assert_eq!(
        colocated_view_file("lib/web/templates/page.html.heex"),
        None,
    );
}

#[test]
fn path_without_templates_segment_is_none() {
    assert_eq!(colocated_view_file("lib/web/live/foo_live.html.heex"), None);
}

#[test]
fn backslash_paths_are_normalized() {
    assert_eq!(
        colocated_view_file(r"lib\changelog_web\templates\admin\episode\index.html.heex")
            .as_deref(),
        Some("lib/changelog_web/views/admin/episode_view.ex"),
    );
}
