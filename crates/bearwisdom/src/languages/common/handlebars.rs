// =============================================================================
// languages/common/handlebars.rs — Handlebars helper export detection
//
// Two patterns:
//   - Ember/Ghost: ESM `export default helper(...)` re-emits the file basename as
//     a Handlebars helper.
//   - Runtime: `Handlebars.RegisterHelper("name", ...)` calls register helpers.
// =============================================================================

use crate::types::{ExtractedSymbol, ExtractionResult, SymbolKind};

// ---------------------------------------------------------------------------
// Handlebars helper-export detection (Ember + Ghost-style themes)
//
// Helpers and modifiers invoked from Handlebars templates are stored on
// disk as JavaScript / TypeScript modules whose default export wraps a
// helper-callable. The invocation name is the file stem (kebab → snake by
// the Handlebars→JS wrapper), but the inner function name doesn't match
// it. Without injecting a synthetic file-stem symbol, every template call
// like `{{gh-pluralize ...}}` lands in unresolved_refs.
//
// Three patterns are detected, all gated on per-file content signals to
// avoid false-positive matches in unrelated `helpers/` directories:
//
//   1. Ember helper:    `**/app/helpers/<name>.{js,ts,gjs,gts}`
//                       with `@ember/component/helper` or `template-only`.
//   2. Ember modifier:  `**/app/modifiers/<name>.{js,ts,gjs,gts}`
//                       with `@ember/component/modifier` or `@ember/render-modifiers`.
//   3. Ghost theme:     `**/<...>/helpers/<name>.{js,ts}` (any depth)
//                       with `require('...handlebars...')` or `services/handlebars`.
//
// All three append a Function symbol with `qualified_name = "__npm_globals__.<name>"`
// so the TS resolver's bare-name fallback finds it.
// ---------------------------------------------------------------------------

pub fn append_ember_helper_default_export(
    file_path: &str,
    source: &str,
    result: &mut crate::types::ExtractionResult,
) {
    let Some((stem, signature_hint)) = handlebars_helper_stem(file_path, source) else {
        return;
    };
    let invocation_name = stem.replace('-', "_");
    let qname = format!("__npm_globals__.{invocation_name}");
    if result.symbols.iter().any(|s| s.qualified_name == qname) {
        return;
    }
    result.symbols.push(crate::types::ExtractedSymbol {
        name: invocation_name.clone(),
        qualified_name: qname,
        kind: crate::types::SymbolKind::Function,
        visibility: Some(crate::types::Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("/* {signature_hint} export of {stem} */")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
    });
}

/// Detect a Handlebars-callable export and return (stem, signature_hint)
/// suitable for emitting the synthetic symbol. Returns None if the file
/// doesn't match any known convention.
fn handlebars_helper_stem(file_path: &str, source: &str) -> Option<(String, &'static str)> {
    let norm = file_path.replace('\\', "/");

    // 1. Ember helper: `app/helpers/<name>.{js,ts,gjs,gts}`
    if let Some(stem) = path_stem_after_segment(&norm, "/app/helpers/", &EMBER_EXTENSIONS) {
        if source.contains("@ember/component/helper")
            || source.contains("@ember/component/template-only")
        {
            return Some((stem, "Ember helper"));
        }
    }

    // 2. Ember modifier: `app/modifiers/<name>.{js,ts,gjs,gts}`
    if let Some(stem) = path_stem_after_segment(&norm, "/app/modifiers/", &EMBER_EXTENSIONS) {
        if source.contains("@ember/component/modifier")
            || source.contains("@ember/render-modifiers")
            || source.contains("ember-modifier")
        {
            return Some((stem, "Ember modifier"));
        }
    }

    // 3. Ghost-style theme helper: any `**/helpers/<name>.{js,ts}` with a
    //    handlebars-services import. Excludes the `app/helpers/` case
    //    already handled above (different content signal).
    if !norm.contains("/app/helpers/") {
        if let Some(stem) = path_stem_after_segment(&norm, "/helpers/", &THEME_EXTENSIONS) {
            if is_ghost_style_theme_helper(source) {
                return Some((stem, "Handlebars theme helper"));
            }
        }
    }

    None
}

const EMBER_EXTENSIONS: [&str; 4] = [".js", ".ts", ".gjs", ".gts"];
const THEME_EXTENSIONS: [&str; 2] = [".js", ".ts"];

/// Find the file stem that sits directly inside `segment` (no nested subdir).
/// Nested helper paths (`segment/sub/name.js`) return None — they require
/// dotted-name resolution which the bare-name fallback doesn't cover.
fn path_stem_after_segment(norm: &str, segment: &str, exts: &[&str]) -> Option<String> {
    let idx = norm.rfind(segment)?;
    let after = &norm[idx + segment.len()..];
    let after = after.trim_start_matches('/');
    if after.contains('/') {
        return None;
    }
    for ext in exts {
        if let Some(stem) = after.strip_suffix(ext) {
            if !stem.is_empty() {
                return Some(stem.to_string());
            }
        }
    }
    None
}

fn is_ghost_style_theme_helper(source: &str) -> bool {
    // Ghost theme helpers `require('../services/handlebars')` for SafeString
    // and friends; other Handlebars-host frameworks (Express-Handlebars,
    // hbs-engine) use `Handlebars.registerHelper`. Either signal qualifies.
    source.contains("services/handlebars")
        || source.contains("Handlebars.registerHelper")
        || source.contains("handlebars').SafeString")
        || source.contains("handlebars\").SafeString")
}


// ---------------------------------------------------------------------------
// Handlebars.RegisterHelper("name", ...) — runtime registration scan
//
// Some hosts (Bitwarden's C# Handlebars.Net, JS server-side templating)
// register helpers imperatively rather than by file convention. The helper
// name is a string literal in the registration call, and the consuming
// templates invoke it bare. Without scanning these registrations the
// invocations land in unresolved_refs.
//
// The scan is regex-free deliberately — a small state machine that
// recognizes the literal `Handlebars.RegisterHelper(` or
// `Handlebars.registerHelper(` token, then captures the next quoted string
// argument. Works for C#, JS, TS, and any language that calls the same
// API. Each captured name is appended as a Function symbol with
// `qualified_name = "__npm_globals__.<name>"` so the TS resolver's
// bare-name fallback finds it from a Handlebars-embedded template call.
// ---------------------------------------------------------------------------

pub fn append_handlebars_register_helper_globals(
    source: &str,
    result: &mut crate::types::ExtractionResult,
) {
    for name in scan_register_helper_names(source) {
        let invocation_name = name.replace('-', "_");
        let qname = format!("__npm_globals__.{invocation_name}");
        if result.symbols.iter().any(|s| s.qualified_name == qname) {
            continue;
        }
        result.symbols.push(crate::types::ExtractedSymbol {
            name: invocation_name.clone(),
            qualified_name: qname,
            kind: crate::types::SymbolKind::Function,
            visibility: Some(crate::types::Visibility::Public),
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            signature: Some(format!("/* Handlebars.RegisterHelper(\"{name}\", ...) */")),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
            byte_offset: 0,
        });
    }
}

fn scan_register_helper_names(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let needles = [
        "Handlebars.RegisterHelper(",
        "Handlebars.registerHelper(",
        "handlebars.RegisterHelper(",
        "handlebars.registerHelper(",
    ];
    let bytes = source.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let mut matched_len: Option<usize> = None;
        for needle in needles {
            let nb = needle.as_bytes();
            if i + nb.len() <= bytes.len() && &bytes[i..i + nb.len()] == nb {
                matched_len = Some(nb.len());
                break;
            }
        }
        if let Some(after_open) = matched_len {
            let mut j = i + after_open;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n' || bytes[j] == b'\r') {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
                let quote = bytes[j];
                let start = j + 1;
                let mut k = start;
                while k < bytes.len() && bytes[k] != quote {
                    if bytes[k] == b'\\' && k + 1 < bytes.len() {
                        k += 2;
                    } else {
                        k += 1;
                    }
                }
                if k <= bytes.len() && k > start {
                    if let Ok(name) = std::str::from_utf8(&bytes[start..k]) {
                        let trimmed = name.trim();
                        if !trimmed.is_empty()
                            && trimmed.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                        {
                            out.push(trimmed.to_string());
                        }
                    }
                }
            }
            i += after_open;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::amd::{append_amd_define_imports, scan_amd_define_pairs};
    use super::super::html::{
        extract_astro_frontmatter, extract_html_script_style_regions, extract_script_refs,
    };
    use super::super::jquery::append_jquery_fn_plugin_globals;
    use crate::types::EmbeddedOrigin;

    #[test]
    fn extracts_vue_sfc_script_and_style_blocks() {
        let src = "<template>\n  <div>Hello</div>\n</template>\n\n<script setup lang=\"ts\">\nimport { ref } from 'vue'\nconst count = ref(0)\n</script>\n\n<style lang=\"scss\" scoped>\n.foo { color: red; }\n</style>\n";
        let regions = extract_html_script_style_regions(src);
        assert_eq!(regions.len(), 2, "expected one script + one style region");

        let script = &regions[0];
        assert_eq!(script.language_id, "typescript");
        assert_eq!(script.origin, EmbeddedOrigin::ScriptBlock);
        assert!(script.text.contains("import { ref } from 'vue'"));
        assert!(script.text.contains("const count = ref(0)"));
        // tree-sitter-html's `raw_text` node begins immediately after the
        // start tag's `>`, so it starts at the trailing newline on the same
        // line as `<script setup lang="ts">` (line 4). The sub-extracted
        // `import` line is at region-line 1, which the dispatcher rewrites
        // to file-line 5.
        assert_eq!(script.line_offset, 4);

        let style = &regions[1];
        assert_eq!(style.language_id, "scss");
        assert_eq!(style.origin, EmbeddedOrigin::StyleBlock);
        assert!(style.text.contains(".foo { color: red; }"));
    }

    #[test]
    fn plain_script_defaults_to_javascript() {
        let src = "<template><p/></template>\n<script>\nconsole.log('hi')\n</script>\n";
        let regions = extract_html_script_style_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].language_id, "javascript");
    }

    #[test]
    fn json_ld_script_is_skipped() {
        // application/ld+json is not executable JavaScript; sub-dispatch would
        // treat it as JS and emit garbage. The helper must drop it.
        let src = "<script type=\"application/ld+json\">{\"@context\":\"https://schema.org\"}</script>\n";
        let regions = extract_html_script_style_regions(src);
        assert!(regions.is_empty(), "ld+json must be skipped, not sub-parsed");
    }

    #[test]
    fn unsupported_style_lang_is_skipped() {
        // less / stylus aren't wired to any plugin yet — skip rather than
        // hand the text to the CSS extractor and produce wrong results.
        let src = "<style lang=\"less\">.foo { color: red; }</style>\n";
        let regions = extract_html_script_style_regions(src);
        assert!(regions.is_empty());
    }

    #[test]
    fn line_offset_matches_raw_text_start_for_multiline_file() {
        // Line offsets point at tree-sitter's `raw_text` node, which starts
        // immediately after the opening tag's `>` — on the same line as
        // `<script>`. The sub-extractor sees a region whose first character
        // is `\n`, so `let x = 1` sits on region-line 1; adding the
        // line_offset of 3 resolves it back to file-line 4.
        let src = "<template>\n  <div/>\n</template>\n<script lang=\"ts\">\nlet x = 1\n</script>\n";
        let regions = extract_html_script_style_regions(src);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].line_offset, 3);
    }

    #[test]
    fn extracts_astro_frontmatter_block() {
        let src = "---\nimport Layout from '../layouts/Layout.astro';\nconst title = 'Home';\n---\n<Layout title={title}>\n  <h1>Hello</h1>\n</Layout>\n";
        let fm = extract_astro_frontmatter(src).expect("frontmatter");
        assert_eq!(fm.language_id, "typescript");
        assert_eq!(fm.origin, EmbeddedOrigin::Frontmatter);
        assert!(fm.text.contains("import Layout"));
        assert!(fm.text.contains("const title = 'Home';"));
        // Opening fence `---\n` is line 0; body starts on line 1.
        assert_eq!(fm.line_offset, 1);
    }

    #[test]
    fn missing_astro_frontmatter_returns_none() {
        let src = "<h1>No frontmatter here</h1>\n";
        assert!(extract_astro_frontmatter(src).is_none());
    }

    #[test]
    fn astro_frontmatter_respects_leading_whitespace() {
        // Astro allows (ignores) leading newlines before the opening fence.
        let src = "\n\n---\nconst x = 1;\n---\n<p/>\n";
        let fm = extract_astro_frontmatter(src).expect("frontmatter");
        assert!(fm.text.contains("const x = 1;"));
    }

    #[test]
    fn script_ref_double_quoted_url() {
        let src = r#"<html><head>
<script src="~/lib/jquery/jquery.js"></script>
</head></html>"#;
        let refs = extract_script_refs(src);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "~/lib/jquery/jquery.js");
        assert_eq!(refs[0].line, 1);
    }

    #[test]
    fn script_ref_single_quoted_url() {
        let src = "<script src='/js/app.js'></script>\n";
        let refs = extract_script_refs(src);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "/js/app.js");
    }

    #[test]
    fn script_ref_unquoted_url() {
        let src = "<script src=lib/foo.js></script>\n";
        let refs = extract_script_refs(src);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "lib/foo.js");
    }

    #[test]
    fn script_ref_with_extra_attrs() {
        // Typical ASP.NET MVC pattern with tag helper before src.
        let src = r#"<script simpl-append-version="true" src="~/lib/bootstrap/dist/js/bootstrap.js"></script>"#;
        let refs = extract_script_refs(src);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "~/lib/bootstrap/dist/js/bootstrap.js");
    }

    #[test]
    fn script_ref_cdn_skipped() {
        let src = r#"
<script src="https://cdn.jsdelivr.net/npm/vue"></script>
<script src="//cdn.example.com/jquery.js"></script>
<script src="http://localhost/foo.js"></script>
<script src="data:application/javascript,console.log(1)"></script>
"#;
        assert!(extract_script_refs(src).is_empty());
    }

    #[test]
    fn inline_script_without_src_ignored() {
        let src = "<script>console.log('hi');</script>";
        assert!(extract_script_refs(src).is_empty());
    }

    #[test]
    fn multiple_script_refs_collected() {
        let src = r#"
<script src="~/lib/jquery/jquery.js"></script>
<script src="~/lib/angular/angular.js"></script>
<script src="/custom/app.js"></script>
"#;
        let refs = extract_script_refs(src);
        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].url, "~/lib/jquery/jquery.js");
        assert_eq!(refs[1].url, "~/lib/angular/angular.js");
        assert_eq!(refs[2].url, "/custom/app.js");
    }

    #[test]
    fn script_ref_skips_similar_tag_names() {
        // `<scripts>` and `<scriptoid>` must not trigger a match — only
        // `<script` followed by whitespace, `>`, or `/`.
        let src = r#"
<scripts src="foo.js"></scripts>
<scriptoid src="bar.js"></scriptoid>
"#;
        assert!(extract_script_refs(src).is_empty());
    }

    #[test]
    fn script_ref_case_insensitive_tag() {
        let src = r#"<SCRIPT SRC="lib/foo.js"></SCRIPT>"#;
        let refs = extract_script_refs(src);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "lib/foo.js");
    }

    #[test]
    fn script_ref_survives_razor_at_syntax() {
        // Razor files mix `@...` directives with HTML; our byte scan must
        // not choke on them.
        let src = r#"@{
    Layout = "_Layout";
}
<script src="~/lib/jquery/jquery.js"></script>
@section Scripts {
    <script src="~/js/page.js"></script>
}"#;
        let refs = extract_script_refs(src);
        assert_eq!(refs.len(), 2);
    }

    // -----------------------------------------------------------------------
    // Ember helper-export detection
    // -----------------------------------------------------------------------

    fn empty_result() -> crate::types::ExtractionResult {
        crate::types::ExtractionResult {
            symbols: Vec::new(),
            refs: Vec::new(),
            routes: Vec::new(),
            db_sets: Vec::new(),
            has_errors: false,
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
        }
    }

    #[test]
    fn ember_helper_appends_npm_globals_symbol() {
        let mut r = empty_result();
        let src = "import {helper} from '@ember/component/helper';\nexport default helper(() => 'x');";
        append_ember_helper_default_export(
            "ghost/admin/app/helpers/gh-pluralize.js",
            src,
            &mut r,
        );
        let sym = r
            .symbols
            .iter()
            .find(|s| s.qualified_name == "__npm_globals__.gh_pluralize")
            .expect("expected synthetic helper symbol");
        assert_eq!(sym.name, "gh_pluralize");
        assert_eq!(sym.kind, crate::types::SymbolKind::Function);
    }

    #[test]
    fn ember_helper_skipped_when_no_ember_import() {
        let mut r = empty_result();
        let src = "// just a regular module\nexport function thing() { return 1; }";
        append_ember_helper_default_export(
            "myproject/app/helpers/random.js",
            src,
            &mut r,
        );
        assert!(
            r.symbols.is_empty(),
            "non-Ember files in helpers/ should not get the synthetic; got: {:?}",
            r.symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn ember_helper_skipped_outside_app_helpers_dir() {
        let mut r = empty_result();
        let src = "import {helper} from '@ember/component/helper';\nexport default helper(() => 'x');";
        append_ember_helper_default_export(
            "ghost/admin/app/lib/random.js",
            src,
            &mut r,
        );
        assert!(r.symbols.is_empty());
    }

    #[test]
    fn ember_helper_skipped_for_nested_helper_paths() {
        // `app/helpers/blog/post-card.js` — invocation would be `blog/post-card`
        // (rewritten to `blog.post_card` by the Handlebars wrapper). That's a
        // dotted lookup, not a bare-name fallback target — out of scope here.
        let mut r = empty_result();
        let src = "import {helper} from '@ember/component/helper';\nexport default helper(() => 'x');";
        append_ember_helper_default_export(
            "ghost/admin/app/helpers/blog/post-card.js",
            src,
            &mut r,
        );
        assert!(r.symbols.is_empty());
    }

    #[test]
    fn ember_helper_handles_typescript_extension() {
        let mut r = empty_result();
        let src = "import {helper} from '@ember/component/helper';\nexport default helper(() => 'x');";
        append_ember_helper_default_export(
            "myapp/app/helpers/format-date.ts",
            src,
            &mut r,
        );
        assert!(r.symbols.iter().any(|s| s.name == "format_date"));
    }

    #[test]
    fn ember_helper_idempotent_on_repeat_calls() {
        let mut r = empty_result();
        let src = "import {helper} from '@ember/component/helper';\nexport default helper(() => 'x');";
        append_ember_helper_default_export(
            "myapp/app/helpers/eq.js",
            src,
            &mut r,
        );
        append_ember_helper_default_export(
            "myapp/app/helpers/eq.js",
            src,
            &mut r,
        );
        assert_eq!(
            r.symbols.iter().filter(|s| s.qualified_name == "__npm_globals__.eq").count(),
            1,
            "duplicate detection should keep the symbol unique"
        );
    }

    #[test]
    fn ember_modifier_appends_npm_globals_symbol() {
        let mut r = empty_result();
        let src = "import Modifier from '@ember/component/modifier';\nexport default class extends Modifier { modify() {} }";
        append_ember_helper_default_export(
            "ghost/admin/app/modifiers/react-render.js",
            src,
            &mut r,
        );
        let sym = r
            .symbols
            .iter()
            .find(|s| s.qualified_name == "__npm_globals__.react_render")
            .expect("expected synthetic modifier symbol");
        assert_eq!(sym.name, "react_render");
        assert!(sym.signature.as_ref().unwrap().contains("Ember modifier"));
    }

    #[test]
    fn ember_modifier_via_render_modifiers_import() {
        let mut r = empty_result();
        let src = "import { modifier } from 'ember-modifier';\nexport default modifier((el) => {});";
        append_ember_helper_default_export(
            "myapp/app/modifiers/on-key.js",
            src,
            &mut r,
        );
        assert!(r.symbols.iter().any(|s| s.name == "on_key"));
    }

    #[test]
    fn ghost_theme_helper_appends_npm_globals_symbol() {
        let mut r = empty_result();
        let src = "const {SafeString} = require('../services/handlebars');\nmodule.exports = function tiers(options) { return new SafeString(''); };";
        append_ember_helper_default_export(
            "ghost/core/core/frontend/helpers/tiers.js",
            src,
            &mut r,
        );
        let sym = r
            .symbols
            .iter()
            .find(|s| s.qualified_name == "__npm_globals__.tiers")
            .expect("expected synthetic theme-helper symbol");
        assert_eq!(sym.name, "tiers");
        assert!(sym.signature.as_ref().unwrap().contains("theme helper"));
    }

    #[test]
    fn ghost_theme_helper_via_register_helper_pattern() {
        let mut r = empty_result();
        let src = "const Handlebars = require('handlebars');\nHandlebars.registerHelper('formatDate', function(d) { return d; });\nmodule.exports = formatDate;";
        append_ember_helper_default_export(
            "myapp/lib/helpers/format-date.js",
            src,
            &mut r,
        );
        assert!(
            r.symbols.iter().any(|s| s.name == "format_date"),
            "Handlebars.registerHelper pattern should activate detection"
        );
    }

    #[test]
    fn random_helpers_dir_without_handlebars_signal_skipped() {
        let mut r = empty_result();
        // A folder named `helpers/` but with no Handlebars signal — could be
        // a generic JS utility module. Don't claim it as a template helper.
        let src = "export function helper() {}\nexport default helper;";
        append_ember_helper_default_export(
            "src/helpers/utility.js",
            src,
            &mut r,
        );
        assert!(r.symbols.is_empty(), "no Handlebars signal → no synthetic");
    }

    // -----------------------------------------------------------------------
    // Handlebars.RegisterHelper("name", ...) scan
    // -----------------------------------------------------------------------

    #[test]
    fn register_helper_csharp_double_quoted_captures_name() {
        let mut r = empty_result();
        let src = "Handlebars.RegisterHelper(\"usd\", (writer, ctx, args) => writer.Write(args[0]));";
        append_handlebars_register_helper_globals(src, &mut r);
        assert!(r.symbols.iter().any(|s|
            s.name == "usd" && s.qualified_name == "__npm_globals__.usd"
        ));
    }

    #[test]
    fn register_helper_js_lowercase_captures_name() {
        let mut r = empty_result();
        let src = "Handlebars.registerHelper('format-date', function(d) { return d; });";
        append_handlebars_register_helper_globals(src, &mut r);
        assert!(r.symbols.iter().any(|s|
            s.name == "format_date" && s.qualified_name == "__npm_globals__.format_date"
        ));
    }

    #[test]
    fn register_helper_multiple_in_one_file() {
        let mut r = empty_result();
        let src = r#"
            Handlebars.RegisterHelper("date", X);
            Handlebars.RegisterHelper("usd", Y);
            Handlebars.RegisterHelper("plurality", Z);
        "#;
        append_handlebars_register_helper_globals(src, &mut r);
        for n in ["date", "usd", "plurality"] {
            assert!(r.symbols.iter().any(|s| s.name == n), "missing {n}");
        }
    }

    #[test]
    fn register_helper_idempotent_on_duplicate_registration() {
        let mut r = empty_result();
        let src = "Handlebars.RegisterHelper(\"eq\", X);\nHandlebars.RegisterHelper(\"eq\", Y);";
        append_handlebars_register_helper_globals(src, &mut r);
        assert_eq!(
            r.symbols.iter().filter(|s| s.name == "eq").count(),
            1,
            "duplicate registrations of the same name should yield one symbol"
        );
    }

    #[test]
    fn register_helper_skips_non_string_first_arg() {
        let mut r = empty_result();
        // Variable as helper name — can't statically capture it.
        let src = "Handlebars.RegisterHelper(myHelperName, fn);";
        append_handlebars_register_helper_globals(src, &mut r);
        assert!(r.symbols.is_empty());
    }

    // -----------------------------------------------------------------------
    // AMD `define([deps], function(params) {...})` scan
    // -----------------------------------------------------------------------

    #[test]
    fn amd_define_emits_imports_per_dep() {
        let src = "define([ \"jquery\", \"./config\", \"preferences\" ],\n        function($, config, preferences) { return $.fn; });";
        let pairs = scan_amd_define_pairs(src);
        let by_dep: std::collections::HashMap<&str, &str> =
            pairs.iter().map(|(d, p, _)| (d.as_str(), p.as_str())).collect();
        assert_eq!(by_dep.get("jquery"), Some(&"$"));
        assert_eq!(by_dep.get("./config"), Some(&"config"));
        assert_eq!(by_dep.get("preferences"), Some(&"preferences"));
    }

    #[test]
    fn amd_define_handles_named_module_form() {
        // `define("modname", [...], function(...) {...})` — leading
        // string is the module name, ignored.
        let src = "define(\"my/mod\", [\"jquery\"], function($) {});";
        let pairs = scan_amd_define_pairs(src);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "jquery");
        assert_eq!(pairs[0].1, "$");
    }

    #[test]
    fn amd_define_arrow_callback_works() {
        let src = "define([\"jquery\"], ($) => $.fn);";
        let pairs = scan_amd_define_pairs(src);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].1, "$");
    }

    #[test]
    fn amd_define_multiline_dep_array() {
        let src = "define([\n  \"a\",\n  \"b\",\n  \"c\"\n], function(a, b, c) {});";
        let pairs = scan_amd_define_pairs(src);
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[0].0, "a");
        assert_eq!(pairs[2].1, "c");
    }

    #[test]
    fn append_amd_define_imports_skips_amd_bookkeeping() {
        let mut r = empty_result();
        // `require`, `exports`, `module` are AMD bookkeeping pseudo-deps.
        let src = "define([\"require\", \"exports\", \"./real\"], function(req, exp, real) {});";
        append_amd_define_imports(src, &mut r);
        let names: Vec<&str> = r
            .refs
            .iter()
            .filter(|x| matches!(x.kind, crate::types::EdgeKind::Imports))
            .map(|x| x.target_name.as_str())
            .collect();
        assert_eq!(names, vec!["real"]);
    }

    #[test]
    fn amd_define_xhelper_define_no_match() {
        // `xdefine([...])` must not match `define([...])`.
        let src = "xdefine([\"jquery\"], function($){});";
        let pairs = scan_amd_define_pairs(src);
        assert!(pairs.is_empty());
    }

    // -----------------------------------------------------------------------
    // `$.fn.NAME = function(...)` jQuery plugin registration scan
    // -----------------------------------------------------------------------

    #[test]
    fn jquery_fn_plugin_emits_npm_globals() {
        let mut r = empty_result();
        let src = "$.fn.prologEditor = function(method) { return this; };";
        append_jquery_fn_plugin_globals(src, &mut r);
        assert!(r.symbols.iter().any(|s|
            s.name == "prologEditor" && s.qualified_name == "__npm_globals__.prologEditor"
        ));
    }

    #[test]
    fn jquery_fn_plugin_handles_full_jquery_prefix() {
        let mut r = empty_result();
        let src = "jQuery.fn.tooltip = function(opts) {};";
        append_jquery_fn_plugin_globals(src, &mut r);
        assert!(r.symbols.iter().any(|s| s.name == "tooltip"));
    }

    #[test]
    fn jquery_fn_plugin_handles_bracketed_form() {
        let mut r = empty_result();
        let src = "$.fn['nbCell'] = function(method) {};";
        append_jquery_fn_plugin_globals(src, &mut r);
        assert!(r.symbols.iter().any(|s| s.name == "nbCell"));
    }

    #[test]
    fn jquery_fn_plugin_skips_non_function_rhs() {
        let mut r = empty_result();
        // Plain value assignment, not a callable plugin.
        let src = "$.fn.version = '1.0';";
        append_jquery_fn_plugin_globals(src, &mut r);
        assert!(r.symbols.is_empty());
    }

    #[test]
    fn jquery_fn_plugin_dedupes_same_name() {
        let mut r = empty_result();
        let src = "$.fn.foo = function() {};\n$.fn.foo = function() {};";
        append_jquery_fn_plugin_globals(src, &mut r);
        assert_eq!(r.symbols.iter().filter(|s| s.name == "foo").count(), 1);
    }
}
