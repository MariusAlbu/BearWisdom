use super::extract::extract;

// Regression: extract() must not panic on CREATE TABLE definitions whose
// column COMMENT clauses contain multi-byte UTF-8 characters that straddle
// the 80-byte AST-fallback window in `has_constraint_clause_marker`.
//
// Source repro from the corpus sweep (jupyter-python-100-days,
// thymeleaf-myblog): MySQL DDL with Chinese column comments where the
// 80-byte cut landed inside a 3-byte CJK char.
#[test]
fn create_table_with_cjk_column_comment_does_not_panic() {
    let src = r#"
CREATE TABLE orders (
  `order_sn` varchar(255) COLLATE utf8mb4_unicode_ci NOT NULL COMMENT '交易单号',
  `locked` tinyint(4) DEFAULT '0' COMMENT '是否锁定 0未锁定 1已锁定无法登陆'
);
"#;
    let result = extract(src);
    let table = result.symbols.iter().find(|s| s.name == "orders");
    assert!(table.is_some(), "expected `orders` table symbol to be extracted");
}
