use super::*;

#[test]
fn parses_vite_config_with_path_resolve() {
    let src = r#"
        import { defineConfig } from 'vite';
        import path from 'path';
        export default defineConfig({
            resolve: {
                alias: {
                    next: path.resolve('./app/javascript/dashboard/components-next'),
                    dashboard: path.resolve('./app/javascript/dashboard'),
                },
            },
        });
    "#;
    let aliases = parse_js_config_aliases(src);
    assert!(
        aliases
            .iter()
            .any(|(k, v)| k == "next/" && v == "app/javascript/dashboard/components-next/"),
        "expected next → app/javascript/dashboard/components-next, got {aliases:?}"
    );
    assert!(
        aliases
            .iter()
            .any(|(k, v)| k == "dashboard/" && v == "app/javascript/dashboard/"),
        "expected dashboard alias, got {aliases:?}"
    );
}

#[test]
fn parses_bare_string_alias() {
    // Vue$ with exact-match suffix should be skipped; bare string aliases
    // pass through. Mirrors chatwoot's `vue: 'vue/dist/vue.esm-bundler.js'`.
    let src = r#"
        export default {
            resolve: {
                alias: {
                    'vue$': 'vue/dist/vue.esm-bundler.js',
                    '@': 'src',
                },
            },
        };
    "#;
    let aliases = parse_js_config_aliases(src);
    assert!(
        aliases.iter().any(|(k, _)| k == "@/"),
        "bare `@` alias should be captured: {aliases:?}"
    );
    assert!(
        !aliases.iter().any(|(k, _)| k.starts_with("vue$")),
        "exact-match vue$ alias must be skipped: {aliases:?}"
    );
}

#[test]
fn parses_webpack_nested_under_configure_webpack() {
    // Vue CLI pattern: `configureWebpack.resolve.alias`.
    let src = r#"
        module.exports = {
            configureWebpack: {
                resolve: {
                    alias: {
                        '@': path.resolve(__dirname, 'src'),
                        '@components': path.resolve(__dirname, 'src/components'),
                    },
                },
            },
        };
    "#;
    let aliases = parse_js_config_aliases(src);
    assert!(
        aliases.iter().any(|(k, v)| k == "@/" && v == "src/"),
        "nested @ alias must resolve: {aliases:?}"
    );
    assert!(
        aliases
            .iter()
            .any(|(k, v)| k == "@components/" && v == "src/components/"),
        "nested @components alias: {aliases:?}"
    );
}

#[test]
fn parses_fileurl_to_path_new_url() {
    let src = r#"
        export default defineConfig({
            resolve: {
                alias: {
                    '@': fileURLToPath(new URL('./src', import.meta.url)),
                },
            },
        });
    "#;
    let aliases = parse_js_config_aliases(src);
    assert!(
        aliases.iter().any(|(k, v)| k == "@/" && v == "src/"),
        "fileURLToPath(new URL(...)) should unwrap: {aliases:?}"
    );
}

#[test]
fn parses_sveltekit_kit_alias() {
    // SvelteKit declares aliases under `kit.alias` with bare-string targets.
    // The walk finds the nested `alias` object; the `$`-prefixed keys are
    // ordinary identifiers. The non-wildcard `$lib` entry is the one the
    // resolver's longest-prefix match uses for `$lib/utils/x` imports.
    let src = r#"
        const config = {
            kit: {
                paths: { relative: false },
                alias: {
                    $lib: 'src/lib',
                    '$lib/*': 'src/lib/*',
                    $i18n: '../i18n',
                },
            },
        };
        export default config;
    "#;
    let aliases = parse_js_config_aliases(src);
    assert!(
        aliases.iter().any(|(k, v)| k == "$lib/" && v == "src/lib/"),
        "kit.alias `$lib` must map to src/lib: {aliases:?}"
    );
    assert!(
        aliases
            .iter()
            .any(|(k, v)| k == "$i18n/" && v == "../i18n/"),
        "kit.alias `$i18n` must map to ../i18n: {aliases:?}"
    );
}

#[test]
fn ignores_dynamic_values() {
    // Template interpolation and bare identifier references can't be
    // statically evaluated — entries should be dropped, not guessed.
    let src = r#"
        const base = './src';
        export default {
            resolve: {
                alias: {
                    '@': base,
                    '@str': `${base}/str`,
                    '@static': './literal',
                },
            },
        };
    "#;
    let aliases = parse_js_config_aliases(src);
    assert!(
        !aliases.iter().any(|(k, _)| k == "@/"),
        "identifier reference `base` must be dropped: {aliases:?}"
    );
    assert!(
        !aliases.iter().any(|(k, _)| k == "@str/"),
        "interpolated template string must be dropped: {aliases:?}"
    );
    assert!(
        aliases
            .iter()
            .any(|(k, v)| k == "@static/" && v == "literal/"),
        "plain string alias must still pass through: {aliases:?}"
    );
}

#[test]
fn empty_config_yields_empty_vec() {
    let src = "export default {};";
    assert!(parse_js_config_aliases(src).is_empty());
}

#[test]
fn no_alias_key_yields_empty_vec() {
    let src = r#"
        export default {
            resolve: {
                extensions: ['.js', '.ts'],
            },
        };
    "#;
    assert!(parse_js_config_aliases(src).is_empty());
}

#[test]
fn chatwoot_vite_config_real_shape() {
    // Single-arg `path.resolve()` calls plus a bare-string alias.
    let src = r#"
        export default defineConfig({
            plugins: plugins,
            resolve: {
                alias: {
                    vue: 'vue/dist/vue.esm-bundler.js',
                    components: path.resolve('./app/javascript/dashboard/components'),
                    next: path.resolve('./app/javascript/dashboard/components-next'),
                    v3: path.resolve('./app/javascript/v3'),
                    dashboard: path.resolve('./app/javascript/dashboard'),
                },
            },
        });
    "#;
    let aliases = parse_js_config_aliases(src);
    let find = |prefix: &str| {
        aliases
            .iter()
            .find(|(k, _)| k == prefix)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(
        find("next/"),
        Some("app/javascript/dashboard/components-next/".to_string())
    );
    assert_eq!(find("v3/"), Some("app/javascript/v3/".to_string()));
    assert_eq!(
        find("dashboard/"),
        Some("app/javascript/dashboard/".to_string())
    );
    assert_eq!(
        find("components/"),
        Some("app/javascript/dashboard/components/".to_string())
    );
    assert_eq!(
        find("vue/"),
        Some("vue/dist/vue.esm-bundler.js/".to_string())
    );
}
