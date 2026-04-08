/// Runtime globals always external for SCSS/CSS.
/// Includes framework mixins/functions that are never project-defined.
pub(crate) const EXTERNALS: &[&str] = &[
    // Bootstrap SCSS mixins/functions
    "media-breakpoint-up", "media-breakpoint-down", "media-breakpoint-between",
    "media-breakpoint-only",
    // Nebular (Angular UI kit) SCSS
    "nb-theme", "nb-install-component", "nb-register-component",
    "nb-rtl", "nb-ltr",
    "nb-get-statuses", "nb-for-themes",
];
