use super::predicates;

#[test]
fn third_party_libraries_not_classified_as_scala_builtin() {
    // Cats / fs2 / cats-effect / http4s / ScalaCheck names used to be
    // classified as Scala builtins. They are gem-provided third-party
    // APIs indexed by Maven externals when the project's build.sbt
    // declares them.
    for name in &[
        // Cats FP symbolic operators
        "*>", "<*", "===", "=!=", ">>=", "<*>", "<$>", "|+|",
        ">>>", "<<<", "&>", "<&",
        // fs2 / cats-effect / http4s effect-stream methods
        "unsafeRunSync", "evalMap", "compile", "drain", "through",
        "attempt", "handleErrorWith", "recoverWith",
        "tupleLeft", "tupleRight", "productL", "productR",
        // ScalaCheck
        "Gen", "Arbitrary", "forAll",
    ] {
        assert!(
            !predicates::is_scala_builtin(name),
            "{name:?} should not be classified as a scala builtin",
        );
    }
}

#[test]
fn real_scala_stdlib_still_classified() {
    // Sanity: Scala stdlib FP method names + Object methods + pseudo-
    // keywords still match.
    for name in &[
        // Stdlib FP
        "flatMap", "map", "fold", "foldLeft", "foldRight",
        "filter", "collect", "exists", "forall", "foreach",
        "groupBy", "toList", "toMap", "getOrElse", "orElse",
        "mkString", "zip", "head", "tail", "headOption",
        "isEmpty", "nonEmpty", "size", "length",
        // Object identity
        "toString", "hashCode", "equals", "canEqual",
        // pseudo-keywords
        "this", "super",
    ] {
        assert!(
            predicates::is_scala_builtin(name),
            "{name:?} must remain a scala builtin",
        );
    }
}
