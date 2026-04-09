// =============================================================================
// scss/builtins.rs — SCSS builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Edge-kind / symbol-kind compatibility for SCSS.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "function" | "method"),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        _ => true,
    }
}

/// SCSS / Sass built-in function names — provided by the Sass runtime or the
/// browser's CSS engine, not defined in the project.
///
/// Covers:
/// - Sass built-in modules: sass:color, sass:math, sass:string, sass:list,
///   sass:map, sass:selector, sass:meta
/// - Legacy (non-namespaced) Sass functions
/// - CSS native functions (calc, var, url, …)
/// - CSS transform / filter functions
pub(crate) fn is_scss_builtin(name: &str) -> bool {
    matches!(
        name,
        // ── Sass color functions ──────────────────────────────────────────────
        "rgb"
            | "rgba"
            | "hsl"
            | "hsla"
            | "red"
            | "green"
            | "blue"
            | "mix"
            | "lighten"
            | "darken"
            | "saturate"
            | "desaturate"
            | "grayscale"
            | "complement"
            | "invert"
            | "alpha"
            | "opacity"
            | "opacify"
            | "transparentize"
            | "fade-in"
            | "fade-out"
            | "adjust-hue"
            | "adjust-color"
            | "scale-color"
            | "change-color"
            | "ie-hex-str"
            | "color"
            | "hue"
            | "saturation"
            | "lightness"
            | "whiteness"
            | "blackness"
            // ── Sass string functions ─────────────────────────────────────────
            | "unquote"
            | "quote"
            | "str-length"
            | "str-insert"
            | "str-index"
            | "str-slice"
            | "to-upper-case"
            | "to-lower-case"
            | "unique-id"
            // ── Sass math functions ───────────────────────────────────────────
            | "percentage"
            | "round"
            | "ceil"
            | "floor"
            | "abs"
            | "min"
            | "max"
            | "random"
            | "unit"
            | "unitless"
            | "comparable"
            | "sqrt"
            | "pow"
            | "log"
            | "cos"
            | "sin"
            | "tan"
            | "acos"
            | "asin"
            | "atan"
            | "atan2"
            | "hypot"
            | "clamp"
            // ── Sass list functions ───────────────────────────────────────────
            | "length"
            | "nth"
            | "set-nth"
            | "join"
            | "append"
            | "zip"
            | "index"
            | "list-separator"
            | "is-bracketed"
            // ── Sass map functions ────────────────────────────────────────────
            | "map-get"
            | "map-merge"
            | "map-remove"
            | "map-keys"
            | "map-values"
            | "map-has-key"
            | "keywords"
            // ── Sass selector functions ───────────────────────────────────────
            | "selector-nest"
            | "selector-append"
            | "selector-extend"
            | "selector-replace"
            | "selector-unify"
            | "is-superselector"
            | "simple-selectors"
            | "selector-parse"
            // ── Sass meta / introspection functions ───────────────────────────
            | "type-of"
            | "inspect"
            | "variable-exists"
            | "global-variable-exists"
            | "function-exists"
            | "mixin-exists"
            | "content-exists"
            | "get-function"
            | "call"
            | "if"
            // ── CSS native functions ──────────────────────────────────────────
            | "var"
            | "calc"
            | "env"
            | "url"
            | "attr"
            | "counter"
            | "counters"
            | "format"
            | "local"
            | "linear-gradient"
            | "radial-gradient"
            | "repeating-linear-gradient"
            | "repeating-radial-gradient"
            | "conic-gradient"
            | "image-set"
            | "cross-fade"
            // ── CSS transform functions ───────────────────────────────────────
            | "translate"
            | "translateX"
            | "translateY"
            | "translateZ"
            | "translate3d"
            | "scale"
            | "scaleX"
            | "scaleY"
            | "rotate"
            | "rotateX"
            | "rotateY"
            | "rotateZ"
            | "skew"
            | "skewX"
            | "skewY"
            | "perspective"
            | "matrix"
            | "matrix3d"
            // ── CSS filter functions ──────────────────────────────────────────
            | "blur"
            | "brightness"
            | "contrast"
            | "drop-shadow"
            | "grayscale-filter"
            | "hue-rotate"
            | "invert-filter"
            | "opacity-filter"
            | "saturate-filter"
            | "sepia"
    )
}
