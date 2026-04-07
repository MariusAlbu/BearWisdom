// =============================================================================
// haskell/primitives.rs — Haskell primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for Haskell.
pub(crate) const PRIMITIVES: &[&str] = &[
    // Prelude constructors / types
    "Just", "Nothing", "Left", "Right", "True", "False",
    "IO", "Maybe", "Either", "Bool", "Int", "Integer", "Float", "Double",
    "Char", "String", "Word", "Ordering", "LT", "GT", "EQ",
    // type classes
    "Show", "Read", "Eq", "Ord", "Num", "Integral", "Fractional", "Floating",
    "RealFrac", "Enum", "Bounded",
    "Functor", "Applicative", "Monad", "MonadIO",
    "Monoid", "Semigroup", "Foldable", "Traversable",
    // Prelude functions
    "return", "pure", "map", "fmap",
    "mapM", "mapM_", "forM", "forM_",
    "sequence", "sequence_", "traverse", "traverse_",
    "fold", "foldl", "foldr", "foldl'", "foldr'", "foldMap",
    "concat", "concatMap", "filter",
    "zip", "zipWith", "unzip",
    "lookup", "elem", "notElem",
    "null", "length", "head", "tail", "last", "init",
    "take", "drop", "takeWhile", "dropWhile", "span", "break",
    "reverse", "and", "or", "any", "all", "sum", "product",
    "maximum", "minimum", "iterate", "repeat", "replicate", "cycle",
    "words", "unwords", "lines", "unlines",
    "show", "read", "print",
    "putStr", "putStrLn", "getLine", "getContents", "interact",
    "readFile", "writeFile", "appendFile",
    "id", "const", "flip", "compose", "not", "otherwise",
    "undefined", "error", "seq",
    // Maybe/Either helpers
    "maybe", "either", "fromMaybe", "isJust", "isNothing", "fromJust",
    "catMaybes", "mapMaybe",
    "toList", "fromList",
    "isLeft", "isRight", "fromLeft", "fromRight", "partitionEithers",
    // monad / applicative helpers
    "guard", "when", "unless", "void", "join", "liftIO", "lift",
    // MTL
    "ask", "asks", "get", "gets", "put", "modify",
    "throwError", "catchError",
    // exceptions
    "catch", "try", "bracket", "finally", "evaluate", "throw",
    // operators
    "<>", ".", "$", "<$>", "<*>", ">>=", ">>", "<|>", "++",
    "==", "/=", "<", ">", "<=", ">=", "&&", "||",
    "=<<", "<$", "$>", "*>", "<*", ">>>", "<<<", "***", "&&&",
    // common types
    "Text", "ByteString",
    "Map", "Set", "IntMap", "IntSet", "HashMap", "HashSet",
    "Vector", "IORef", "MVar", "TVar", "STM", "Chan", "TChan", "TMVar",
    "Async",
    "Reader", "Writer", "State",
    "ReaderT", "WriterT", "StateT", "ExceptT", "MaybeT",
    "Identity", "Proxy", "Void",
    "All", "Any", "Sum", "Product", "First", "Last", "Endo", "Dual", "Down",
    "Generic", "Typeable", "Data", "NFData", "Hashable",
    "ToJSON", "FromJSON",
    "Exception", "SomeException", "IOException",
    "Handle", "FilePath", "IOMode",
    // generic type params
    "T", "U", "K", "V", "a", "b", "c", "d", "e", "f", "m", "n", "s", "t", "r", "w",
];
