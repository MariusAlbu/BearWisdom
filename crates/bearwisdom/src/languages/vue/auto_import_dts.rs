// =============================================================================
// languages/vue/auto_import_dts.rs — parser for unplugin-generated `.d.ts`
//
// `unplugin-vue-components` and `unplugin-auto-import` write a declaration file
// the project ships in source control. Each entry pins a template tag (or a
// composable identifier) to the module its symbol comes from:
//
//   // components.d.ts (unplugin-vue-components)
//   declare module 'vue' {
//     export interface GlobalComponents {
//       FooBar: typeof import('./components/foo/Bar.vue')['default']
//       ExtWidget: typeof import('@scope/ui')['ExtWidget']
//     }
//   }
//
//   // auto-imports.d.ts (unplugin-auto-import)
//   declare global {
//     const useThing: typeof import('@scope/composables')['useThing']
//     const ref: typeof import('vue')['ref']
//   }
//
// Both forms share one line shape:  `<Name>: typeof import('<module>')[...]`
// (component map) or `const <Name>: typeof import('<module>')[...]` (auto-
// import map). This parser extracts `(Name, module)` pairs from that shape. The
// module is either a project-relative path (resolves to an in-index `.vue` /
// `.ts` file) or a bare package specifier (resolves to the external index).
//
// Pure data: every pair comes from a file the project generated and committed.
// =============================================================================

/// One `(local name, source module)` binding extracted from a generated
/// declaration file. `module` is the verbatim specifier from the
/// `import('...')` clause — relative path or bare package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoImportEntry {
    pub name: String,
    pub module: String,
}

/// Parse a generated unplugin declaration file (`components.d.ts` or
/// `auto-imports.d.ts`) into its `(name, module)` bindings.
///
/// Recognizes both the `Name: typeof import('mod')[...]` interface-member form
/// and the `const Name: typeof import('mod')[...]` global-const form. Lines that
/// don't match the shape (comments, `export {}`, the `declare module` wrapper)
/// are skipped.
pub fn parse_dts(source: &str) -> Vec<AutoImportEntry> {
    let mut entries: Vec<AutoImportEntry> = Vec::new();
    for line in source.lines() {
        if let Some(entry) = parse_entry_line(line) {
            entries.push(entry);
        }
    }
    entries
}

/// Parse a single declaration line. Returns `None` for any line that isn't a
/// `Name: typeof import('module')[...]` binding.
fn parse_entry_line(line: &str) -> Option<AutoImportEntry> {
    let trimmed = line.trim();

    // The binding must reference a `typeof import(...)` source. This single
    // gate rejects the wrapper lines (`declare module`, `export interface`,
    // `export {}`) and comments, which never contain `typeof import(`.
    let typeof_idx = trimmed.find("typeof import(")?;

    // Left of `:` is the bound name, after stripping a leading `const`/`export`.
    let colon_idx = trimmed[..typeof_idx].find(':')?;
    let name_part = trimmed[..colon_idx]
        .trim()
        .trim_start_matches("export ")
        .trim()
        .trim_start_matches("const ")
        .trim()
        .trim_start_matches("readonly ")
        .trim();
    if !is_binding_name(name_part) {
        return None;
    }

    let module = extract_import_specifier(&trimmed[typeof_idx..])?;
    if module.is_empty() {
        return None;
    }

    Some(AutoImportEntry {
        name: name_part.to_string(),
        module,
    })
}

/// Extract the quoted specifier from a `typeof import('<module>')` fragment.
/// Accepts single, double, or backtick quotes.
fn extract_import_specifier(fragment: &str) -> Option<String> {
    let open = fragment.find('(')?;
    let after = fragment[open + 1..].trim_start();
    let quote = after.chars().next()?;
    if quote != '\'' && quote != '"' && quote != '`' {
        return None;
    }
    let rest = &after[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

/// A bound name is a single JS identifier (component tags and composables are
/// always single identifiers in these generated files — namespaced re-exports
/// land as their own `Name` keys).
fn is_binding_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        && s.chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
}

#[cfg(test)]
#[path = "auto_import_dts_tests.rs"]
mod tests;
