// =============================================================================
// scss/primitives.rs — SCSS/CSS primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for SCSS/CSS.
pub(crate) const PRIMITIVES: &[&str] = &[
    // SCSS-specific at-rules / constructs
    "$variable",
    "@mixin", "@include", "@extend",
    "@import", "@use", "@forward",
    "@function", "@return",
    "@if", "@else", "@each", "@for", "@while",
    "@at-root", "@error", "@warn", "@debug", "@content",
    // CSS at-rules
    "@media", "@supports", "@keyframes", "@font-face",
    "@page", "@charset", "@namespace", "@layer",
    "@property", "@container", "@scope",
    "@starting-style",
    // CSS color functions
    "rgb", "rgba", "hsl", "hsla", "hwb",
    "lab", "lch", "oklch", "oklab",
    "color", "color-mix",
    // CSS math / utility functions
    "calc", "min", "max", "clamp", "var", "env",
    "url", "attr", "counter", "counters",
    // gradient functions
    "linear-gradient", "radial-gradient", "conic-gradient",
    "image-set", "element", "cross-fade",
    // transform functions
    "translate", "rotate", "scale", "skew",
    "matrix", "perspective",
    // animation
    "cubic-bezier", "steps",
    // shape / clip-path
    "path", "polygon", "circle", "ellipse", "inset",
    // grid
    "repeat", "minmax", "fit-content",
    // pseudo-class functions
    "not", "is", "where", "has",
    "nth-child", "nth-of-type",
    "first-child", "last-child", "only-child",
    "hover", "focus", "active", "visited", "checked",
    "disabled", "enabled", "required",
    "valid", "invalid", "placeholder-shown",
    "focus-within", "focus-visible",
    // SCSS color manipulation functions
    "lighten", "darken", "saturate", "desaturate",
    "grayscale", "invert", "complement",
    "opacify", "transparentize", "adjust-hue", "mix",
    // SCSS math functions
    "percentage", "round", "ceil", "floor", "abs", "random",
    // SCSS list functions
    "length", "nth", "set-nth", "join", "append", "zip",
    "index", "list-separator", "is-bracketed",
    // SCSS map functions
    "map-get", "map-merge", "map-remove", "map-keys",
    "map-values", "map-has-key",
    // SCSS string functions
    "unquote", "quote", "str-length", "str-insert",
    "str-index", "str-slice", "to-upper-case", "to-lower-case",
    "unique-id",
    // SCSS introspection functions
    "unit", "unitless", "comparable", "type-of", "inspect", "if",
    "feature-exists", "global-variable-exists",
    "variable-exists", "function-exists", "mixin-exists", "content-exists",
    // SCSS selector functions
    "selector-nest", "selector-append", "selector-extend",
    "selector-replace", "selector-unify", "is-superselector",
    "simple-selectors", "selector-parse",
];
