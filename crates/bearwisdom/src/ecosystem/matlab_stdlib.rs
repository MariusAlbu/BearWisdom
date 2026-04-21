// =============================================================================
// ecosystem/matlab_stdlib.rs — MATLAB built-in functions (synthetic stdlib)
//
// MATLAB ships ~1000+ built-ins as part of the runtime; none live as source
// on disk in user projects. This ecosystem synthesises a curated set of ~400
// top-tier built-ins into a single virtual file so the resolver can turn
// bare calls like `zeros(3,3)`, `plot(x,y)`, or `sprintf(...)` into real
// edges instead of unresolved references.
//
// Activation: any `.m` file in the project (`LanguagePresent("matlab")`).
// walk_root: returns empty — synthetic only, no disk walk.
// uses_demand_driven_parse: true — build_symbol_index populates the index
//   from the curated const list; the indexer skips any eager walk.
// =============================================================================

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;
use std::path::Path;
use std::sync::Arc;

pub const ID: EcosystemId = EcosystemId::new("matlab-stdlib");
const LEGACY_ECOSYSTEM_TAG: &str = "matlab-stdlib";
const LANGUAGES: &[&str] = &["matlab"];

/// Synthetic file path used for all emitted symbols.
const SYNTHETIC_PATH: &str = "ext:matlab-stdlib:builtins.m";

// =============================================================================
// Curated built-in list (~400 names, one entry each, no overloads)
// =============================================================================

/// Checks whether `name` is one of MATLAB's ~560 curated built-in functions.
/// Used by the MATLAB resolver to classify bare builtin calls as `matlab-stdlib`
/// external without duplicating the name list. O(1) via a once-built set.
pub fn is_builtin(name: &str) -> bool {
    use std::collections::HashSet;
    use std::sync::OnceLock;
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| BUILTINS.iter().copied().collect())
        .contains(name)
}

const BUILTINS: &[&str] = &[
    // -------------------------------------------------------------------------
    // Array creation & manipulation
    // -------------------------------------------------------------------------
    "zeros",
    "ones",
    "eye",
    "rand",
    "randn",
    "randi",
    "linspace",
    "logspace",
    "colon",
    "reshape",
    "repmat",
    "repelem",
    "cat",
    "horzcat",
    "vertcat",
    "blkdiag",
    "diag",
    "tril",
    "triu",
    "flipud",
    "rot90",
    "transpose",
    "ctranspose",
    "permute",
    "ipermute",
    "squeeze",
    "shiftdim",
    "circshift",
    "sub2ind",
    "ind2sub",
    "meshgrid",
    "ndgrid",
    "accumarray",
    "sparse",
    "full",
    "speye",
    "sprand",
    "sprandn",
    // -------------------------------------------------------------------------
    // Size / shape / type queries
    // -------------------------------------------------------------------------
    "size",
    "length",
    "numel",
    "ndims",
    "rows",
    "columns",
    "height",
    "width",
    "isempty",
    "isvector",
    "isscalar",
    "ismatrix",
    "issquare",
    "issparse",
    "isnumeric",
    "isinteger",
    "isfloat",
    "isreal",
    "ischar",
    "isstring",
    "iscell",
    "isstruct",
    "islogical",
    "isnan",
    "isinf",
    "isfinite",
    "isa",
    "class",
    "typecast",
    "cast",
    "int8",
    "int16",
    "int32",
    "int64",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "single",
    "double",
    "logical",
    "char",
    "string",
    "cell",
    "struct",
    // -------------------------------------------------------------------------
    // Elementary math
    // -------------------------------------------------------------------------
    "abs",
    "sign",
    "ceil",
    "floor",
    "round",
    "fix",
    "mod",
    "rem",
    "sqrt",
    "exp",
    "expm1",
    "log",
    "log2",
    "log10",
    "log1p",
    "pow2",
    "nextpow2",
    "hypot",
    "gcd",
    "lcm",
    "factorial",
    "nchoosek",
    "primes",
    "isprime",
    "factor",
    "realmin",
    "realmax",
    "eps",
    "inf",
    "nan",
    "pi",
    // -------------------------------------------------------------------------
    // Trigonometry
    // -------------------------------------------------------------------------
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "sind",
    "cosd",
    "tand",
    "asind",
    "acosd",
    "atand",
    "sinh",
    "cosh",
    "tanh",
    "asinh",
    "acosh",
    "atanh",
    "deg2rad",
    "rad2deg",
    // -------------------------------------------------------------------------
    // Aggregate / reduction
    // -------------------------------------------------------------------------
    "sum",
    "prod",
    "cumsum",
    "cumprod",
    "mean",
    "median",
    "mode",
    "std",
    "var",
    "min",
    "max",
    "bounds",
    "range",
    "diff",
    "gradient",
    "movmean",
    "movmedian",
    "movstd",
    "movvar",
    "movmin",
    "movmax",
    "movsum",
    "movprod",
    "histcounts",
    "histcounts2",
    "hist",
    "histc",
    // -------------------------------------------------------------------------
    // Search / sort / set
    // -------------------------------------------------------------------------
    "sort",
    "sortrows",
    "issorted",
    "find",
    "any",
    "all",
    "unique",
    "union",
    "intersect",
    "setdiff",
    "ismember",
    "lookup",
    "searchsorted",
    // -------------------------------------------------------------------------
    // Linear algebra
    // -------------------------------------------------------------------------
    "inv",
    "det",
    "rank",
    "trace",
    "norm",
    "normest",
    "cond",
    "condest",
    "eig",
    "eigs",
    "svd",
    "svds",
    "lu",
    "qr",
    "chol",
    "ldl",
    "schur",
    "rsf2csf",
    "hess",
    "balance",
    "cdf2rdf",
    "linsolve",
    "lsqminnorm",
    "pinv",
    "null",
    "orth",
    "cross",
    "dot",
    "kron",
    "expm",
    "logm",
    "sqrtm",
    "funm",
    "mldivide",
    "mrdivide",
    // -------------------------------------------------------------------------
    // Polynomials
    // -------------------------------------------------------------------------
    "poly",
    "polyval",
    "polyfit",
    "polyint",
    "polyder",
    "conv",
    "deconv",
    "roots",
    "residue",
    // -------------------------------------------------------------------------
    // Signal / FFT
    // -------------------------------------------------------------------------
    "fft",
    "ifft",
    "fft2",
    "ifft2",
    "fftn",
    "ifftn",
    "fftshift",
    "ifftshift",
    "fftfreq",
    "filter",
    "filter2",
    "conv2",
    "xcorr",
    "xcov",
    "interpft",
    // -------------------------------------------------------------------------
    // Interpolation / integration
    // -------------------------------------------------------------------------
    "interp1",
    "interp2",
    "interp3",
    "interpn",
    "spline",
    "pchip",
    "mkpp",
    "ppval",
    "trapz",
    "cumtrapz",
    "quad",
    "integral",
    "integral2",
    "integral3",
    "ode45",
    "ode23",
    "ode113",
    "ode15s",
    "ode23s",
    "ode23t",
    "ode23tb",
    "dde23",
    // -------------------------------------------------------------------------
    // Optimisation / root-finding
    // -------------------------------------------------------------------------
    "fzero",
    "fminbnd",
    "fminsearch",
    "fminunc",
    "fmincon",
    "lsqnonlin",
    "lsqcurvefit",
    "linprog",
    "quadprog",
    // -------------------------------------------------------------------------
    // Cell / struct operations
    // -------------------------------------------------------------------------
    "fieldnames",
    "isfield",
    "rmfield",
    "orderfields",
    "setfield",
    "getfield",
    "cellfun",
    "structfun",
    "arrayfun",
    "cell2mat",
    "mat2cell",
    "num2cell",
    "cell2struct",
    "struct2cell",
    // -------------------------------------------------------------------------
    // String / character operations
    // -------------------------------------------------------------------------
    "sprintf",
    "fprintf",
    "printf",
    "display",
    "num2str",
    "str2num",
    "str2double",
    "mat2str",
    "int2str",
    "strtrim",
    "strsplit",
    "strjoin",
    "strcat",
    "strrep",
    "strcmp",
    "strcmpi",
    "strncmp",
    "strncmpi",
    "strfind",
    "strtok",
    "fliplr",
    "upper",
    "lower",
    "deblank",
    "blanks",
    "regexp",
    "regexpi",
    "regexprep",
    "textscan",
    "sscanf",
    "num2hex",
    "hex2num",
    "hex2dec",
    "dec2hex",
    "dec2bin",
    "bin2dec",
    "base2dec",
    "dec2base",
    // -------------------------------------------------------------------------
    // I/O
    // -------------------------------------------------------------------------
    "fopen",
    "fclose",
    "fread",
    "fwrite",
    "fgetl",
    "fgets",
    "feof",
    "ferror",
    "frewind",
    "fseek",
    "ftell",
    "fflush",
    "fscanf",
    "load",
    "save",
    "csvread",
    "csvwrite",
    "dlmread",
    "dlmwrite",
    "xlsread",
    "xlswrite",
    "readtable",
    "writetable",
    "readmatrix",
    "writematrix",
    "readcell",
    "writecell",
    "readstruct",
    "writestruct",
    "importdata",
    "uiimport",
    "dir",
    "ls",
    "pwd",
    "cd",
    "mkdir",
    "rmdir",
    "copyfile",
    "movefile",
    "delete",
    "exist",
    "which",
    "fileparts",
    "fullfile",
    "tempname",
    "tempdir",
    // -------------------------------------------------------------------------
    // Plotting (2-D)
    // -------------------------------------------------------------------------
    "plot",
    "plot3",
    "fplot",
    "fplot3",
    "loglog",
    "semilogx",
    "semilogy",
    "polar",
    "scatter",
    "scatter3",
    "bar",
    "barh",
    "bar3",
    "bar3h",
    "histogram",
    "histogram2",
    "pie",
    "pie3",
    "area",
    "stairs",
    "stem",
    "stem3",
    "errorbar",
    "quiver",
    "quiver3",
    "compass",
    "feather",
    "comet",
    "comet3",
    "plotmatrix",
    "pareto",
    "qqplot",
    // -------------------------------------------------------------------------
    // Plotting (3-D / surface)
    // -------------------------------------------------------------------------
    "surf",
    "surfc",
    "surfl",
    "mesh",
    "meshc",
    "meshz",
    "contour",
    "contourf",
    "contour3",
    "contourc",
    "waterfall",
    "ribbon",
    "pcolor",
    "fill",
    "fill3",
    "patch",
    "slice",
    "isosurface",
    "isonormals",
    "isocaps",
    "streamline",
    "streamtube",
    "streamribbon",
    // -------------------------------------------------------------------------
    // Figure / axes management
    // -------------------------------------------------------------------------
    "figure",
    "axes",
    "subplot",
    "hold",
    "grid",
    "box",
    "axis",
    "xlim",
    "ylim",
    "zlim",
    "xlabel",
    "ylabel",
    "zlabel",
    "title",
    "legend",
    "colorbar",
    "colormap",
    "caxis",
    "clim",
    "text",
    "annotation",
    "gca",
    "gcf",
    "gco",
    "cla",
    "clf",
    "close",
    "drawnow",
    "shading",
    "lighting",
    "material",
    "camlight",
    "view",
    "rotate3d",
    "zoom",
    "pan",
    "datacursormode",
    "saveas",
    "print",
    "exportgraphics",
    "copygraphics",
    // -------------------------------------------------------------------------
    // Image processing
    // -------------------------------------------------------------------------
    "imagesc",
    "imshow",
    "imwrite",
    "imread",
    "image",
    "rgb2gray",
    "rgb2hsv",
    "hsv2rgb",
    "ind2rgb",
    "gray2ind",
    "imresize",
    "imrotate",
    "imcrop",
    "imflip",
    "imadjust",
    "histeq",
    "edge",
    "imfilter",
    // -------------------------------------------------------------------------
    // Flow control / diagnostics
    // -------------------------------------------------------------------------
    "error",
    "warning",
    "assert",
    "disp",
    "input",
    "keyboard",
    "pause",
    "echo",
    "more",
    "clc",
    "clear",
    "clearvars",
    "who",
    "whos",
    "workspace",
    "nargin",
    "nargout",
    "nargchk",
    "narginchk",
    "nargoutchk",
    "inputParser",
    "validateattributes",
    "validatestring",
    "mustBeNumeric",
    "mustBePositive",
    "mustBeNonzero",
    "mustBeInteger",
    "mustBeNonNegative",
    "mustBeNonNan",
    "mustBeFinite",
    "mustBeMember",
    "mustBeNonempty",
    "mustBeLogical",
    "mustBeText",
    // -------------------------------------------------------------------------
    // OOP / reflection
    // -------------------------------------------------------------------------
    "methods",
    "properties",
    "events",
    "metaclass",
    "superclasses",
    "enumeration",
    // -------------------------------------------------------------------------
    // Miscellaneous
    // -------------------------------------------------------------------------
    "tic",
    "toc",
    "clock",
    "cputime",
    "now",
    "datetime",
    "duration",
    "calendarDuration",
    "isdatetime",
    "isduration",
    "datevec",
    "datenum",
    "datestr",
    "calendar",
    "deal",
    "feval",
    "eval",
    "builtin",
    "func2str",
    "str2func",
    "spfun",
    "bsxfun",
    "version",
    "ver",
    "computer",
    "ispc",
    "ismac",
    "isunix",
    "getenv",
    "setenv",
    "system",
    "dos",
    "unix",
    "java",
    "javaaddpath",
    "javarmpath",
    "javaclasspath",
    "profile",
    "profsave",
    "profview",
    "memory",
    "maxNumCompThreads",
    "feature",
];

// =============================================================================
// Ecosystem impl
// =============================================================================

pub struct MatlabStdlibEcosystem;

impl Ecosystem for MatlabStdlibEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("matlab")
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        // Single synthetic root — no disk probe needed.
        vec![ExternalDepRoot {
            module_path: "matlab-stdlib".to_string(),
            version: String::new(),
            root: std::path::PathBuf::from(SYNTHETIC_PATH),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        }]
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        // Synthetic-only — nothing to walk on disk.
        Vec::new()
    }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn build_symbol_index(
        &self,
        _dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        let mut idx = SymbolLocationIndex::new();
        for &name in BUILTINS {
            idx.insert("matlab-stdlib", name, SYNTHETIC_PATH);
        }
        idx
    }

    /// Eagerly emit the synthetic builtins file on ecosystem activation so
    /// the indexer can resolve bare MATLAB built-in calls without requiring a
    /// chain walker. Called by the indexer for every active dep root.
    fn parse_metadata_only(&self, _dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        Some(vec![build_synthetic_parsed_file()])
    }

    fn demand_pre_pull(
        &self,
        _dep_roots: &[ExternalDepRoot],
    ) -> Vec<crate::walker::WalkedFile> {
        // No walkable files — synthetic only. The symbol index + parse_metadata_only
        // together cover all resolution paths.
        Vec::new()
    }
}

impl ExternalSourceLocator for MatlabStdlibEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        Ecosystem::locate_roots(
            self,
            &LocateContext {
                project_root: _project_root,
                manifests: &Default::default(),
                active_ecosystems: &[],
            },
        )
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        Some(vec![build_synthetic_parsed_file()])
    }
}

// =============================================================================
// Synthetic ParsedFile construction
// =============================================================================

fn build_synthetic_parsed_file() -> ParsedFile {
    let symbols: Vec<ExtractedSymbol> = BUILTINS
        .iter()
        .map(|&name| ExtractedSymbol {
            name: name.to_string(),
            qualified_name: name.to_string(),
            kind: SymbolKind::Function,
            visibility: Some(Visibility::Public),
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            signature: Some(format!("function {name}(...)")),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
        })
        .collect();

    let content_hash = format!("matlab-stdlib-synthetic-{}", symbols.len());

    ParsedFile {
        path: SYNTHETIC_PATH.to_string(),
        language: "matlab".to_string(),
        content_hash,
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    }
}

// =============================================================================
// Shared locator (process-wide singleton)
// =============================================================================

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<MatlabStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(MatlabStdlibEcosystem)).clone()
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for &name in BUILTINS {
            assert!(seen.insert(name), "duplicate built-in: {name}");
        }
    }

    #[test]
    fn symbol_index_contains_sentinel_names() {
        let eco = MatlabStdlibEcosystem;
        let idx = eco.build_symbol_index(&[]);
        assert!(
            idx.locate("matlab-stdlib", "zeros").is_some(),
            "zeros not found"
        );
        assert!(
            idx.locate("matlab-stdlib", "plot").is_some(),
            "plot not found"
        );
        assert!(
            idx.locate("matlab-stdlib", "sprintf").is_some(),
            "sprintf not found"
        );
    }

    #[test]
    fn symbol_index_count_above_threshold() {
        let eco = MatlabStdlibEcosystem;
        let idx = eco.build_symbol_index(&[]);
        // After deduplication the effective count equals BUILTINS.len()
        // (no overloads). Require at least 300.
        assert!(
            idx.len() >= 300,
            "expected at least 300 symbols, got {}",
            idx.len()
        );
    }

    #[test]
    fn parse_metadata_only_returns_parsed_file() {
        let eco = MatlabStdlibEcosystem;
        let dummy_root = ExternalDepRoot {
            module_path: "matlab-stdlib".to_string(),
            version: String::new(),
            root: std::path::PathBuf::from(SYNTHETIC_PATH),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        };
        let files = Ecosystem::parse_metadata_only(&eco, &dummy_root).expect("should return Some");
        assert_eq!(files.len(), 1);
        let pf = &files[0];
        assert_eq!(pf.path, SYNTHETIC_PATH);
        assert_eq!(pf.language, "matlab");
        assert!(!pf.symbols.is_empty());
        assert_eq!(pf.symbols.len(), BUILTINS.len());
    }

    #[test]
    fn all_symbols_are_functions() {
        let eco = MatlabStdlibEcosystem;
        let dummy_root = ExternalDepRoot {
            module_path: "matlab-stdlib".to_string(),
            version: String::new(),
            root: std::path::PathBuf::from(SYNTHETIC_PATH),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        };
        let files = Ecosystem::parse_metadata_only(&eco, &dummy_root)
            .expect("should return Some");
        for sym in &files[0].symbols {
            assert_eq!(
                sym.kind,
                SymbolKind::Function,
                "expected Function kind for '{}'",
                sym.name
            );
        }
    }

    #[test]
    fn walk_root_returns_empty() {
        let eco = MatlabStdlibEcosystem;
        let dummy_root = ExternalDepRoot {
            module_path: "matlab-stdlib".to_string(),
            version: String::new(),
            root: std::path::PathBuf::from(SYNTHETIC_PATH),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        };
        assert!(Ecosystem::walk_root(&eco, &dummy_root).is_empty());
    }
}
