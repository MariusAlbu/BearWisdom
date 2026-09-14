//! Language-neutral callback binding graph assembly.
//!
//! Language plugins normalize callback syntax into `CallbackDescriptor`s. This
//! module only nests callback scopes and attests extracted chain roots.
use crate::{
    indexer::lexical::{BindingId, LexicalBindings, ScopeId},
    types::{ExtractedRef, SourceSpan},
};
use tree_sitter::Node;

#[derive(Clone, Copy)]
pub struct CallbackLexicalAdapter {
    pub describe: fn(Node, &[u8]) -> Option<CallbackDescriptor>,
}

#[derive(Clone)]
pub struct CallbackDescriptor {
    pub body: SourceSpan,
    pub parameters: Vec<CallbackParameter>,
    pub barriers: Vec<CallbackBarrier>,
    pub boundaries: Vec<SourceSpan>,
    pub outer_capture: OuterCapture,
}

#[derive(Clone)]
pub struct CallbackParameter {
    /// Keep this exact grammar span so declaration identity remains precise.
    pub declaration: SourceSpan,
    /// The name as emitted by the reference extractor.
    pub name: String,
    /// The parameter's own type annotation as written, when the source carries
    /// one. It types the binding directly; an unannotated parameter is typed
    /// contextually from the callee.
    pub annotation: Option<String>,
}

#[derive(Clone)]
pub struct CallbackBarrier {
    pub name: String,
    pub range: SourceSpan,
}

#[derive(Clone)]
pub enum OuterCapture {
    /// Reads may bind to an enclosing callback declaration.
    Transparent,
    /// Reads may bind outward only to the listed names. An empty list fences
    /// all outer captures, which represents a callback with an explicit empty capture list.
    Explicit(Vec<String>),
}

struct BoundCallback {
    body: SourceSpan,
    parameters: Vec<(String, BindingId)>,
    barriers: Vec<(String, SourceSpan)>,
    boundaries: Vec<SourceSpan>,
    outer_capture: OuterCapture,
}

/// Capture a graph whose only bindings are callback parameters. The returned
/// graph is deliberately separate from `FlowMeta::lexical`: opting into it
/// does not change legacy local-inference or global-source semantics.
pub(crate) fn capture(
    root: Node,
    _source: &[u8],
    adapter: Option<&CallbackLexicalAdapter>,
    refs: &[ExtractedRef],
) -> Option<LexicalBindings> {
    let adapter = adapter?;
    let mut callbacks = Vec::new();
    collect_callbacks(root, adapter, _source, &mut callbacks);
    if callbacks.is_empty() {
        return None;
    }

    callbacks.sort_by_key(|callback: &CallbackDescriptor| {
        (callback.body.start, std::cmp::Reverse(callback.body.end))
    });
    let mut graph = LexicalBindings::default();
    let root_scope = graph.add_scope(None, root.start_byte() as u32, root.end_byte() as u32, true);
    let mut scopes: Vec<(SourceSpan, ScopeId)> = Vec::new();
    let mut bound = Vec::new();

    for callback in callbacks {
        let parent = scopes
            .iter()
            .filter(|(body, _)| contains(*body, callback.body.start))
            .min_by_key(|(body, _)| body.end - body.start)
            .map(|(_, scope)| *scope)
            .unwrap_or(root_scope);
        let scope = graph.add_scope(Some(parent), callback.body.start, callback.body.end, true);
        let mut parameters = Vec::new();
        for parameter in callback.parameters {
            let name_id = graph.intern(&parameter.name);
            let binding = graph.declare(
                scope,
                name_id,
                callback.body.start,
                parameter.annotation.clone(),
            );
            graph.declarations.insert(parameter.declaration, binding);
            parameters.push((parameter.name, binding));
        }
        let barriers = callback
            .barriers
            .into_iter()
            .map(|barrier| (barrier.name, barrier.range))
            .collect();
        scopes.push((callback.body, scope));
        bound.push(BoundCallback {
            body: callback.body,
            parameters,
            barriers,
            boundaries: callback.boundaries,
            outer_capture: callback.outer_capture,
        });
    }

    // Extracted references are an attested bridge for the later callback
    // cache. Their chain root is authoritative even where the extractor's
    // address points at a member selector.
    for reference in refs {
        let Some(name) = reference
            .chain
            .as_ref()
            .and_then(|chain| chain.segments.first())
            .map(|segment| segment.name.as_str())
        else {
            continue;
        };
        let byte = reference.byte_offset;
        let mut containing: Vec<_> = bound
            .iter()
            .filter(|callback| contains(callback.body, byte))
            .collect();
        containing.sort_by_key(|callback| callback.body.end - callback.body.start);
        for callback in containing {
            if callback
                .boundaries
                .iter()
                .any(|boundary| contains(*boundary, byte))
                || callback
                    .barriers
                    .iter()
                    .any(|(barrier, range)| barrier == name && contains(*range, byte))
            {
                break;
            }
            if let Some((_, binding)) = callback
                .parameters
                .iter()
                .find(|(parameter, _)| parameter == name)
            {
                graph.references.insert(byte, *binding);
                break;
            }
            if matches!(&callback.outer_capture, OuterCapture::Explicit(captures) if !captures.iter().any(|capture| capture == name))
            {
                break;
            }
        }
    }
    Some(graph)
}

fn collect_callbacks(
    node: Node,
    adapter: &CallbackLexicalAdapter,
    source: &[u8],
    callbacks: &mut Vec<CallbackDescriptor>,
) {
    if let Some(callback) = (adapter.describe)(node, source) {
        callbacks.push(callback);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_callbacks(child, adapter, source, callbacks);
    }
}

pub fn span(node: Node) -> SourceSpan {
    SourceSpan {
        start: node.start_byte() as u32,
        end: node.end_byte() as u32,
    }
}

fn contains(range: SourceSpan, byte: u32) -> bool {
    range.start <= byte && byte < range.end
}

#[cfg(test)]
#[path = "callback_lexical_tests.rs"]
mod tests;
