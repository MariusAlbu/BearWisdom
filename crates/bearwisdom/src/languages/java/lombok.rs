// =============================================================================
// java/lombok.rs — synthesize the accessor methods Lombok generates
//
// Lombok's @Data/@Value/@Getter/@Setter expand at compile time to getX()/setX()
// (isX() for a primitive boolean) that never appear in source text, so a ref to
// `user.getName()` goes unresolved without synthesis. This recognizer reads the
// already-extracted class symbols + their fields plus the annotation refs (Java
// emits a TYPE-level annotation as a TypeRef whose source_symbol_index points at
// the class) and emits those accessors, parented to the owning class.
//
// Type-level annotations only. An annotation and a field whose TYPE is named
// like a Lombok annotation are BOTH `TypeRef`s sourced at the class (Java
// attributes a field's type to its enclosing class), so source-kind alone can't
// tell `@Value` from `private Value price;`. The reliable discriminator: an
// annotation ref's `byte_offset` points at the `@` (the extractor stamps the
// `marker_annotation`/`annotation` node start), a type ref points at the type
// identifier. Field-level @Getter/@Setter is a follow-on (those refs are sourced
// at the field). Also out of scope for this slice: @Builder's nested builder
// class, the @*Constructor family, and the AccessLevel / static / final nuances
// that suppress accessors.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use std::collections::{HashMap, HashSet};

/// Which accessors a Lombok annotation generates for a field.
#[derive(Clone, Copy, Default)]
struct Accessors {
    getter: bool,
    setter: bool,
}

impl Accessors {
    fn merge(self, other: Accessors) -> Accessors {
        Accessors {
            getter: self.getter || other.getter,
            setter: self.setter || other.setter,
        }
    }
}

/// Map a Lombok annotation name (bare or `lombok.`-qualified) to the accessors
/// it generates. `None` for any non-accessor annotation.
fn accessors_for(annotation: &str) -> Option<Accessors> {
    let bare = annotation.rsplit('.').next().unwrap_or(annotation);
    match bare {
        "Data" => Some(Accessors { getter: true, setter: true }),
        "Value" => Some(Accessors { getter: true, setter: false }), // immutable
        "Getter" => Some(Accessors { getter: true, setter: false }),
        "Setter" => Some(Accessors { getter: false, setter: true }),
        _ => None,
    }
}

pub(super) fn synthesize_lombok_accessors(
    source: &str,
    symbols: &[ExtractedSymbol],
    refs: &[ExtractedRef],
) -> Vec<ExtractedSymbol> {
    // 1. Collect accessor intent from type-level annotation refs. The annotation
    //    applies to every field of that class.
    let src = source.as_bytes();
    let mut class_intent: HashMap<&str, Accessors> = HashMap::new(); // class qname -> intent

    for r in refs {
        if r.kind != EdgeKind::TypeRef {
            continue;
        }
        // Discriminate an annotation from a like-named field type: the
        // annotation ref starts at the `@`.
        if src.get(r.byte_offset as usize) != Some(&b'@') {
            continue;
        }
        let Some(acc) = accessors_for(&r.target_name) else {
            continue;
        };
        let Some(sym) = symbols.get(r.source_symbol_index) else {
            continue;
        };
        if is_type_decl(sym.kind) {
            let e = class_intent.entry(sym.qualified_name.as_str()).or_default();
            *e = e.merge(acc);
        }
    }

    if class_intent.is_empty() {
        return Vec::new();
    }

    // 2. For each field of an annotated class, emit its accessors. A hand-written
    //    method of the same qualified name always wins.
    let existing: HashSet<&str> = symbols.iter().map(|s| s.qualified_name.as_str()).collect();
    let mut emitted: HashSet<String> = HashSet::new();
    let mut out: Vec<ExtractedSymbol> = Vec::new();

    for sym in symbols {
        if sym.kind != SymbolKind::Field {
            continue;
        }
        let Some(class_qname) = sym.scope_path.as_deref() else {
            continue;
        };
        let acc = class_intent.get(class_qname).copied().unwrap_or_default();
        if !acc.getter && !acc.setter {
            continue;
        }

        let field_type = field_type_of(sym);
        let cap = capitalize_first(&sym.name);
        if cap.is_empty() {
            continue;
        }
        let ty = ret_type(&field_type);

        if acc.getter {
            // A primitive `boolean` field gets `isX()`; everything else `getX()`.
            let name = if field_type == "boolean" {
                format!("is{cap}")
            } else {
                format!("get{cap}")
            };
            push_unique(
                &mut out,
                &mut emitted,
                &existing,
                make_method(&name, format!("{ty} {name}()"), class_qname, sym.start_line),
            );
        }
        if acc.setter {
            let name = format!("set{cap}");
            let sig = format!("void {}({} {})", name, ty, sym.name);
            push_unique(
                &mut out,
                &mut emitted,
                &existing,
                make_method(&name, sig, class_qname, sym.start_line),
            );
        }
    }

    out
}

fn is_type_decl(kind: SymbolKind) -> bool {
    matches!(kind, SymbolKind::Class | SymbolKind::Interface | SymbolKind::Enum)
}

/// A field's `signature` is `"{type} {name}"`; strip the trailing ` {name}` to
/// recover the declared type. Empty when the signature is absent/malformed.
fn field_type_of(field: &ExtractedSymbol) -> String {
    let Some(sig) = field.signature.as_deref() else {
        return String::new();
    };
    let suffix = format!(" {}", field.name);
    sig.strip_suffix(&suffix).unwrap_or(sig).trim().to_string()
}

/// Render a type for a synthesized signature; an unknown type reads as `Object`.
fn ret_type(t: &str) -> &str {
    if t.is_empty() {
        "Object"
    } else {
        t
    }
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Append `method` unless its qualified name already exists (hand-written wins)
/// or was already synthesized for this file.
fn push_unique(
    out: &mut Vec<ExtractedSymbol>,
    emitted: &mut HashSet<String>,
    existing: &HashSet<&str>,
    method: ExtractedSymbol,
) {
    if existing.contains(method.qualified_name.as_str()) {
        return;
    }
    if emitted.insert(method.qualified_name.clone()) {
        out.push(method);
    }
}

fn make_method(name: &str, signature: String, class_qname: &str, line: u32) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: format!("{class_qname}.{name}"),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: 0,
        end_col: 0,
        signature: Some(signature),
        doc_comment: None,
        scope_path: Some(class_qname.to_string()),
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}
