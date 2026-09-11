//! Ruby callback syntax and capture policy.
use crate::{
    indexer::callback_lexical::{
        CallbackBarrier, CallbackDescriptor, CallbackLexicalAdapter, CallbackParameter,
        OuterCapture,
    },
    types::SourceSpan,
};
use tree_sitter::Node;
pub(crate) static ADAPTER: CallbackLexicalAdapter = CallbackLexicalAdapter { describe };
fn sp(n: Node) -> SourceSpan {
    SourceSpan {
        start: n.start_byte() as u32,
        end: n.end_byte() as u32,
    }
}
fn kids(n: Node) -> Vec<Node> {
    let mut c = n.walk();
    n.named_children(&mut c).collect()
}
fn txt(s: &[u8], n: Node) -> Option<String> {
    std::str::from_utf8(&s[n.start_byte()..n.end_byte()])
        .ok()
        .map(str::to_owned)
}
fn describe(n: Node, s: &[u8]) -> Option<CallbackDescriptor> {
    if n.has_error() || !matches!(n.kind(), "block" | "do_block" | "lambda") {
        return None;
    }
    let body = n.child_by_field_name("body")?;
    let mut parameters = vec![];
    let mut barriers = barriers(body, s);
    if let Some(ps) = n.child_by_field_name("parameters") {
        for i in 0..ps.child_count() {
            let Some(p) = ps.child(i) else { continue };
            if !p.is_named() {
                continue;
            }
            let local = ps.field_name_for_child(i as u32) == Some("locals");
            if p.kind() == "identifier" && !local {
                parameters.push(CallbackParameter {
                    declaration: sp(p),
                    name: txt(s, p)?,
                })
            } else {
                for x in unsupported(p) {
                    if let Some(name) = txt(s, x) {
                        barriers.push(CallbackBarrier {
                            name,
                            range: sp(body),
                        })
                    }
                }
            }
        }
    }
    let boundaries = boundaries(body);
    if parameters.is_empty() && barriers.is_empty() && boundaries.is_empty() {
        return None;
    }
    Some(CallbackDescriptor {
        body: sp(body),
        parameters,
        barriers,
        boundaries: boundaries.into_iter().map(sp).collect(),
        outer_capture: OuterCapture::Transparent,
    })
}
fn unsupported(n: Node) -> Vec<Node> {
    match n.kind() {
        "identifier" => vec![n],
        "optional_parameter"
        | "keyword_parameter"
        | "splat_parameter"
        | "hash_splat_parameter"
        | "block_parameter" => n.child_by_field_name("name").into_iter().collect(),
        "destructured_parameter" => kids(n).into_iter().flat_map(unsupported).collect(),
        _ => vec![],
    }
}
fn boundaries(n: Node) -> Vec<Node> {
    fn v<'a>(n: Node<'a>, r: Node<'a>, o: &mut Vec<Node<'a>>) {
        if n != r
            && matches!(
                n.kind(),
                "method" | "singleton_method" | "class" | "module" | "singleton_class"
            )
        {
            o.push(n);
            return;
        }
        for c in kids(n) {
            v(c, r, o)
        }
    }
    let mut o = vec![];
    v(n, n, &mut o);
    o
}
fn barriers(body: Node, s: &[u8]) -> Vec<CallbackBarrier> {
    fn v(n: Node, r: Node, s: &[u8], o: &mut Vec<CallbackBarrier>) {
        if n != r && matches!(n.kind(), "block" | "do_block" | "lambda") {
            return;
        }
        let mut add = |x: Node| {
            if let Some(name) = txt(s, x) {
                o.push(CallbackBarrier {
                    name,
                    range: SourceSpan {
                        start: n.start_byte() as u32,
                        end: r.end_byte() as u32,
                    },
                })
            }
        };
        match n.kind() {
            "assignment" | "operator_assignment" => {
                if let Some(x) = n
                    .child_by_field_name("left")
                    .filter(|x| x.kind() == "identifier")
                {
                    add(x)
                }
            }
            "for" => {
                if let Some(x) = n
                    .child_by_field_name("pattern")
                    .filter(|x| x.kind() == "identifier")
                {
                    add(x)
                }
            }
            "rescue" => {
                if let Some(x) = n
                    .child_by_field_name("variable")
                    .and_then(|x| kids(x).into_iter().next())
                    .filter(|x| x.kind() == "identifier")
                {
                    add(x)
                }
            }
            _ => {}
        }
        for c in kids(n) {
            v(c, r, s, o)
        }
    }
    let mut o = vec![];
    v(body, body, s, &mut o);
    o
}
