// =============================================================================
// matlab/primitives.rs — MATLAB primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for MATLAB.
pub(crate) const PRIMITIVES: &[&str] = &[
    // array / matrix
    "size", "length", "numel", "ndims", "repmat", "reshape",
    "zeros", "ones", "eye", "rand", "randn", "randi",
    "linspace", "logspace",
    // statistics
    "max", "min", "sum", "prod", "mean", "median", "var", "std",
    "cumsum", "cumprod", "sort", "sortrows", "unique",
    // logic / type checks
    "find", "isempty", "isequal", "isa", "isnan", "isinf", "isfinite",
    "isreal", "ischar", "isstring", "isnumeric", "islogical", "iscell",
    "isstruct", "isfloat", "isinteger",
    // math
    "abs", "ceil", "floor", "round", "fix", "mod", "rem", "sign",
    "sqrt", "exp", "log", "log2", "log10",
    "sin", "cos", "tan", "asin", "acos", "atan", "atan2",
    "sinh", "cosh", "tanh",
    "real", "imag", "conj", "angle",
    // linear algebra
    "norm", "dot", "cross", "det", "inv", "eig", "svd", "rank",
    "trace", "diag", "triu", "tril", "kron",
    // array construction / manipulation
    "cat", "horzcat", "vertcat",
    // cell / struct
    "cell", "struct", "fieldnames", "rmfield", "cellfun", "arrayfun", "structfun",
    // I/O
    "sprintf", "fprintf", "printf", "disp", "display", "warning", "error",
    // control
    "assert", "nargin", "nargout", "varargin", "varargout",
    "deal", "feval", "eval", "assignin", "evalin",
    "exist", "who", "whos", "clear",
    // graphics
    "close", "hold", "plot", "figure", "subplot", "title", "xlabel", "ylabel",
    "legend", "grid", "axis", "xlim", "ylim", "set", "get", "gcf", "gca",
    "drawnow", "pause",
    // I/O
    "input", "keyboard",
    // flow keywords
    "return", "break", "continue", "switch", "case", "otherwise",
    "for", "while", "end", "if", "else", "elseif",
    "function", "classdef", "properties", "methods", "events", "enumeration",
    // constants / special values
    "true", "false", "pi", "eps", "inf", "NaN", "nan", "Inf", "i", "j", "ans",
    // numeric types
    "char", "double", "single",
    "int8", "int16", "int32", "int64",
    "uint8", "uint16", "uint32", "uint64",
    "logical", "string", "table", "timetable",
    // conversion
    "cell2mat", "mat2cell", "num2str", "str2num", "str2double",
    "strsplit", "strjoin",
    // string predicates
    "contains", "startsWith", "endsWith", "replace", "strrep",
    "regexp", "regexpi", "regexprep",
    "strcmp", "strcmpi", "lower", "upper", "strip", "strtrim",
    // filesystem
    "fullfile", "fileparts", "tempname", "tempdir", "pwd", "cd", "dir", "ls",
    "mkdir", "delete", "copyfile", "movefile",
    // file I/O
    "fopen", "fclose", "fread", "fwrite", "fscanf", "fgets", "fgetl",
    "feof", "ftell", "fseek",
    // data I/O
    "load", "save", "importdata", "readtable", "writetable",
    "readmatrix", "writematrix", "jsonencode", "jsondecode",
    "webread", "webwrite",
    // containers.Map
    "map", "keys", "values", "containers.Map", "isKey", "remove",
    // operators exposed as functions
    "subsref", "subsasgn", "colon", "transpose", "ctranspose",
    "plus", "minus", "times", "mtimes", "rdivide", "mrdivide",
    "ldivide", "mldivide", "power", "mpower",
    "lt", "gt", "le", "ge", "eq", "ne",
    "and", "or", "not", "any", "all", "xor",
    "bitand", "bitor", "bitxor",
];
