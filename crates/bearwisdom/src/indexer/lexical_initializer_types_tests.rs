use super::*;

#[test]
fn expressions_preserve_source_sites_during_type_recipe_mapping() {
    let site = SourceSpan { start: 12, end: 19 };
    let expression = Expression::Construct {
        callee: site,
        arguments: vec![Expression::Read(site), Expression::Typed(1)],
        types: vec![2],
    };
    let Expression::Construct {
        callee,
        arguments,
        types,
    } = expression.map(&|value| value + 10)
    else {
        panic!()
    };
    assert_eq!(callee, site);
    assert_eq!(types, vec![12]);
    assert!(matches!(&arguments[0], Expression::Read(read) if *read == site));
    assert!(matches!(&arguments[1], Expression::Typed(11)));
}
