use std::collections::HashSet;

/// Runtime globals always external for Haskell.
/// Covers: Prelude functions, common operators, Maybe/Either/Bool constructors,
/// and frequently-used names from Data.Map, Data.Set, Data.Text, Control.Monad.
pub(crate) const EXTERNALS: &[&str] = &[
    // --- Operators (critical — most unresolved in practice) ---
    "<>", ".", "<$>", "<*>", "<|>", "<$", "$>",
    "++", "==", "/=", ">>=", ">>", "=<<",
    "$", "&&", "||",
    ">=", "<=", ">", "<",
    "+", "-", "*", "/", "^", "**", "^^",
    "!!", ":", "@",
    "<=<", ">=>",
    "liftA2", "liftM2",

    // --- Bool / Maybe / Either / Ordering constructors ---
    "True", "False",
    "Just", "Nothing",
    "Left", "Right",
    "LT", "EQ", "GT",

    // --- Monad / Applicative / Functor ---
    "return", "pure", "fmap", "bind",
    "sequence", "sequence_", "mapM", "mapM_",
    "forM", "forM_", "void", "join",
    "when", "unless", "guard",
    "mconcat", "mempty", "mappend",
    "ap", "liftA", "liftM",

    // --- Core Prelude ---
    "id", "const", "flip", "curry", "uncurry",
    "fix", "on", "(&)",
    "not", "and", "or", "any", "all",
    "show", "read", "reads", "print",
    "putStr", "putStrLn", "getLine", "getContents",
    "interact", "readFile", "writeFile", "appendFile",
    "error", "undefined", "errorWithoutStackTrace",
    "seq", "otherwise",
    "toEnum", "fromEnum",
    "succ", "pred",
    "minBound", "maxBound",

    // --- Numeric / Ord ---
    "abs", "signum", "negate", "recip",
    "fromInteger", "fromIntegral", "realToFrac",
    "floor", "ceiling", "round", "truncate",
    "div", "mod", "quot", "rem", "divMod", "quotRem",
    "gcd", "lcm", "even", "odd",
    "pi", "exp", "log", "sqrt",
    "sin", "cos", "tan", "asin", "acos", "atan", "atan2",
    "logBase", "(**)",
    "compare", "max", "min",

    // --- List / String (Prelude) ---
    "map", "filter", "foldr", "foldl", "foldl'", "foldr1", "foldl1",
    "head", "tail", "last", "init",
    "take", "drop", "takeWhile", "dropWhile",
    "span", "break", "splitAt",
    "zip", "zip3", "unzip", "unzip3",
    "zipWith", "zipWith3",
    "iterate", "repeat", "replicate", "cycle",
    "reverse", "elem", "notElem", "lookup",
    "null", "length",
    "concat", "concatMap",
    "words", "unwords", "lines", "unlines",
    "sum", "product", "maximum", "minimum",
    "scanl", "scanr", "scanl1", "scanr1",
    "and", "or",
    "nub", "sort", "sortBy", "groupBy",
    "partition", "isPrefixOf", "isSuffixOf", "isInfixOf",
    "intercalate", "intersperse", "transpose",
    "find", "findIndex", "findIndices",
    "elemIndex", "elemIndices",
    "stripPrefix",

    // --- String / Char ---
    "ord", "chr",
    "isAlpha", "isAlphaNum", "isDigit", "isLower", "isUpper",
    "isSpace", "isPunctuation",
    "toUpper", "toLower",
    "digitToInt", "intToDigit",

    // --- IO / Monad plumbing ---
    "ioError", "userError", "catch", "try", "evaluate",
    "newIORef", "readIORef", "writeIORef", "modifyIORef", "modifyIORef'",
    "newMVar", "takeMVar", "putMVar", "readMVar", "modifyMVar", "modifyMVar_",
    "newTMVar", "newTVar", "readTVar", "writeTVar", "modifyTVar", "modifyTVar'",
    "atomically", "retry", "orElse",
    "hPutStrLn", "hPutStr", "hGetLine", "hGetContents",
    "hSetBuffering", "hFlush", "hClose",
    "stdin", "stdout", "stderr",
    "handle", "throwIO", "throw", "catches",

    // --- Data.Maybe (unqualified via import) ---
    "maybe", "fromMaybe", "fromJust", "isJust", "isNothing",
    "listToMaybe", "maybeToList", "catMaybes", "mapMaybe",

    // --- Data.Either (unqualified) ---
    "either", "fromLeft", "fromRight", "isLeft", "isRight",
    "lefts", "rights", "partitionEithers",

    // --- Data.List extras (commonly imported unqualified) ---
    "sortOn", "groupBy", "group", "nubBy", "deleteBy",
    "maximumBy", "minimumBy", "genericLength",
    "tails", "inits", "permutations", "subsequences",

    // --- Data.Map / Data.Map.Strict common names ---
    "Map.empty", "Map.singleton", "Map.insert", "Map.insertWith",
    "Map.delete", "Map.lookup", "Map.findWithDefault",
    "Map.member", "Map.notMember", "Map.size", "Map.null",
    "Map.toList", "Map.fromList", "Map.toAscList", "Map.fromListWith",
    "Map.unionWith", "Map.union", "Map.intersectionWith", "Map.intersection",
    "Map.difference", "Map.map", "Map.mapWithKey", "Map.filter",
    "Map.filterWithKey", "Map.foldr", "Map.foldl", "Map.foldrWithKey",
    "Map.adjust", "Map.update", "Map.alter",
    "Map.keys", "Map.elems", "Map.assocs",

    // --- Data.Set common names ---
    "Set.empty", "Set.singleton", "Set.insert", "Set.delete",
    "Set.member", "Set.notMember", "Set.size", "Set.null",
    "Set.toList", "Set.fromList", "Set.toAscList",
    "Set.union", "Set.intersection", "Set.difference",
    "Set.map", "Set.filter", "Set.fold",
    "Set.isSubsetOf", "Set.disjoint",

    // --- Data.Text common names ---
    "Text.pack", "Text.unpack", "Text.empty", "Text.null",
    "Text.length", "Text.append", "Text.concat", "Text.intercalate",
    "Text.map", "Text.filter", "Text.foldl", "Text.foldr",
    "Text.isPrefixOf", "Text.isSuffixOf", "Text.isInfixOf",
    "Text.strip", "Text.stripPrefix", "Text.splitOn", "Text.words", "Text.lines",
    "Text.toLower", "Text.toUpper",
    "Text.replace", "Text.take", "Text.drop", "Text.takeWhile",
    "Text.decodeUtf8", "Text.encodeUtf8",

    // --- Data.ByteString common names ---
    "BS.pack", "BS.unpack", "BS.empty", "BS.null", "BS.length",
    "BS.append", "BS.concat", "BS.intercalate",

    // --- Control.Monad extras ---
    "replicateM", "replicateM_", "filterM", "foldM", "foldM_",
    "zipWithM", "zipWithM_", "forever", "forM", "forM_",
    "msum", "mfilter", "MonadPlus",

    // --- Data.Char ---
    "generalCategory",

    // --- System.IO ---
    "withFile", "openFile", "doesFileExist", "doesDirectoryExist",
    "createDirectory", "removeFile", "renameFile",
    "getArgs", "getProgName", "lookupEnv", "getEnvironment",
    "exitSuccess", "exitFailure", "exitWith",

    // --- Numeric / formatting ---
    "showHex", "showOct", "showIntAtBase", "readHex", "readOct",
    "printf", "hPrintf",
];

/// Dependency-gated framework globals for Haskell.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    // HSpec
    for dep in ["hspec", "hspec-core", "hspec-discover"] {
        if deps.contains(dep) {
            globals.extend(HSPEC_GLOBALS);
            break;
        }
    }

    // HUnit
    for dep in ["HUnit", "hunit"] {
        if deps.contains(dep) {
            globals.extend(HUNIT_GLOBALS);
            break;
        }
    }

    // QuickCheck
    for dep in ["QuickCheck", "quickcheck"] {
        if deps.contains(dep) {
            globals.extend(QUICKCHECK_GLOBALS);
            break;
        }
    }

    // Tasty
    for dep in ["tasty", "tasty-hspec", "tasty-hunit", "tasty-quickcheck"] {
        if deps.contains(dep) {
            globals.extend(TASTY_GLOBALS);
            break;
        }
    }

    globals
}

const HSPEC_GLOBALS: &[&str] = &[
    "describe", "context", "it", "xit", "specify", "xspecify",
    "before", "before_", "beforeAll", "beforeAll_",
    "after", "after_", "afterAll", "afterAll_",
    "around", "around_", "aroundAll", "aroundAll_",
    "parallel", "sequential",
    "shouldBe", "shouldNotBe", "shouldSatisfy", "shouldNotSatisfy",
    "shouldContain", "shouldNotContain", "shouldStartWith", "shouldEndWith",
    "shouldThrow", "shouldReturn", "shouldNotReturn",
    "shouldMatchList", "shouldBeSameAs",
    "hspec", "spec", "runSpec",
    "expectationFailure", "pendingWith", "pending",
    "expectTrue", "expectFalse",
];

const HUNIT_GLOBALS: &[&str] = &[
    "assertEqual", "assertBool", "assertFailure",
    "assertString", "assertThrows",
    "@=?", "~=?", "@?=", "~?=", "@?", "~?",
    "runTestTT", "runTestTTAndExit",
    "TestCase", "TestList", "TestLabel",
];

const QUICKCHECK_GLOBALS: &[&str] = &[
    "quickCheck", "quickCheckWith", "verboseCheck",
    "property", "forAll", "forAllShrink",
    "arbitrary", "shrink", "coarbitrary",
    "choose", "oneof", "frequency", "elements",
    "listOf", "listOf1", "vectorOf",
    "suchThat", "suchThatMaybe",
    "resize", "scale", "sized",
    "classify", "collect", "cover", "label",
    "expectFailure", "once", "within",
    "Gen", "Arbitrary",
];

const TASTY_GLOBALS: &[&str] = &[
    "testGroup", "testCase", "testProperty",
    "defaultMain", "defaultMainWithIngredients",
    "(@=?)", "(@?=)", "(@?)","(~=?)", "(~?=)",
    "assertEqual", "assertBool", "assertFailure",
];
