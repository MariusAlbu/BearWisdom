//! Extracted argument syntax and snapshot-local source addresses.
use serde::{Deserialize, Serialize};

/// A single argument in a call expression, captured at extract time.
///
/// Populated only for call-site refs (`EdgeKind::Calls`, `EdgeKind::Imports`)
/// when the extractor walks the argument list. The `Other` arm covers shapes
/// without a supported syntax representation; it never invents type evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallArg {
    /// Plain string literal: `"users"`, `'users'`.
    StringLit(String),
    /// Template literal with interpolation holes replaced by `{}`:
    /// `` `/api/users/${id}` `` → `"/api/users/{}"`.
    TemplateLit(String),
    /// Legacy identifier payload for unmigrated extractors. TS/JS use IdentAt.
    Ident(String),
    /// Exact identifier use; semantic consumers must use its prebound identity.
    IdentAt(SourceSpan),
    /// Whole source-owned value recipe; absence is unknown, never display text.
    ValueAt(SourceSpan),
    /// Exact borrow expression; source-owned region/mutability evidence is
    /// independent of the operand's binding identity.
    BorrowAt {
        span: SourceSpan,
        expr: Box<CallArg>,
    },
    /// Tagged template: `` gql`query Foo { users { id } }` ``.
    /// `tag` is the tag identifier (`"gql"`, `"sql"`, `"html"`).
    /// `body` is the raw inner text with interpolation holes removed.
    TaggedTemplate { tag: String, body: String },
    /// Numeric, boolean, null, or simple array/object literal — stored as
    /// its source text.
    Literal(String),
    /// Object literal whose property names are statically determinable, e.g.
    /// `{ template: 'welcome', subject: s }`. Captured as the ordered list of
    /// `(key, optional string-literal value)` pairs. The value is `Some(_)`
    /// only when the property's value is a plain string or template literal
    /// (no interpolation) — identifiers, function references, computed
    /// expressions all produce `None`. Used by mailer detectors that need
    /// the `template:` value AND by handler-registration detectors that only
    /// need the keys (`server.addService(SvcDef, { m1: h, m2: h })`).
    ObjectKeys(Vec<(String, Option<String>)>),
    /// Conditional (ternary) expression: `cond ? then_branch : else_branch`.
    /// The condition is discarded; both value branches are preserved for
    /// downstream typing of the result type.
    Ternary {
        then_branch: Box<CallArg>,
        else_branch: Box<CallArg>,
    },
    /// Array literal: `[elem0, elem1, ...]`. Each element is a nested `CallArg`
    /// so spread elements inside are represented as `Spread` children.
    ArrayLiteral { elements: Vec<CallArg> },
    /// Awaited expression: `await expr`. Carries the inner expression so the
    /// resolver can unwrap the promise type.
    Await { expr: Box<CallArg> },
    /// Spread element: `...expr`. Carries the inner expression.
    Spread { expr: Box<CallArg> },
    /// Subscript / index access: `container[index]`.
    IndexAccess {
        container: Box<CallArg>,
        index: Box<CallArg>,
    },
    /// Binary expression: `left op right`. `op` is the operator source text
    /// (e.g. `"+"`, `"&&"`).
    Binary {
        op: String,
        left: Box<CallArg>,
        right: Box<CallArg>,
    },
    /// Arrow-function or function-expression argument (`x => x.foo`,
    /// `function (a, b) { ... }`). `params` are the lambda's own positional
    /// parameter identifiers in declaration order. A parameter whose binding
    /// is not a plain identifier (destructuring / rest pattern) contributes an
    /// empty string in its slot so positions stay aligned with the callback
    /// signature. Carries the names only — they are the keys the chain walker
    /// seeds the local-type cache under after typing each param from the
    /// higher-order method's callback-parameter signature.
    Lambda { params: Vec<String> },
    /// Syntax-addressed callback parameters. None preserves unsupported pattern
    /// positions; spans map directly to file-local BindingIds, never names.
    LambdaAt { params: Vec<Option<SourceSpan>> },
    /// Any argument shape not covered by the above variants.
    Other,
}

/// Half-open UTF-8 byte range in one immutable source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceSpan {
    pub start: u32,
    pub end: u32,
}

impl CallArg {
    /// Traverse only syntax-addressed identifier leaves, preserving source identity.
    pub(crate) fn visit_identifiers(&self, visit: &mut impl FnMut(SourceSpan)) {
        match self {
            Self::IdentAt(span) => visit(*span),
            Self::ArrayLiteral { elements } => {
                for arg in elements {
                    arg.visit_identifiers(visit);
                }
            }
            Self::Await { expr } | Self::Spread { expr } | Self::BorrowAt { expr, .. } => {
                expr.visit_identifiers(visit)
            }
            Self::Ternary {
                then_branch,
                else_branch,
            } => {
                then_branch.visit_identifiers(visit);
                else_branch.visit_identifiers(visit);
            }
            Self::IndexAccess { container, index } => {
                container.visit_identifiers(visit);
                index.visit_identifiers(visit);
            }
            Self::Binary { left, right, .. } => {
                left.visit_identifiers(visit);
                right.visit_identifiers(visit);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "call_arg_tests.rs"]
mod tests;
