// =============================================================================
// nix/externals.rs — Nix / nixpkgs external symbol list
// =============================================================================
//
// Bare names that appear as call/ref targets in real nixpkgs code but will
// never resolve to a project-defined symbol.  These complement the prefix
// checks in `builtins::is_nix_builtin` for names that are used as bare
// identifiers after `with lib;` or `with pkgs;` patterns.

/// Nix builtins and nixpkgs `lib.*` names that are always external.
pub(crate) const EXTERNALS: &[&str] = &[
    // -----------------------------------------------------------------------
    // Nix language primitives
    // -----------------------------------------------------------------------
    "import", "builtins", "derivation", "abort", "throw",
    "toString", "toJSON", "fromJSON", "toPath", "storePath",
    "baseNameOf", "dirOf", "isNull", "isBool", "isInt", "isFloat",
    "isList", "isAttrs", "isFunction", "isString", "isPath", "typeOf",
    "map", "filter", "foldl'", "foldl", "foldr",
    "head", "tail", "length", "elem", "elemAt",
    "concatLists", "concatMap", "listToAttrs",
    "attrNames", "attrValues", "hasAttr", "getAttr", "removeAttrs",
    "mapAttrs", "intersectAttrs", "functionArgs",
    "readFile", "readDir", "pathExists", "path",
    "fetchurl", "fetchTarball", "fetchGit", "fetchFromGitHub",
    "currentSystem", "currentTime", "nixPath", "storeDir", "nixVersion",
    "tryEval", "seq", "deepSeq", "trace",
    // -----------------------------------------------------------------------
    // Nixpkgs package set / callPackage
    // -----------------------------------------------------------------------
    "pkgs", "lib", "config", "stdenv", "self", "super", "prev", "final",
    "callPackage", "callPackageWith", "mkDerivation", "makeOverridable",
    // -----------------------------------------------------------------------
    // lib.modules
    // -----------------------------------------------------------------------
    "mkIf", "mkOption", "mkDefault", "mkForce", "mkMerge", "mkOverride",
    "mkEnableOption", "mkPackageOption",
    "mkRemovedOptionModule", "mkRenamedOptionModule", "mkAliasOptionModule",
    "recursiveUpdate",
    // lib.options — types.* used bare after `with lib;`
    "types",
    // -----------------------------------------------------------------------
    // lib.strings
    // -----------------------------------------------------------------------
    "optionalString", "optionals", "optional",
    "concatMapStrings", "concatStringsSep", "concatStrings",
    "removeSuffix", "removePrefix", "hasPrefix", "hasSuffix",
    "splitString", "toLower", "toUpper",
    "escapeShellArg", "escapeShellArgs",
    "floatToString", "intToString",
    // -----------------------------------------------------------------------
    // lib.attrsets
    // -----------------------------------------------------------------------
    "filterAttrs", "mapAttrsToList", "nameValuePair",
    // -----------------------------------------------------------------------
    // lib.trivial / lib.fixed-points
    // -----------------------------------------------------------------------
    "flip", "const", "id", "fix", "extends",
    // -----------------------------------------------------------------------
    // lib.packages / lib.meta
    // -----------------------------------------------------------------------
    "getExe", "getExe'", "getBin", "getDev", "getLib", "getOutput",
    "makeSearchPath", "makeBinPath", "makeLibraryPath", "makeIncludePath",
    // -----------------------------------------------------------------------
    // Common single-letter locals the extractor may surface
    // -----------------------------------------------------------------------
    "_", "n", "v", "k", "x", "f", "s",
];
