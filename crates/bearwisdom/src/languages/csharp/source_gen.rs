// =============================================================================
// csharp/source_gen.rs — synthesize the members the C# compiler generates for
// positional records
//
// A positional record `record Point(int X, int Y)` makes the compiler generate
// a `Deconstruct(out int X, out int Y)` that never appears in source text, so a
// ref to `point.Deconstruct(out _, out _)` (or the positional pattern
// `var (a, b) = point`) goes unresolved without synthesis. This recognizer reads
// the already-extracted symbols and emits that Deconstruct, parented to the
// record.
//
// Detection: a record is extracted as `SymbolKind::Class` with signature
// `"class {Name}..."` — indistinguishable from a real class by kind or
// signature. The only discriminator at synthesis time (where we have `source` +
// flat `symbols`, not the CST) is source text: the class header line contains
// the `record` keyword as a whole word before the type name. This mirrors
// Kotlin's `is_data_class`, which scans for the `data` modifier the same way.
//
// Positional properties: the extractor emits each positional parameter AND each
// body property as a `SymbolKind::Property` with `signature: Some("{type}
// {name}")` and `scope_path == record_qname` — identical shape. The compiler
// generates Deconstruct ONLY for positional parameters, so the recognizer keeps
// only the properties whose source position falls inside the record's positional
// parameter list (the parenthesized group right after the type name, before any
// `{`). Body properties (`record Person { string Name {...} }`) sit after the
// `{` and are excluded — a record with no parameter list yields no Deconstruct.
// The recognizer reads these to build Deconstruct's `out` parameter list — it
// does NOT re-emit properties (the chain `dto.Category.Id` rides the property
// signature the extractor already produced).
//
// Deconstruct returns void, so it carries no return-type `TypeRef` — the same
// convention the Lombok/derive/data-class recognizers use for void/primitive
// returns.
// =============================================================================

use crate::languages::Synthesized;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use std::collections::HashSet;

/// The plugin synthesis seam: run every C# source-generator recognizer and merge
/// their output. Each recognizer reads the already-extracted symbols + their
/// source text and emits the members the corresponding generator would produce.
pub(super) fn synthesize_symbols(
    source: &str,
    symbols: &[ExtractedSymbol],
    refs: &[ExtractedRef],
) -> Synthesized {
    let mut out = synthesize_record_members(source, symbols, refs);
    merge(&mut out, synthesize_mvvm_members(source, symbols, refs));
    merge(&mut out, synthesize_observable_object_members(source, symbols, refs));
    out
}

/// Append `extra` onto `out`, rebasing `extra`'s refs (whose `source_symbol_index`
/// is relative to `extra.symbols`) onto the merged symbol list.
fn merge(out: &mut Synthesized, extra: Synthesized) {
    let base = out.symbols.len();
    out.symbols.extend(extra.symbols);
    for mut r in extra.refs {
        r.source_symbol_index += base;
        out.refs.push(r);
    }
}

pub(super) fn synthesize_record_members(
    source: &str,
    symbols: &[ExtractedSymbol],
    refs: &[ExtractedRef],
) -> Synthesized {
    let lines: Vec<&str> = source.lines().collect();

    let existing: HashSet<&str> = symbols.iter().map(|s| s.qualified_name.as_str()).collect();
    let mut emitted: HashSet<String> = HashSet::new();
    let mut out_symbols: Vec<ExtractedSymbol> = Vec::new();

    for (idx, record_sym) in symbols.iter().enumerate() {
        if record_sym.kind != SymbolKind::Class || !is_record(record_sym, &lines) {
            continue;
        }
        let record_qname = record_sym.qualified_name.as_str();

        // The record's positional parameter-list span, or skip when absent
        // (a body-only record `record Person { ... }` generates no Deconstruct).
        let Some(span) = param_list_span(record_sym, &lines) else {
            continue;
        };

        let own_params = positional_out_params(record_qname, &span, symbols, &lines);
        if own_params.is_empty() {
            continue;
        }

        // A derived positional record's Deconstruct lists the base record's
        // positional members FIRST, then its own (the real C# rule). The base
        // contributes only when it resolves to an in-project positional record;
        // an external/unresolved or non-record base contributes nothing (the
        // derived params stand alone — widening-only, no over-reach).
        let mut params: Vec<OutParam> = base_positional_params(idx, refs, symbols, &lines);
        for p in own_params {
            if !params.iter().any(|seen| seen.name == p.name) {
                params.push(p);
            }
        }

        let qname = format!("{record_qname}.Deconstruct");
        if existing.contains(qname.as_str()) || !emitted.insert(qname.clone()) {
            continue;
        }

        // `void Deconstruct(out T1 P1, out T2 P2, ...)`.
        let rendered: Vec<String> =
            params.iter().map(|p| format!("out {} {}", p.ty, p.name)).collect();
        let signature = format!("void Deconstruct({})", rendered.join(", "));
        out_symbols.push(make_synth(
            "Deconstruct",
            SymbolKind::Method,
            signature,
            record_qname,
            record_sym.start_line,
        ));
    }

    Synthesized { symbols: out_symbols, refs: Vec::new() }
}

/// One `out`-parameter of a synthesized Deconstruct: a positional property's
/// declared type and name.
struct OutParam {
    ty: String,
    name: String,
}

/// The positional `out`-parameters of the record whose qualified name is
/// `record_qname`, in declaration order. Keeps only properties whose start
/// position falls inside the record's positional parameter-list `span` — body
/// properties (`record Person { ... }`) share the same kind and scope_path but
/// sit after the `{`.
fn positional_out_params(
    record_qname: &str,
    span: &ParamSpan,
    symbols: &[ExtractedSymbol],
    lines: &[&str],
) -> Vec<OutParam> {
    symbols
        .iter()
        .filter(|s| {
            s.kind == SymbolKind::Property
                && s.scope_path.as_deref() == Some(record_qname)
                && span.contains(s.start_line, s.start_col, lines)
        })
        .map(|p| OutParam { ty: property_type(p), name: p.name.clone() })
        .collect()
}

/// The base record's positional `out`-parameters for the derived record at
/// symbol index `derived_idx`, or empty when there is no in-project positional
/// record base. The base is read from the derived symbol's `Inherits` ref
/// (matched by simple name against the extracted positional records), so an
/// external/unresolved base or a non-record base contributes nothing.
fn base_positional_params(
    derived_idx: usize,
    refs: &[ExtractedRef],
    symbols: &[ExtractedSymbol],
    lines: &[&str],
) -> Vec<OutParam> {
    let Some(base_name) = refs.iter().find_map(|r| {
        (r.source_symbol_index == derived_idx && r.kind == EdgeKind::Inherits)
            .then(|| r.target_name.as_str())
    }) else {
        return Vec::new();
    };
    // The Inherits target is a simple name; match it against an extracted record
    // by its short name. (A namespace-qualified collision across two records of
    // the same short name is ambiguous without import-scope; the first match is
    // taken — a bounded reach, never a false bind since both would be records.)
    let Some(base_sym) = symbols
        .iter()
        .find(|s| s.kind == SymbolKind::Class && s.name == base_name && is_record(s, lines))
    else {
        return Vec::new();
    };
    let Some(base_span) = param_list_span(base_sym, lines) else {
        return Vec::new();
    };
    positional_out_params(base_sym.qualified_name.as_str(), &base_span, symbols, lines)
}

// =============================================================================
// MVVM Community Toolkit source generators
//
// `[ObservableProperty]` on a backing field and `[RelayCommand]` on a method are
// the two most common MVVM Toolkit generators, and their output is deterministic
// (no Roslyn needed):
//
//   [ObservableProperty] private string _firstName;
//     → public string FirstName { get; set; }
//       partial void OnFirstNameChanging(string value);
//       partial void OnFirstNameChanged(string value);
//
//   [RelayCommand] private void Save() {}
//     → public IRelayCommand SaveCommand { get; }
//   [RelayCommand] private async Task SaveAsync() {}
//     → public IAsyncRelayCommand SaveAsyncCommand { get; }
//
// The attribute→member mappings are library-level facts on this seam, the same
// category as Lombok @Data→getX and the record Deconstruct — not is_*_builtin
// name tables.
//
// Discriminator: the C# extractor does not emit a ref for a field-level
// attribute (the `field_declaration` arm omits decorator extraction), so the
// signal is a source-text scan of the attribute lines that sit on or above the
// symbol's start line. A field symbol's start position points at the
// `variable_declarator` (after the attribute), so the `[ObservableProperty]`
// attribute is on a contiguous line above. A method symbol's start position is
// the `method_declaration` node start, which already covers any attribute list,
// so the `[RelayCommand]` token is found from that line. `attribute_token_above`
// matches the bracketed attribute as a whole token so a comment or a type whose
// name merely contains the text is rejected.
//
// Each synthesized member carries its return-type `TypeRef` (the field-type head
// for a property, the command-interface name for a command) so a chain types
// through it the same way a real member does. A primitive field type emits no
// ref (the Lombok/record convention). The command-interface refs resolve only
// when the CommunityToolkit.Mvvm externals are hydrated; unhydrated they land as
// an external unresolved target, not a false bind.
// =============================================================================

pub(super) fn synthesize_mvvm_members(
    source: &str,
    symbols: &[ExtractedSymbol],
    _refs: &[ExtractedRef],
) -> Synthesized {
    let lines: Vec<&str> = source.lines().collect();
    let mut emit = MvvmEmit::new(symbols);

    for sym in symbols {
        match sym.kind {
            SymbolKind::Field if attribute_token_above(sym, &lines, "ObservableProperty") => {
                synthesize_observable_property(sym, &mut emit);
            }
            SymbolKind::Method if attribute_token_above(sym, &lines, "RelayCommand") => {
                synthesize_relay_command(sym, &lines, &mut emit);
            }
            _ => {}
        }
    }

    Synthesized { symbols: emit.out, refs: emit.refs }
}

/// `[ObservableProperty] private {Type} _name;` → `public {Type} Name {get;set;}`
/// plus the `partial void On{Name}Changing/Changed(...)` hooks. The property
/// carries a return-type ref to the field-type head (unless primitive) so a chain
/// through it types to the field type's members. The hooks are void Methods (no
/// ref) so a call to them resolves to a member instead of going unresolved.
fn synthesize_observable_property(field: &ExtractedSymbol, emit: &mut MvvmEmit) {
    let Some(class_qname) = field.scope_path.as_deref() else {
        return;
    };
    let field_type = field_type_of(field);
    let prop_name = observable_property_name(&field.name);
    // A transform that yields nothing usable (empty, or unchanged from the field
    // name so it would collide with the field) emits nothing rather than a wrong
    // bind.
    if prop_name.is_empty() || prop_name == field.name {
        return;
    }

    let ty = if field_type.is_empty() { "object" } else { field_type.as_str() };
    let prop = make_synth(
        &prop_name,
        SymbolKind::Property,
        format!("{ty} {prop_name}"),
        class_qname,
        field.start_line,
    );
    emit.push(prop, Some(&field_type));

    // `partial void On{Name}Changing(value)` / `On{Name}Changed(value)`.
    for hook in [format!("On{prop_name}Changing"), format!("On{prop_name}Changed")] {
        let sig = format!("void {hook}({ty} value)");
        emit.push(make_synth(&hook, SymbolKind::Method, sig, class_qname, field.start_line), None);
    }
}

/// `[RelayCommand] {void|Task} Foo()` → `public {IRelayCommand|IAsyncRelayCommand}
/// FooCommand {get;}`. The Toolkit appends the `Command` suffix to the method
/// name and uses `IAsyncRelayCommand` when the method is async / Task-returning.
/// The command property carries a return-type ref to the interface so
/// `.Command.Execute(...)` / `.CanExecute(...)` resolve against it.
fn synthesize_relay_command(method: &ExtractedSymbol, lines: &[&str], emit: &mut MvvmEmit) {
    let Some(class_qname) = method.scope_path.as_deref() else {
        return;
    };
    if method.name.is_empty() {
        return;
    }
    let command_name = format!("{}Command", method.name);
    let iface = if is_async_command(method, lines) {
        "IAsyncRelayCommand"
    } else {
        "IRelayCommand"
    };
    let prop = make_synth(
        &command_name,
        SymbolKind::Property,
        format!("{iface} {command_name}"),
        class_qname,
        method.start_line,
    );
    emit.push(prop, Some(iface));
}

// =============================================================================
// MVVM Community Toolkit [ObservableObject] / ObservableObject base class
//
// `[ObservableObject]`, `[INotifyPropertyChanged]`, or inheriting one of the
// Toolkit's observable base classes (`ObservableObject` / `ObservableValidator`
// / `ObservableRecipient`) injects the INotifyPropertyChanged helper surface that
// every hand-written property setter calls:
//
//   protected bool SetProperty<T>(ref T field, T newValue, ...)
//   protected void OnPropertyChanged(...)
//   protected void OnPropertyChanging(...)
//
// These three are the surface common to all three base classes / the attribute,
// so they synthesize whenever the host is recognized. Each returns bool/void →
// no return-type ref (the void/primitive convention), so this pass adds symbols
// only, no refs.
//
// Discrimination is ref-based (unlike the field-level [ObservableProperty]): a
// class-level attribute emits a `TypeRef` ref at the class symbol's index, and a
// base class emits an `Inherits` ref — both carried in `refs`. The class symbol's
// position in `symbols` is its `source_symbol_index`, so a ref pointing at that
// index whose target names the attribute / an MVVM base marks the host. The base
// name is matched as a whole head (generics/namespace stripped), so an unrelated
// base or a like-named type is rejected.
//
// ObservableValidator and ObservableRecipient inherit ObservableObject and add
// their own public surface on top of the common change-notification members:
//
//   ObservableValidator → bool HasErrors, ValidateProperty(...),
//     ValidateAllProperties(), ClearAllErrors(), GetErrors(...), TrySetProperty(...)
//   ObservableRecipient → IMessenger Messenger, bool IsActive, Broadcast(...),
//     OnActivated(), OnDeactivated()
//
// These extras are gated on the corresponding BASE CLASS specifically — the
// `[ObservableObject]`/`[INotifyPropertyChanged]` attributes inject only the
// common surface, never validation or messaging. `Messenger` returns the
// `IMessenger` interface (a complex type) so it carries a return-type ref the
// same way an `[ObservableProperty]` of a class type does; every other extra is
// bool/void → no ref.
//
// Deferred: `[NotifyPropertyChangedFor]`/`[NotifyCanExecuteChangedFor]` generate
// change-notification CALLS inside generated setters, not new callable members —
// nothing sound to synthesize as a symbol.
// =============================================================================

/// Class-level attribute names that inject the change-notification surface.
const OBSERVABLE_OBJECT_ATTRS: &[&str] = &["ObservableObject", "INotifyPropertyChanged"];

/// MVVM Toolkit base classes that carry the change-notification surface.
const OBSERVABLE_OBJECT_BASES: &[&str] =
    &["ObservableObject", "ObservableValidator", "ObservableRecipient"];

/// The change-notification surface common to every observable host — synthesized
/// whenever any host marker (attribute or base) is recognized. `bool SetProperty`
/// is the headline; every hand-written property setter calls it.
const COMMON_NOTIFY_MEMBERS: &[(&str, &str)] = &[
    ("SetProperty", "bool SetProperty(ref T field, T newValue)"),
    ("OnPropertyChanged", "void OnPropertyChanged()"),
    ("OnPropertyChanging", "void OnPropertyChanging()"),
];

/// The data-validation surface `ObservableValidator` adds. All bool/void → no
/// return-type ref.
const VALIDATOR_MEMBERS: &[(&str, &str)] = &[
    ("HasErrors", "bool HasErrors"),
    ("ValidateProperty", "void ValidateProperty(object value)"),
    ("ValidateAllProperties", "void ValidateAllProperties()"),
    ("ClearAllErrors", "void ClearAllErrors()"),
    ("GetErrors", "IEnumerable GetErrors(string propertyName)"),
    ("TrySetProperty", "bool TrySetProperty(ref T field, T newValue)"),
];

/// The messaging surface `ObservableRecipient` adds. `Messenger` returns the
/// `IMessenger` interface (complex → carries a return-type ref); the rest are
/// bool/void → no ref.
const RECIPIENT_MEMBERS: &[(&str, &str)] = &[
    ("IsActive", "bool IsActive"),
    ("Broadcast", "void Broadcast(T oldValue, T newValue, string propertyName)"),
    ("OnActivated", "void OnActivated()"),
    ("OnDeactivated", "void OnDeactivated()"),
];

/// `Messenger`'s interface return type — emitted as a return-type ref so a chain
/// `vm.Messenger.Send(...)` types through to the (hydrated) interface.
const MESSENGER_RETURN: &str = "IMessenger";

pub(super) fn synthesize_observable_object_members(
    _source: &str,
    symbols: &[ExtractedSymbol],
    refs: &[ExtractedRef],
) -> Synthesized {
    let mut emit = MvvmEmit::new(symbols);

    for (idx, sym) in symbols.iter().enumerate() {
        if sym.kind != SymbolKind::Class {
            continue;
        }
        let Some(host) = observable_object_host(idx, refs) else {
            continue;
        };
        let class_qname = sym.qualified_name.as_str();

        for (name, sig) in COMMON_NOTIFY_MEMBERS {
            emit.push(
                make_synth(name, SymbolKind::Method, sig.to_string(), class_qname, sym.start_line),
                None,
            );
        }
        if host.validator {
            for (name, sig) in VALIDATOR_MEMBERS {
                let kind = member_kind(sig);
                emit.push(
                    make_synth(name, kind, sig.to_string(), class_qname, sym.start_line),
                    None,
                );
            }
        }
        if host.recipient {
            let messenger = make_synth(
                "Messenger",
                SymbolKind::Property,
                format!("{MESSENGER_RETURN} Messenger"),
                class_qname,
                sym.start_line,
            );
            emit.push(messenger, Some(MESSENGER_RETURN));
            for (name, sig) in RECIPIENT_MEMBERS {
                let kind = member_kind(sig);
                emit.push(
                    make_synth(name, kind, sig.to_string(), class_qname, sym.start_line),
                    None,
                );
            }
        }
    }

    Synthesized { symbols: emit.out, refs: emit.refs }
}

/// Which base-specific MVVM surfaces a recognized observable host carries. The
/// common change-notification surface is unconditional for any host; these flags
/// gate the validator/recipient extras on the corresponding base class.
struct ObservableHost {
    validator: bool,
    recipient: bool,
}

/// Classify the class at symbol index `class_idx` as an MVVM observable host, or
/// `None` when it carries no host marker. A host carries either an
/// `[ObservableObject]`/`[INotifyPropertyChanged]` attribute (a `TypeRef` ref at
/// its index) or an MVVM observable base (an `Inherits` ref at its index). The
/// `validator`/`recipient` flags are set by the corresponding BASE specifically —
/// the attributes inject only the common surface. Targets are matched as a whole
/// head name (generics/namespace stripped).
fn observable_object_host(class_idx: usize, refs: &[ExtractedRef]) -> Option<ObservableHost> {
    let mut is_host = false;
    let mut validator = false;
    let mut recipient = false;
    for r in refs {
        if r.source_symbol_index != class_idx {
            continue;
        }
        // The target may be namespace-qualified (`CommunityToolkit.Mvvm...
        // .ObservableObject`); compare the final dotted segment of the head.
        let head = type_head(&r.target_name);
        let last = head.rsplit('.').next().unwrap_or(head);
        match r.kind {
            EdgeKind::TypeRef if OBSERVABLE_OBJECT_ATTRS.contains(&last) => is_host = true,
            EdgeKind::Inherits if OBSERVABLE_OBJECT_BASES.contains(&last) => {
                is_host = true;
                match last {
                    "ObservableValidator" => validator = true,
                    "ObservableRecipient" => recipient = true,
                    _ => {}
                }
            }
            _ => {}
        }
    }
    is_host.then_some(ObservableHost { validator, recipient })
}

/// The `SymbolKind` for a synthesized member from its signature: a parenthesized
/// signature is a `Method`, a bare `{type} {name}` is a `Property`. These members
/// are a fixed fact set, so the presence of a parameter list is an exact
/// discriminator.
fn member_kind(signature: &str) -> SymbolKind {
    if signature.contains('(') {
        SymbolKind::Method
    } else {
        SymbolKind::Property
    }
}

// ---------------------------------------------------------------------------
// MVVM helpers
// ---------------------------------------------------------------------------

/// The MVVM Toolkit property name for an `[ObservableProperty]` backing field:
/// strip a single leading underscore, then uppercase the first remaining letter
/// (`_firstName`→`FirstName`, `name`→`Name`, `_x`→`X`). Empty when the field name
/// is empty or only underscores.
fn observable_property_name(field_name: &str) -> String {
    let stripped = field_name.strip_prefix('_').unwrap_or(field_name);
    capitalize_first(stripped)
}

/// Whether a `[RelayCommand]` method is async, deciding `IAsyncRelayCommand` vs
/// `IRelayCommand`. The Toolkit treats `async` methods and `Task`/`ValueTask`
/// returners as async commands. `async` is a modifier (not part of the extracted
/// return type), so scan the method's own start line for the keyword; the
/// signature's return-type head covers the Task-returning non-async form.
fn is_async_command(method: &ExtractedSymbol, lines: &[&str]) -> bool {
    if let Some(line) = lines.get(method.start_line as usize) {
        if line.split(|c: char| !c.is_alphanumeric()).any(|w| w == "async") {
            return true;
        }
    }
    let ret = method
        .signature
        .as_deref()
        .and_then(|s| s.split_whitespace().next())
        .unwrap_or("");
    let head = type_head(ret);
    matches!(head, "Task" | "ValueTask")
}

/// Whether the bracketed attribute `attr` appears as a whole token on the
/// symbol's start line or on a contiguous run of attribute lines directly above
/// it. A field symbol's start line is the declarator (the attribute sits above);
/// a method symbol's start line already covers its attribute list. The scan stops
/// at the first line that is neither the start line nor attribute-shaped so an
/// attribute on an earlier sibling can't leak down.
fn attribute_token_above(sym: &ExtractedSymbol, lines: &[&str], attr: &str) -> bool {
    let start = sym.start_line as usize;
    // The start line itself (covers an inline `[Attr] private void M()` and a
    // method whose node start already spans the attribute list).
    if line_has_attribute(lines.get(start).copied(), attr) {
        return true;
    }
    // Contiguous attribute lines directly above the declarator.
    let mut i = start;
    while i > 0 {
        i -= 1;
        let line = lines.get(i).copied().unwrap_or("").trim();
        if line.is_empty() {
            break;
        }
        if !is_attribute_line(line) {
            break;
        }
        if line_has_attribute(Some(line), attr) {
            return true;
        }
    }
    false
}

/// Whether `line` is a *standalone* C# attribute line — one or more `[...]`
/// attribute groups and nothing else. Used to bound the upward attribute scan so
/// only genuine attribute lines above a declarator are walked. An inline
/// declaration like `[ObservableProperty] private string _x;` is `[...]`-shaped at
/// its start but carries a member declarator after the `]`, so the non-bracket
/// remainder is non-empty and this returns false — a plain field directly below
/// such a line cannot leak the attribute upward.
fn is_attribute_line(line: &str) -> bool {
    let t = line.trim_start();
    if !t.starts_with('[') {
        return false;
    }
    // Strip every `[...]` group; the line is standalone-attribute iff the
    // remainder (the text outside any bracket group) is all whitespace.
    let mut depth = 0i32;
    let mut had_close = false;
    for c in t.chars() {
        match c {
            '[' => depth += 1,
            ']' => {
                if depth > 0 {
                    depth -= 1;
                    had_close = true;
                }
            }
            _ if depth > 0 => {}
            _ if c.is_whitespace() => {}
            _ => return false,
        }
    }
    had_close && depth == 0
}

/// Whether `line` contains the attribute `attr` as a whole token inside a
/// bracketed attribute group. Splits on non-identifier characters so `[ObservableProperty]`
/// and `[ObservableProperty, ...]` match but `ObservablePropertyHolder` does not.
fn line_has_attribute(line: Option<&str>, attr: &str) -> bool {
    let Some(line) = line else {
        return false;
    };
    // Only consider text inside `[...]` so a type/identifier mention outside an
    // attribute group (e.g. a field of a like-named type) is ignored.
    let mut hay = String::new();
    let mut depth = 0i32;
    for c in line.chars() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            _ if depth > 0 => hay.push(c),
            _ => {}
        }
    }
    hay.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|w| w == attr)
}

/// A field's `signature` is `"{type} {name}"`; strip the trailing ` {name}` to
/// recover the declared type. Empty when absent/malformed.
fn field_type_of(field: &ExtractedSymbol) -> String {
    let Some(sig) = field.signature.as_deref() else {
        return String::new();
    };
    let suffix = format!(" {}", field.name);
    sig.strip_suffix(&suffix).unwrap_or(sig).trim().to_string()
}

/// The bare head type usable as a `TypeRef` target: strips generic args and
/// array brackets and a trailing nullable `?`. `List<User>` → `List`.
fn type_head(t: &str) -> &str {
    let t = t.split('<').next().unwrap_or(t);
    let t = t.split('[').next().unwrap_or(t);
    t.trim().trim_end_matches('?').trim()
}

/// C# primitives / scalar types that carry no useful chain member — a synthesized
/// member returning one of these emits no return-type ref (the Lombok/record
/// convention), avoiding a manufactured unresolved ref.
fn is_csharp_primitive(t: &str) -> bool {
    matches!(
        t,
        "bool" | "byte" | "sbyte" | "short" | "ushort" | "int" | "uint" | "long" | "ulong"
            | "float" | "double" | "decimal" | "char" | "string" | "object" | "void"
            | "nint" | "nuint"
    )
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
/// `source_symbol_index` on a pushed ref is RELATIVE to `out`; the caller rebases.
struct MvvmEmit<'a> {
    out: Vec<ExtractedSymbol>,
    refs: Vec<ExtractedRef>,
    emitted: HashSet<String>,
    existing: HashSet<&'a str>,
}

impl<'a> MvvmEmit<'a> {
    fn new(symbols: &'a [ExtractedSymbol]) -> Self {
        Self {
            out: Vec::new(),
            refs: Vec::new(),
            emitted: HashSet::new(),
            existing: symbols.iter().map(|s| s.qualified_name.as_str()).collect(),
        }
    }

    /// Push `sym`; when `return_type` is a non-primitive, non-empty type, emit its
    /// return-type `TypeRef` (head, stripped of generic args) so the chain walker
    /// types a call/access to it.
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
        if let Some(rt) = return_type {
            let head = type_head(rt);
            if !head.is_empty() && !is_csharp_primitive(head) {
                self.refs.push(return_type_ref(idx, head, line));
            }
        }
    }
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns true when `sym` (a `Class` symbol) is a `record` declaration.
///
/// Records and classes are both extracted as `SymbolKind::Class` with a
/// `"class {name}..."` signature, so the discriminator is source text. The
/// `record_declaration` node spans its attributes/modifiers, so the header may
/// start with `[Attr]` lines before the `record` keyword. Accumulate the header
/// from `start_line` up to and including the line holding the type name, then
/// check for a `record` token before that name. `record` is a contextual
/// keyword, so a whitespace-delimited word match excludes identifiers like
/// `RecordStore`.
fn is_record(sym: &ExtractedSymbol, lines: &[&str]) -> bool {
    let start = sym.start_line as usize;
    let end = (sym.end_line as usize).min(lines.len().saturating_sub(1));
    let name = sym.name.as_str();
    let mut header = String::new();
    for i in start..=end {
        let Some(line) = lines.get(i) else { break };
        header.push_str(line);
        header.push(' ');
        // The type name first appears once we reach the `record X` line.
        if header.contains(name) {
            break;
        }
    }
    // The discriminator is `record` appearing as a whole word before the name.
    let Some(name_pos) = header.find(name) else {
        return false;
    };
    header[..name_pos].split_whitespace().any(|w| w == "record")
}

/// The character span of a record's positional parameter list, as absolute
/// offsets into `source` (newlines counted as one char each).
struct ParamSpan {
    open: usize,  // offset just after the `(`
    close: usize, // offset of the matching `)`
}

impl ParamSpan {
    /// Whether a symbol at `(line, col)` starts inside the parameter list.
    fn contains(&self, line: u32, col: u32, lines: &[&str]) -> bool {
        let off = offset_of(line, col, lines);
        off >= self.open && off < self.close
    }
}

/// Locate the record's positional parameter list: the parenthesized group
/// immediately after the type name, before any body `{`. `None` for a record
/// with no parameter list (`record Person { ... }`). The scan begins at the
/// record name to skip any `(` in attributes/modifiers, and bails at the first
/// `{` so a body initializer can't be mistaken for a parameter list.
fn param_list_span(sym: &ExtractedSymbol, lines: &[&str]) -> Option<ParamSpan> {
    let chars: Vec<char> = lines.join("\n").chars().collect();
    let name_off = find_name_offset(&chars, sym.start_line, lines, sym.name.as_str())?;

    // First `(` after the name, with no intervening `{`.
    let mut i = name_off + sym.name.chars().count();
    let open = loop {
        let c = *chars.get(i)?;
        if c == '{' {
            return None;
        }
        if c == '(' {
            break i;
        }
        i += 1;
    };

    // Matching `)` by paren depth.
    let mut depth = 0usize;
    let mut j = open;
    let close = loop {
        match chars.get(j)? {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    break j;
                }
            }
            _ => {}
        }
        j += 1;
    };

    Some(ParamSpan { open: open + 1, close })
}

/// Absolute char offset of `name`'s first occurrence at or after the start of
/// `start_line` in `chars` (which is `lines.join("\n")` as a char vector).
fn find_name_offset(chars: &[char], start_line: u32, lines: &[&str], name: &str) -> Option<usize> {
    let start = line_start_offset(start_line, lines);
    let needle: Vec<char> = name.chars().collect();
    (start..=chars.len().saturating_sub(needle.len()))
        .find(|&i| chars[i..i + needle.len()] == needle[..])
}

/// Absolute char offset of the start of `line` in `lines.join("\n")`.
fn line_start_offset(line: u32, lines: &[&str]) -> usize {
    lines
        .iter()
        .take(line as usize)
        .map(|l| l.chars().count() + 1)
        .sum()
}

/// Absolute char offset of `(line, col)`.
fn offset_of(line: u32, col: u32, lines: &[&str]) -> usize {
    line_start_offset(line, lines) + col as usize
}

/// A positional property's `signature` is `"{type} {name}"`; strip the trailing
/// ` {name}` to recover the declared type. `object` when absent/malformed (an
/// untyped `out` param still resolves as a member).
fn property_type(prop: &ExtractedSymbol) -> String {
    let Some(sig) = prop.signature.as_deref() else {
        return "object".to_string();
    };
    let suffix = format!(" {}", prop.name);
    let ty = sig.strip_suffix(&suffix).unwrap_or(sig).trim();
    if ty.is_empty() {
        "object".to_string()
    } else {
        ty.to_string()
    }
}

/// Build a synthesized member named `name` under `scope_qname`.
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

// ---------------------------------------------------------------------------
// Test exposure
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(super) fn _test_synthesize(source: &str) -> Synthesized {
    let r = super::extract::extract(source);
    synthesize_symbols(source, &r.symbols, &r.refs)
}
