// =============================================================================
// nuget/cs_header.rs — line-based scanner for top-level C# declarations.
//
// Tracks brace depth to determine scope (namespace, type nesting). Emits
// public/internal type declarations and public methods; skips private
// members, compiler-generated names, and method body interiors. Used by
// source-side NuGet helpers — DLL metadata uses a separate `dotscope` path.
// =============================================================================

#[derive(Debug)]
pub(crate) struct CsDecl {
    pub(super) name: String,
    /// Dot-joined namespace + enclosing type path (empty at global scope).
    pub(super) scope: String,
    pub(super) kind: crate::types::SymbolKind,
    pub(super) signature: Option<String>,
    /// 1-based source line number.
    pub(super) line: usize,
}

/// Scan a C# source file and extract top-level public declarations.
/// Returns one `CsDecl` per class/interface/enum/struct/record/delegate
/// and public method found at namespace→type→member depth.
pub(crate) fn scan_cs_header(source: &str) -> Vec<CsDecl> {
    use crate::types::SymbolKind;

    let mut out = Vec::new();
    // Stack entries: (name, kind_char) where 'n'=namespace, 't'=type.
    let mut scope_stack: Vec<(String, char)> = Vec::new();

    for (line_idx, raw_line) in source.lines().enumerate() {
        let line = raw_line.trim();

        // Count brace deltas on this line.
        let opens = line.chars().filter(|&c| c == '{').count() as i32;
        let closes = line.chars().filter(|&c| c == '}').count() as i32;

        // Pop scope for net closing braces (handles standalone `}` lines).
        if closes > opens {
            let net_close = (closes - opens) as usize;
            for _ in 0..net_close.min(scope_stack.len()) {
                scope_stack.pop();
            }
        }

        // Skip non-declaration lines early.
        if line.is_empty()
            || line.starts_with("//")
            || line.starts_with("/*")
            || line.starts_with('*')
            || line.starts_with('[')
            || line.starts_with('#')
        {
            continue;
        }

        // Namespace declaration — capture the full dotted name
        // (`namespace Acme.Orders` → "Acme.Orders").
        if let Some(rest) = strip_cs_keyword(line, "namespace") {
            let ns_name = cs_namespace_name(rest);
            if !ns_name.is_empty() {
                scope_stack.push((ns_name, 'n'));
            }
            continue;
        }

        // Skip non-public members (private/protected/internal-only at type level).
        let is_public = line.contains("public ")
            || (!line.contains("private ")
                && !line.contains("protected ")
                && !line.contains("internal "));
        if !is_public {
            continue;
        }

        // Type declarations.
        let type_kw: Option<(&str, SymbolKind)> = [
            ("interface ", SymbolKind::Interface),
            ("class ", SymbolKind::Class),
            ("struct ", SymbolKind::Struct),
            ("enum ", SymbolKind::Enum),
            ("record ", SymbolKind::Class),
            ("delegate ", SymbolKind::Function),
        ]
        .iter()
        .find_map(|(kw, kind)| {
            if line.contains(kw) {
                Some((*kw, *kind))
            } else {
                None
            }
        });

        if let Some((kw, kind)) = type_kw {
            if let Some(pos) = line.find(kw) {
                let after_kw = &line[pos + kw.len()..];
                let type_name = cs_first_ident(after_kw);
                if !type_name.is_empty() && !type_name.starts_with('<') {
                    let scope = cs_scope_string(&scope_stack, 'n');
                    let sig = Some(format!("{}{}", kw.trim_end(), format!(" {type_name}")));
                    out.push(CsDecl {
                        name: type_name.clone(),
                        scope,
                        kind,
                        signature: sig,
                        line: line_idx + 1,
                    });
                    scope_stack.push((type_name, 't'));
                }
            }
            continue;
        }

        // Method declarations — only emit if directly inside a type scope.
        let inside_type = scope_stack.last().map(|(_, k)| *k == 't').unwrap_or(false);
        if !inside_type {
            continue;
        }
        if line.contains("operator ") {
            continue;
        }

        if line.contains('(') {
            let method_name = extract_cs_method_name(line);
            if !method_name.is_empty() && !is_cs_noise_ident(&method_name) {
                let scope = cs_scope_string(&scope_stack, 't');
                let sig = Some(truncate_to_paren(line, 120));
                out.push(CsDecl {
                    name: method_name,
                    scope,
                    kind: SymbolKind::Method,
                    signature: sig,
                    line: line_idx + 1,
                });
            }
        }
    }

    out
}

/// Dot-join scope names of the requested kind and above.
/// `min_kind='n'` collects only namespace segments.
/// `min_kind='t'` collects namespace + type segments.
fn cs_scope_string(stack: &[(String, char)], min_kind: char) -> String {
    stack
        .iter()
        .filter(|(_, k)| *k == 'n' || (min_kind == 't' && *k == 't'))
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

/// Return the suffix after `kw ` in `line` if the keyword is present.
fn strip_cs_keyword<'a>(line: &'a str, kw: &str) -> Option<&'a str> {
    let pattern = format!("{kw} ");
    line.find(&pattern).map(|pos| &line[pos + pattern.len()..])
}

/// Grab the first C# identifier from `s` (alphanumeric + underscore).
fn cs_first_ident(s: &str) -> String {
    s.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

/// Grab a dotted namespace name from `s` (alphanumeric + `_` + `.`).
/// Stops at whitespace, `{`, or `;`. Used for `namespace Acme.Orders`.
fn cs_namespace_name(s: &str) -> String {
    s.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
        .collect::<String>()
        .trim_end_matches('.')
        .to_string()
}

/// Extract the method name from a line like
/// `public async Task<T> MyMethod(...)` — last identifier before `(`.
fn extract_cs_method_name(line: &str) -> String {
    let paren = match line.find('(') {
        Some(p) => p,
        None => return String::new(),
    };
    let before_paren = line[..paren].trim_end();
    // Strip trailing generic suffix `<T>` before the paren.
    let before_paren = if before_paren.ends_with('>') {
        match before_paren.rfind('<') {
            Some(lt) => before_paren[..lt].trim_end(),
            None => before_paren,
        }
    } else {
        before_paren
    };
    // Walk backwards to extract trailing identifier.
    before_paren
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

/// Truncate a line to `max_len` chars but keep up to the first `)`.
fn truncate_to_paren(line: &str, max_len: usize) -> String {
    let end = line
        .find(')')
        .map(|p| (p + 1).min(line.len()))
        .unwrap_or(line.len());
    let s = &line[..end.min(line.len())];
    if s.len() > max_len {
        s[..max_len].to_string()
    } else {
        s.to_string()
    }
}

/// True for C# keywords and noise identifiers that can never be method names.
fn is_cs_noise_ident(name: &str) -> bool {
    matches!(
        name,
        "if" | "else"
            | "for"
            | "foreach"
            | "while"
            | "do"
            | "switch"
            | "catch"
            | "finally"
            | "using"
            | "return"
            | "new"
            | "throw"
            | "var"
            | "get"
            | "set"
            | "init"
            | "add"
            | "remove"
    ) || name.starts_with('<')
}
