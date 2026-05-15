// =============================================================================
// indexer/resolve/adapters/mailer.rs — mailer-template file-path Consumer
//
// Detects files living under recognised mailer-template roots (`mails/`,
// `emails/`, `templates/email/`, `views/mails/`, …) and surfaces the
// file's basename as the template name. The resolve loop emits a Mailer
// Consumer keyed on that name so Producer-side calls naming the same
// template can pair against it.
// =============================================================================

/// If `path` lives under a recognised mailer template root, return the
/// file's basename (without extension) as the template name. Returns `None`
/// for any other path. The supported roots match the conventions of
/// Nodemailer Express handlebars (`emails/`), NestJS mailer module
/// (`mails/`, `templates/email/`), and Rails-style mailers (`views/mails/`,
/// `app/views/mailers/`). The match is path-segment exact — a file named
/// `MyEmail.tsx` ten levels deep under any non-template ancestor is not
/// matched.
pub(crate) fn mailer_template_name_for_path(path: &str) -> Option<String> {
    const ROOTS: &[&[&str]] = &[
        &["mails"],
        &["emails"],
        &["templates", "email"],
        &["templates", "emails"],
        &["templates", "mail"],
        &["templates", "mails"],
        &["views", "mails"],
        &["views", "mailers"],
        &["app", "views", "mailers"],
    ];
    let norm: Vec<&str> = path.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    if norm.len() < 2 {
        return None;
    }
    let basename = norm.last()?;
    let stem = basename.rsplit_once('.').map(|(s, _)| s).unwrap_or(basename);
    if stem.is_empty() {
        return None;
    }
    let parent_segments = &norm[..norm.len() - 1];
    for root in ROOTS {
        if parent_segments
            .windows(root.len())
            .any(|w| w.iter().zip(root.iter()).all(|(a, b)| a.eq_ignore_ascii_case(b)))
        {
            return Some(stem.to_string());
        }
    }
    None
}
