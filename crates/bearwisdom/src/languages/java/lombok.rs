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
// at the field, indistinguishable from a field type). @Builder synthesizes the
// `builder()` entry, the nested `{Class}Builder` class, a fluent setter per
// field, and `build()`. Out of scope: the @*Constructor family and the
// AccessLevel / static / final nuances that suppress accessors.
//
// Each synthesized method carries a return-type `TypeRef` (the field type for a
// getter, the builder qname for `builder()`/fluent setters, the class qname for
// `build()`) so it types through a chain the same way a real method does — a
// getter chain and the fluent builder chain both resolve end to end. Two known
// boundaries: void setters and primitive returns emit no ref (nothing to chain
// into), and a generic return like `List<User>` types to its head `List`, not
// the element `User` — typing through a generic method return needs
// `return_type_args` on the chain walker's method branch (generic-engine work,
// not Lombok-specific).
// =============================================================================

use crate::languages::Synthesized;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use std::collections::{HashMap, HashSet};

/// What a Lombok annotation generates for a class.
#[derive(Clone, Copy, Default)]
struct Accessors {
    getter: bool,
    setter: bool,
    builder: bool,
}

impl Accessors {
    fn merge(self, other: Accessors) -> Accessors {
        Accessors {
            getter: self.getter || other.getter,
            setter: self.setter || other.setter,
            builder: self.builder || other.builder,
        }
    }
}

/// Map a Lombok annotation name (bare or `lombok.`-qualified) to what it
/// generates. `None` for any annotation that synthesizes nothing here.
fn accessors_for(annotation: &str) -> Option<Accessors> {
    let bare = annotation.rsplit('.').next().unwrap_or(annotation);
    match bare {
        "Data" => Some(Accessors { getter: true, setter: true, ..Default::default() }),
        "Value" => Some(Accessors { getter: true, ..Default::default() }), // immutable
        "Getter" => Some(Accessors { getter: true, ..Default::default() }),
        "Setter" => Some(Accessors { setter: true, ..Default::default() }),
        "Builder" => Some(Accessors { builder: true, ..Default::default() }),
        _ => None,
    }
}

pub(super) fn synthesize_lombok_accessors(
    source: &str,
    symbols: &[ExtractedSymbol],
    refs: &[ExtractedRef],
) -> Synthesized {
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
        return Synthesized::default();
    }

    // 2. For each field of an annotated class, emit its accessors. A hand-written
    //    method of the same qualified name always wins. A getter carries a
    //    return-type ref (its field type) so a chain types through it.
    let mut emit = Emit::new(symbols);

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
            let sym = make_synth(&name, SymbolKind::Method, format!("{ty} {name}()"), class_qname, sym.start_line);
            emit.push(sym, Some(&field_type));
        }
        if acc.setter {
            // Lombok setters return void — no return-type ref.
            let name = format!("set{cap}");
            let sig = format!("void {}({} {})", name, ty, sym.name);
            emit.push(make_synth(&name, SymbolKind::Method, sig, class_qname, sym.start_line), None);
        }
    }

    // 3. For each @Builder class, synthesize `builder()` + the nested builder
    //    class with a fluent setter per field and `build()`. Lombok names the
    //    builder `{ClassName}Builder`, nested under the class.
    let builder_classes: Vec<String> = class_intent
        .iter()
        .filter(|(_, a)| a.builder)
        .map(|(q, _)| q.to_string())
        .collect();
    for class_qname in &builder_classes {
        emit_builder(class_qname, symbols, &mut emit);
    }

    Synthesized { symbols: emit.out, refs: emit.refs }
}

/// Emit the `@Builder` machinery for one class: a static `builder()` returning
/// the nested `{Simple}Builder` class, that class, a fluent setter per field
/// (named after the field, returning the builder), and `build()` returning the
/// owning class. The methods carry return-type refs (the builder qname, or the
/// class qname for `build()`) so the fluent chain types end to end.
fn emit_builder(class_qname: &str, symbols: &[ExtractedSymbol], emit: &mut Emit) {
    let simple = class_qname.rsplit('.').next().unwrap_or(class_qname);
    let builder_name = format!("{simple}Builder");
    let builder_qname = format!("{class_qname}.{builder_name}");
    let line = symbols
        .iter()
        .find(|s| s.qualified_name == class_qname)
        .map_or(0, |s| s.start_line);

    // `static {Simple}Builder builder()` on the owning class → returns the builder.
    let builder_method = make_synth("builder", SymbolKind::Method, format!("{builder_name} builder()"), class_qname, line);
    emit.push(builder_method, Some(&builder_qname));
    // The nested builder class (no return type).
    emit.push(
        make_synth(&builder_name, SymbolKind::Class, format!("class {builder_name}"), class_qname, line),
        None,
    );
    // A fluent setter per field, returning the builder for chaining.
    for sym in symbols {
        if sym.kind != SymbolKind::Field || sym.scope_path.as_deref() != Some(class_qname) {
            continue;
        }
        let ty = field_type_of(sym);
        let sig = format!("{} {}({} {})", builder_name, sym.name, ret_type(&ty), sym.name);
        emit.push(
            make_synth(&sym.name, SymbolKind::Method, sig, &builder_qname, sym.start_line),
            Some(&builder_qname),
        );
    }
    // `{Simple} build()` on the builder class → returns the owning class.
    emit.push(
        make_synth("build", SymbolKind::Method, format!("{simple} build()"), &builder_qname, line),
        Some(class_qname),
    );
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

/// Accumulates synthesized symbols + their return-type refs, skipping any whose
/// qualified name already exists (hand-written wins) or was already synthesized.
struct Emit<'a> {
    out: Vec<ExtractedSymbol>,
    refs: Vec<ExtractedRef>,
    emitted: HashSet<String>,
    existing: HashSet<&'a str>,
}

impl<'a> Emit<'a> {
    fn new(symbols: &'a [ExtractedSymbol]) -> Self {
        Self {
            out: Vec::new(),
            refs: Vec::new(),
            emitted: HashSet::new(),
            existing: symbols.iter().map(|s| s.qualified_name.as_str()).collect(),
        }
    }

    /// Push `sym`; when `return_type` is a non-primitive, non-empty type, also
    /// emit its return-type `TypeRef` sourced at the new symbol's index (so the
    /// chain walker types a call to it). `source_symbol_index` is RELATIVE to
    /// `out` — `parse_file` rebases it onto the file table.
    fn push(&mut self, sym: ExtractedSymbol, return_type: Option<&str>) {
        if self.existing.contains(sym.qualified_name.as_str()) {
            return;
        }
        if !self.emitted.insert(sym.qualified_name.clone()) {
            return;
        }
        let line = sym.start_line;
        self.out.push(sym);
        let idx = self.out.len() - 1;
        if let Some(head) = return_type.map(type_head) {
            if !head.is_empty() && !is_primitive(head) {
                self.refs.push(return_type_ref(idx, head, line));
            }
        }
    }
}

/// The bare head type usable as a `TypeRef` target: strips generic args and
/// array brackets. `List<User>` → `List`, `String[]` → `String`.
fn type_head(t: &str) -> &str {
    let t = t.split('<').next().unwrap_or(t);
    let t = t.split('[').next().unwrap_or(t);
    t.trim()
}

fn is_primitive(t: &str) -> bool {
    matches!(
        t,
        "boolean" | "byte" | "short" | "int" | "long" | "char" | "float" | "double" | "void"
    )
}

fn return_type_ref(source_symbol_index: usize, type_name: &str, line: u32) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index,
        target_name: type_name.to_string(),
        kind: EdgeKind::TypeRef,
        line,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

/// Build a synthesized symbol named `name` under `scope_qname` (its
/// `qualified_name` is `scope_qname.name`, its `scope_path` is `scope_qname`).
fn make_synth(
    name: &str,
    kind: SymbolKind,
    signature: String,
    scope_qname: &str,
    line: u32,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: format!("{scope_qname}.{name}"),
        kind,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: 0,
        end_col: 0,
        signature: Some(signature),
        doc_comment: None,
        scope_path: Some(scope_qname.to_string()),
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}
