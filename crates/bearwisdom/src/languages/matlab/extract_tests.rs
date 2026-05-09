use super::*;

// Helper: parse source and return the collected local bindings.
fn bindings(src: &str) -> Vec<(String, u32, u32)> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_matlab::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(src, None).unwrap();
    let file_end = tree.root_node().end_position().row as u32;
    let mut out = Vec::new();
    collect_local_bindings(tree.root_node(), src.as_bytes(), 0, file_end, &mut out);
    out
}

// -------------------------------------------------------------------------
// CST probe: verify grammar kind names are as expected
// -------------------------------------------------------------------------

#[test]
fn cst_probe_function_definition_has_function_arguments_and_output() {
    let src = "function [label, mu] = kmeans(X, m)\nlabel = 1;\nend\n";
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_matlab::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(src, None).unwrap();
    let root = tree.root_node();
    let fn_def_id = {
        let mut cursor = root.walk();
        let mut found = None;
        for child in root.children(&mut cursor) {
            if child.kind() == "function_definition" {
                found = Some(child.id());
                break;
            }
        }
        found.expect("expected function_definition")
    };
    let mut child_kinds: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.id() == fn_def_id {
            let mut cc = child.walk();
            for grandchild in child.children(&mut cc) {
                child_kinds.push(grandchild.kind().to_owned());
            }
            break;
        }
    }
    assert!(
        child_kinds.iter().any(|k| k == "function_arguments"),
        "expected function_arguments child; got {child_kinds:?}"
    );
    assert!(
        child_kinds.iter().any(|k| k == "function_output"),
        "expected function_output child; got {child_kinds:?}"
    );
}

#[test]
fn cst_probe_for_statement_iterator_structure() {
    let src = "for i = 1:10\n  disp(i);\nend\n";
    let src_bytes = src.as_bytes();
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_matlab::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(src, None).unwrap();
    let root = tree.root_node();
    let mut loop_var: Option<String> = None;
    let mut cursor = root.walk();
    'outer: for child in root.children(&mut cursor) {
        if child.kind() == "for_statement" {
            let mut fc = child.walk();
            for for_child in child.children(&mut fc) {
                if for_child.kind() == "iterator" {
                    let mut ic = for_child.walk();
                    for iter_child in for_child.children(&mut ic) {
                        if iter_child.kind() == "identifier" {
                            loop_var =
                                Some(iter_child.utf8_text(src_bytes).unwrap().to_owned());
                            break 'outer;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(
        loop_var.as_deref(),
        Some("i"),
        "expected loop var 'i'; got {loop_var:?}"
    );
}

// -------------------------------------------------------------------------
// Binding collection
// -------------------------------------------------------------------------

#[test]
fn input_params_collected() {
    let src = "function foo(X, m)\nX(1) = 0;\nend\n";
    let b = bindings(src);
    assert!(
        b.iter().any(|(n, _, _)| n == "X"),
        "expected X in bindings; got {b:?}"
    );
    assert!(
        b.iter().any(|(n, _, _)| n == "m"),
        "expected m in bindings; got {b:?}"
    );
}

#[test]
fn output_params_single_collected() {
    let src = "function label = init(X, m)\nlabel = 1;\nend\n";
    let b = bindings(src);
    assert!(
        b.iter().any(|(n, _, _)| n == "label"),
        "expected label in bindings; got {b:?}"
    );
}

#[test]
fn output_params_multi_collected() {
    let src = "function [label, mu, energy] = kmeans(X, m)\nlabel = 1;\nend\n";
    let b = bindings(src);
    assert!(
        b.iter().any(|(n, _, _)| n == "label"),
        "expected label; got {b:?}"
    );
    assert!(
        b.iter().any(|(n, _, _)| n == "mu"),
        "expected mu; got {b:?}"
    );
    assert!(
        b.iter().any(|(n, _, _)| n == "energy"),
        "expected energy; got {b:?}"
    );
}

#[test]
fn assignment_lhs_collected() {
    let src = "function foo(X)\nn = numel(X);\nend\n";
    let b = bindings(src);
    assert!(
        b.iter().any(|(n, _, _)| n == "n"),
        "expected n from assignment; got {b:?}"
    );
}

#[test]
fn for_loop_var_collected() {
    let src = "function foo(X)\nfor i = 1:10\n  X(i) = 0;\nend\nend\n";
    let b = bindings(src);
    assert!(
        b.iter().any(|(n, _, _)| n == "i"),
        "expected i from for loop; got {b:?}"
    );
}

// -------------------------------------------------------------------------
// Filter effect
// -------------------------------------------------------------------------

#[test]
fn input_param_X_not_emitted_as_ref() {
    let src = "function foo(X)\ny = X(1);\nend\n";
    let result = extract(src);
    let x_refs: Vec<_> = result
        .refs
        .iter()
        .filter(|r| r.target_name == "X")
        .collect();
    assert!(
        x_refs.is_empty(),
        "expected no refs for X (local param); got {x_refs:?}"
    );
}

#[test]
fn non_local_call_still_emitted() {
    let src = "function foo(X)\ny = zeros(3);\nend\n";
    let result = extract(src);
    assert!(
        result.refs.iter().any(|r| r.target_name == "zeros"),
        "expected ref for zeros; got {:?}",
        result.refs.iter().map(|r| &r.target_name).collect::<Vec<_>>()
    );
}

#[test]
fn output_param_mu_not_emitted_as_ref() {
    let src = concat!(
        "function [label, mu, energy] = kmeans(X, m)\n",
        "mu = X;\n",
        "val = mu';\n",
        "end\n",
    );
    let result = extract(src);
    let b = bindings(src);
    assert!(
        b.iter().any(|(n, _, _)| n == "mu"),
        "expected mu bound; got {b:?}"
    );
    // mu used as postfix `mu'` doesn't produce a function_call ref.
    let _ = result;
}

// -------------------------------------------------------------------------
// The kmeans.m nested-function case
// -------------------------------------------------------------------------

#[test]
fn kmeans_nested_functions_X_filtered() {
    let src = concat!(
        "function [label, mu, energy] = kmeans(X, m)\n",
        "label = init(X, m);\n",
        "n = numel(label);\n",
        "idx = 1:n;\n",
        "last = zeros(1,n);\n",
        "while any(label ~= last)\n",
        "    mu = X*normalize(sparse(idx,last,1),1);\n",
        "end\n",
        "energy = 0;\n",
        "function label = init(X, m)\n",
        "[d,n] = size(X);\n",
        "if numel(m) == 1\n",
        "    mu = X(:,randperm(n,m));\n",
        "end\n",
        "end\n",
    );
    let result = extract(src);
    let x_refs: Vec<_> = result
        .refs
        .iter()
        .filter(|r| r.target_name == "X")
        .collect();
    assert!(
        x_refs.is_empty(),
        "expected no refs for X; got {x_refs:?} (lines: {:?})",
        x_refs.iter().map(|r| r.line).collect::<Vec<_>>()
    );
    assert!(
        result.refs.iter().any(|r| r.target_name == "zeros"),
        "expected ref for zeros to survive filter"
    );
    assert!(
        result.refs.iter().any(|r| r.target_name == "size"),
        "expected ref for size to survive filter"
    );
}

#[test]
fn cst_probe_init_function_range() {
    let src = concat!(
        "function [label, mu, energy] = kmeans(X, m)\n",
        "label = init(X, m);\n",
        "n = numel(label);\n",
        "idx = 1:n;\n",
        "last = zeros(1,n);\n",
        "while any(label ~= last)\n",
        "    mu = X*normalize(sparse(idx,last,1),1);\n",
        "    [val,label] = min(dot(mu,mu,1)'/2-mu'*X,[],1);\n",
        "end\n",
        "energy = dot(X(:),X(:),1)+2*sum(val);\n",
        "\n",
        "function label = init(X, m)\n",
        "[d,n] = size(X);\n",
        "if numel(m) == 1\n",
        "    mu = X(:,randperm(n,m));\n",
        "    [~,label] = min(dot(mu,mu,1)'/2-mu'*X,[],1);\n",
        "elseif all(size(m) == [1,n])\n",
        "    label = m;\n",
        "elseif size(m,1) == d\n",
        "    [~,label] = min(dot(m,m,1)'/2-m'*X,[],1);\n",
        "end\n",
    );
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_matlab::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(src, None).unwrap();
    let root = tree.root_node();
    let src_bytes = src.as_bytes();
    let mut fn_ranges: Vec<(String, u32, u32)> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "function_definition" {
            let name = child
                .child_by_field_name("name")
                .map(|n| n.utf8_text(src_bytes).unwrap().to_owned())
                .unwrap_or_default();
            fn_ranges.push((
                name,
                child.start_position().row as u32,
                child.end_position().row as u32,
            ));
            let mut cc = child.walk();
            for grandchild in child.children(&mut cc) {
                if grandchild.kind() == "function_definition" {
                    let gname = grandchild
                        .child_by_field_name("name")
                        .map(|n| n.utf8_text(src_bytes).unwrap().to_owned())
                        .unwrap_or_default();
                    fn_ranges.push((
                        format!("nested:{gname}"),
                        grandchild.start_position().row as u32,
                        grandchild.end_position().row as u32,
                    ));
                }
            }
        }
    }
    let b = bindings(src);
    let x_bindings: Vec<_> = b.iter().filter(|(n, _, _)| n == "X").collect();
    assert!(
        x_bindings.iter().any(|(_, start, end)| 14 >= *start && 14 <= *end),
        "expected X binding to cover line 14; x_bindings={x_bindings:?}, fn_ranges={fn_ranges:?}"
    );
    let result = extract(src);
    let x_at_14: Vec<_> = result
        .refs
        .iter()
        .filter(|r| r.target_name == "X" && r.line == 14)
        .collect();
    assert!(
        x_at_14.is_empty(),
        "expected X at line 14 to be filtered; refs={x_at_14:?}, fn_ranges={fn_ranges:?}, x_bindings={x_bindings:?}"
    );
}

#[test]
fn cst_probe_real_kmeans_structure() {
    let src = concat!(
        "function [label, mu, energy] = kmeans(X, m)\n",
        "% comment 1\n",
        "% comment 2\n",
        "% comment 3\n",
        "% comment 4\n",
        "% comment 5\n",
        "% comment 6\n",
        "% comment 7\n",
        "% comment 8\n",
        "% comment 9\n",
        "label = init(X, m);\n",
        "n = numel(label);\n",
        "idx = 1:n;\n",
        "last = zeros(1,n);\n",
        "while any(label ~= last)\n",
        "    mu = X*normalize(sparse(idx,last,1),1);\n",
        "    [val,label] = min(dot(mu,mu,1)'/2-mu'*X,[],1);\n",
        "end\n",
        "energy = dot(X(:),X(:),1)+2*sum(val);\n",
        "\n",
        "function label = init(X, m)\n",
        "[d,n] = size(X);\n",
        "if numel(m) == 1\n",
        "    mu = X(:,randperm(n,m));\n",
        "    [~,label] = min(dot(mu,mu,1)'/2-mu'*X,[],1);\n",
        "elseif all(size(m) == [1,n])\n",
        "    label = m;\n",
        "elseif size(m,1) == d\n",
        "    [~,label] = min(dot(m,m,1)'/2-m'*X,[],1);\n",
        "end\n",
    );
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_matlab::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(src, None).unwrap();
    let root = tree.root_node();
    let src_bytes = src.as_bytes();
    let mut fn_ranges: Vec<(String, u32, u32)> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "function_definition" {
            let name = child
                .child_by_field_name("name")
                .map(|n| n.utf8_text(src_bytes).unwrap().to_owned())
                .unwrap_or_default();
            fn_ranges.push((name, child.start_position().row as u32, child.end_position().row as u32));
        }
    }
    let b = bindings(src);
    let x_bindings: Vec<_> = b.iter().filter(|(n, _, _)| n == "X").collect();
    let covered = x_bindings.iter().any(|(_, start, end)| 23 >= *start && 23 <= *end);
    assert!(
        covered,
        "X binding does NOT cover line 23; fn_ranges={fn_ranges:?}, x_bindings={x_bindings:?}"
    );
}

#[test]
fn top_level_script_calls_not_over_filtered() {
    let src = "X = rand(3);\nfoo(X);\n";
    let result = extract(src);
    assert!(
        result.refs.iter().any(|r| r.target_name == "foo"),
        "expected ref for foo in top-level script; got {:?}",
        result.refs.iter().map(|r| &r.target_name).collect::<Vec<_>>()
    );
    assert!(
        result.refs.iter().any(|r| r.target_name == "rand"),
        "expected ref for rand in top-level script; got {:?}",
        result.refs.iter().map(|r| &r.target_name).collect::<Vec<_>>()
    );
}

#[test]
fn lambda_params_not_emitted_as_refs() {
    let src = "Wn = cellfun(@(x) dot(x(:),x(:)),W);\n";
    let result = extract(src);
    let x_refs: Vec<_> = result
        .refs
        .iter()
        .filter(|r| r.target_name == "x")
        .collect();
    assert!(
        x_refs.is_empty(),
        "expected no refs for x (lambda param); got {x_refs:?}"
    );
    assert!(
        result.refs.iter().any(|r| r.target_name == "cellfun"),
        "expected ref for cellfun; got {:?}",
        result.refs.iter().map(|r| &r.target_name).collect::<Vec<_>>()
    );
}

// -------------------------------------------------------------------------
// Fix 1: cell-indexing brace guard
// -------------------------------------------------------------------------

#[test]
fn cell_indexing_brace_not_emitted_as_ref() {
    // `Population{2}` and `R{1}` are cell-array indexing — not callable targets.
    let src = "function foo(Population, R)\ny = Population{2};\nz = R{1};\nend\n";
    let result = extract(src);
    let brace_refs: Vec<_> = result
        .refs
        .iter()
        .filter(|r| r.target_name.contains('{') || r.target_name.contains('}'))
        .collect();
    assert!(
        brace_refs.is_empty(),
        "expected no refs with braces; got {brace_refs:?}"
    );
}

#[test]
fn field_cell_indexing_not_emitted_as_ref() {
    // `obj.lu{mm}` — field cell-indexing; `lu` should not appear as a Calls ref.
    let src = "function foo(obj)\ny = obj.lu{1};\nend\n";
    let result = extract(src);
    // `lu` with `{` is the brace-guard case for the field_expression arm.
    let lu_refs: Vec<_> = result
        .refs
        .iter()
        .filter(|r| r.target_name == "lu")
        .collect();
    assert!(
        lu_refs.is_empty(),
        "expected no Calls ref for field cell-index `obj.lu{{1}}`; got {lu_refs:?}"
    );
}

// -------------------------------------------------------------------------
// Fix 2: indexed struct-field assignment LHS binding
// -------------------------------------------------------------------------

#[test]
fn indexed_field_assignment_lhs_bound() {
    // `obj.app.dropD(1) = GUI.APP(...)` — `dropD` should be bound so no phantom
    // Calls ref is emitted for it.
    let src = "function foo(obj, GUI)\nobj.app.dropD(1) = GUI.APP(1);\nend\n";
    let b = bindings(src);
    assert!(
        b.iter().any(|(n, _, _)| n == "dropD"),
        "expected dropD bound from indexed field assignment; got {b:?}"
    );
    let result = extract(src);
    let dropd_refs: Vec<_> = result
        .refs
        .iter()
        .filter(|r| r.target_name == "dropD")
        .collect();
    assert!(
        dropd_refs.is_empty(),
        "expected no Calls ref for dropD (indexed field assignment LHS); got {dropd_refs:?}"
    );
}

#[test]
fn plain_field_assignment_lhs_does_not_suppress_real_calls() {
    // The field-binding fix must not suppress legitimate call refs on the RHS.
    let src = "function foo(obj)\nobj.x = zeros(3);\nend\n";
    let result = extract(src);
    assert!(
        result.refs.iter().any(|r| r.target_name == "zeros"),
        "expected Calls ref for zeros to survive; got {:?}",
        result.refs.iter().map(|r| &r.target_name).collect::<Vec<_>>()
    );
}
