use std::collections::HashSet;

/// Runtime globals always external for SCSS/CSS.
/// Includes framework mixins/functions that are never project-defined.
pub(crate) const EXTERNALS: &[&str] = &[
    // ── Bootstrap SCSS mixins/functions ──────────────────────────────────────
    "media-breakpoint-up",
    "media-breakpoint-down",
    "media-breakpoint-between",
    "media-breakpoint-only",
    "make-container",
    "make-row",
    "make-col",
    "make-col-offset",
    "clearfix",
    "box-shadow",
    "border-radius",
    "transition",
    "caret",
    "reset-list",
    "list-unstyled",
    "sr-only",
    "text-truncate",
    "text-hide",
    "hover",
    "hover-focus",
    "hover-focus-active",
    "plain-hover-focus",
    // ── Tailwind CSS SCSS integration ─────────────────────────────────────────
    "tailwind",
    "apply",
    "screen",
    "layer",
    "config",
    "theme",
    // ── Nebular (Angular UI kit) SCSS ─────────────────────────────────────────
    "nb-theme",
    "nb-install-component",
    "nb-register-component",
    "nb-rtl",
    "nb-ltr",
    "nb-get-statuses",
    "nb-for-themes",
    // ── Angular Material SCSS ─────────────────────────────────────────────────
    "mat-typography-level",
    "mat-typography-config",
    "mat-base-typography",
    "mat-core",
    "mat-palette",
    "mat-light-theme",
    "mat-dark-theme",
    "angular-material-theme",
    "angular-material-typography",
    "angular-material-color",
    // ── Compass mixins ────────────────────────────────────────────────────────
    "inline-block",
    "pie-clearfix",
    "border-box",
    "opacity",
    "box-sizing",
    "background-size",
    "background-clip",
    "border-image",
    "columns",
    "column-count",
    "column-gap",
    "column-rule",
    "flex-box",
    "flexbox",
    "order",
    "flex",
    "flex-direction",
    "flex-wrap",
    "flex-flow",
    "justify-content",
    "align-items",
    "align-self",
    "align-content",
    // ── Foundation SCSS ───────────────────────────────────────────────────────
    "breakpoint",
    "xy-grid-container",
    "xy-grid",
    "xy-cell",
    "flex-grid-row",
    "flex-grid-column",
    // ── Susy grid ─────────────────────────────────────────────────────────────
    "susy-use",
    "with-layout",
    "span",
    "gutter",
    "container",
    "bleed",
    // ── Bourbon mixins ────────────────────────────────────────────────────────
    "ellipsis",
    "retina-image",
    "triangle",
    "word-break",
    "hide-text",
    "position",
    "size",
    "clearfix",
];

/// Dependency-gated framework globals for SCSS.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    // Bootstrap
    for dep in ["bootstrap", "@bootstrap/scss", "bootstrap-scss"] {
        if deps.contains(dep) {
            globals.extend(BOOTSTRAP_GLOBALS);
            break;
        }
    }

    // Angular Material
    for dep in ["@angular/material", "@angular/material-experimental"] {
        if deps.contains(dep) {
            globals.extend(ANGULAR_MATERIAL_GLOBALS);
            break;
        }
    }

    globals
}

const BOOTSTRAP_GLOBALS: &[&str] = &[
    "media-breakpoint-up",
    "media-breakpoint-down",
    "media-breakpoint-between",
    "media-breakpoint-only",
    "make-container",
    "make-row",
    "make-col",
    "make-col-offset",
];

const ANGULAR_MATERIAL_GLOBALS: &[&str] = &[
    "mat-core",
    "mat-palette",
    "mat-light-theme",
    "mat-dark-theme",
    "angular-material-theme",
    "angular-material-typography",
    "angular-material-color",
];
