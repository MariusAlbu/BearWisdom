// =============================================================================
// matlab/keywords.rs — MATLAB reserved words, primitives, and operators
// =============================================================================

/// MATLAB's language-level names: reserved words (`iskeyword`), `classdef`
/// block keywords, function-argument keywords, boolean literals, primitive
/// class names, and operator-method names (the operator subset).
///
/// Built-in *functions* (`zeros`, `plot`, `sprintf`, `regexp`, `eig`, …) are
/// deliberately NOT here. They are library API with real `.m` definitions
/// under `$MATLABROOT/toolbox/`; the `matlab-runtime` ecosystem walks those
/// sources so the resolver binds the calls to real symbols. Listing the
/// function names here would be a hand-maintained library API list — which
/// only fake-resolves and masks a missing toolbox install.
pub(crate) const KEYWORDS: &[&str] = &[
    // reserved words (`iskeyword`)
    "break",
    "case",
    "catch",
    "classdef",
    "continue",
    "else",
    "elseif",
    "end",
    "for",
    "function",
    "global",
    "if",
    "otherwise",
    "parfor",
    "persistent",
    "return",
    "spmd",
    "switch",
    "try",
    "while",
    // classdef block keywords
    "properties",
    "methods",
    "events",
    "enumeration",
    "arguments",
    // function-argument keywords (no `.m` definition — built into the call ABI)
    "nargin",
    "nargout",
    "varargin",
    "varargout",
    // boolean literals
    "true",
    "false",
    // primitive class names
    "char",
    "double",
    "single",
    "int8",
    "int16",
    "int32",
    "int64",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "logical",
    // operator-method names (the operator subset — `+`, `-`, `==`, `[a b]`, …)
    "plus",
    "minus",
    "uminus",
    "uplus",
    "times",
    "mtimes",
    "rdivide",
    "mrdivide",
    "ldivide",
    "mldivide",
    "power",
    "mpower",
    "lt",
    "gt",
    "le",
    "ge",
    "eq",
    "ne",
    "and",
    "or",
    "not",
    "xor",
    "colon",
    "transpose",
    "ctranspose",
    "horzcat",
    "vertcat",
    "subsref",
    "subsasgn",
    "subsindex",
];
