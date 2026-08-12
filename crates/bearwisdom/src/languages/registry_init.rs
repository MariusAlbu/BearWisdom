// =============================================================================
// languages/registry_init.rs — default plugin registry construction
//
// Builds the process-wide `LanguageRegistry`: one `reg.register(...)` call per
// language plugin, plus the generic tree-sitter-only fallback plugin used for
// any language with a grammar but no dedicated extractor. Split out of
// `languages/mod.rs` so the `LanguagePlugin` trait definition isn't buried
// under a hundred mechanical registration lines.
// =============================================================================

use std::sync::{Arc, LazyLock};

use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

use super::*;

static DEFAULT_REGISTRY: LazyLock<LanguageRegistry> = LazyLock::new(|| {
    // The generic plugin handles any language with a tree-sitter grammar
    // but no dedicated extractor.
    let generic = Arc::new(GenericPlugin);
    let mut reg = LanguageRegistry::new(generic);

    reg.register(Arc::new(angular::AngularPlugin));
    reg.register(Arc::new(angular_template::AngularTemplatePlugin));
    reg.register(Arc::new(astro::AstroPlugin));
    reg.register(Arc::new(bash::BashPlugin));
    reg.register(Arc::new(bicep::BicepPlugin));
    reg.register(Arc::new(blade::BladePlugin));
    reg.register(Arc::new(c_lang::CLangPlugin));
    reg.register(Arc::new(cmake::CMakePlugin));
    reg.register(Arc::new(csharp::CSharpPlugin));
    reg.register(Arc::new(dart::DartPlugin));
    reg.register(Arc::new(dockerfile::DockerfilePlugin));
    reg.register(Arc::new(elixir::ElixirPlugin));
    reg.register(Arc::new(go::GoPlugin));
    reg.register(Arc::new(gleam::GleamPlugin));
    reg.register(Arc::new(graphql::GraphQlPlugin));
    reg.register(Arc::new(hare::HarePlugin));
    reg.register(Arc::new(haskell::HaskellPlugin));
    reg.register(Arc::new(hcl::HclPlugin));
    reg.register(Arc::new(html::HtmlPlugin));
    reg.register(Arc::new(java::JavaPlugin));
    reg.register(Arc::new(javascript::JavascriptPlugin));
    reg.register(Arc::new(kotlin::KotlinPlugin));
    reg.register(Arc::new(lua::LuaPlugin));
    reg.register(Arc::new(make::MakePlugin));
    reg.register(Arc::new(markdown::MarkdownPlugin));
    reg.register(Arc::new(mdx::MdxPlugin));
    reg.register(Arc::new(nim::NimPlugin));
    reg.register(Arc::new(nix::NixPlugin));
    reg.register(Arc::new(odin::OdinPlugin));
    reg.register(Arc::new(php::PhpPlugin));
    reg.register(Arc::new(prisma::PrismaPlugin));
    reg.register(Arc::new(proto::ProtoPlugin));
    reg.register(Arc::new(puppet::PuppetPlugin));
    reg.register(Arc::new(python::PythonPlugin));
    reg.register(Arc::new(r_lang::RLangPlugin));
    reg.register(Arc::new(razor::RazorPlugin));
    reg.register(Arc::new(robot::RobotPlugin));
    reg.register(Arc::new(ruby::RubyPlugin));
    reg.register(Arc::new(rust_lang::RustLangPlugin));
    reg.register(Arc::new(scala::ScalaPlugin));
    reg.register(Arc::new(scss::ScssPlugin));
    reg.register(Arc::new(sql::SqlPlugin));
    reg.register(Arc::new(starlark::StarlarkPlugin));
    reg.register(Arc::new(svelte::SveltePlugin));
    reg.register(Arc::new(swift::SwiftPlugin));
    reg.register(Arc::new(twig::TwigPlugin));
    reg.register(Arc::new(typescript::TypeScriptPlugin));
    reg.register(Arc::new(vue::VuePlugin));
    reg.register(Arc::new(yaml::YamlPlugin));
    reg.register(Arc::new(zig::ZigPlugin));
    // Wave 3
    reg.register(Arc::new(cobol::CobolPlugin));
    reg.register(Arc::new(pascal::PascalPlugin));
    reg.register(Arc::new(prolog::PrologPlugin));
    reg.register(Arc::new(vba::VbaPlugin));
    // Wave 2
    reg.register(Arc::new(ada::AdaPlugin));
    reg.register(Arc::new(clojure::ClojurePlugin));
    reg.register(Arc::new(fortran::FortranPlugin));
    reg.register(Arc::new(matlab::MatlabPlugin));
    reg.register(Arc::new(ocaml::OcamlPlugin));
    reg.register(Arc::new(vbnet::VbNetPlugin));
    // Wave 7
    reg.register(Arc::new(powershell::PowerShellPlugin));
    reg.register(Arc::new(groovy::GroovyPlugin));
    reg.register(Arc::new(perl::PerlPlugin));
    reg.register(Arc::new(erlang::ErlangPlugin));
    reg.register(Arc::new(fsharp::FSharpPlugin));
    reg.register(Arc::new(gdscript::GDScriptPlugin));
    // E5 — notebook family
    reg.register(Arc::new(jupyter::JupyterPlugin));
    reg.register(Arc::new(rmarkdown::RMarkdownPlugin));
    reg.register(Arc::new(rmarkdown::QuartoPlugin));
    reg.register(Arc::new(polyglot_nb::PolyglotNbPlugin));
    // E8 — Node template engines
    reg.register(Arc::new(handlebars::HandlebarsPlugin));
    reg.register(Arc::new(pug::PugPlugin));
    reg.register(Arc::new(ejs::EjsPlugin));
    reg.register(Arc::new(nunjucks::NunjucksPlugin));
    // E9 — Ruby template engines
    reg.register(Arc::new(erb::ErbPlugin));
    reg.register(Arc::new(slim::SlimPlugin));
    reg.register(Arc::new(haml::HamlPlugin));
    // E10 — Python templates, E11 — Liquid
    reg.register(Arc::new(jinja::JinjaPlugin));
    reg.register(Arc::new(liquid::LiquidPlugin));
    // E12 — Go templates + Templ
    reg.register(Arc::new(gotemplate::GoTemplatePlugin));
    reg.register(Arc::new(templ::TemplPlugin));
    // E13 — Phoenix HEEx
    reg.register(Arc::new(heex::HeexPlugin));
    // E22 — Elixir EEx
    reg.register(Arc::new(eex::EexPlugin));
    // E23 — Python Mako, PHP Smarty
    reg.register(Arc::new(mako::MakoPlugin));
    reg.register(Arc::new(smarty::SmartyPlugin));
    // E25 — Nginx
    reg.register(Arc::new(nginx::NginxPlugin));
    // E26 — systemd + crontab
    reg.register(Arc::new(systemd::SystemdPlugin));
    reg.register(Arc::new(crontab::CrontabPlugin));
    // E20 — JVM template engines
    reg.register(Arc::new(freemarker::FreemarkerPlugin));
    reg.register(Arc::new(jsp::JspPlugin));
    reg.register(Arc::new(velocity::VelocityPlugin));
    reg.register(Arc::new(gsp::GspPlugin));
    reg.register(Arc::new(thymeleaf::ThymeleafPlugin));
    // E21 — Yesod Shakespearean template plugins
    reg.register(Arc::new(shakespeare::HamletPlugin));
    reg.register(Arc::new(shakespeare::CassiusPlugin));
    reg.register(Arc::new(shakespeare::LuciusPlugin));
    reg.register(Arc::new(shakespeare::JuliusPlugin));

    reg
});

/// Return a reference to the shared default language registry.
///
/// The registry is built once on first access (all 59 plugins + lookup map)
/// and reused for the lifetime of the process. During the migration period
/// this coexists with the match statement in `indexer/full.rs`; once all
/// languages are migrated, the match disappears and this becomes the sole
/// dispatch mechanism.
pub fn default_registry() -> &'static LanguageRegistry {
    &DEFAULT_REGISTRY
}

// collect_plugin_connectors / drive_connector / drive_connector_incremental
// removed — all `impl Connector for X` blocks across language plugins were
// deleted or migrated to free `discover_*` functions during the connectors
// kill (Phases A–G). The legacy Connector trait is also slated for deletion
// in Phase H once the matcher + ConnectionPoint type are gone.

// ---------------------------------------------------------------------------
// Generic fallback plugin
// ---------------------------------------------------------------------------

/// Fallback plugin that handles any language with a tree-sitter grammar
/// but no dedicated extractor. Uses heuristic extraction based on common
/// node kinds across languages.
struct GenericPlugin;

impl LanguagePlugin for GenericPlugin {
    fn id(&self) -> &str {
        "generic"
    }

    fn language_ids(&self) -> &[&str] {
        // The generic plugin doesn't claim any specific IDs — it's the fallback.
        &[]
    }

    fn extensions(&self) -> &[&str] {
        &[]
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        crate::parser::languages::get_language(lang_id)
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        // The generic extractor has its own per-language scope configs.
        // Those are consulted inside `generic::extract()`.
        &[]
    }

    fn extract(&self, source: &str, _file_path: &str, lang_id: &str) -> ExtractionResult {
        match generic::extract::extract(source, lang_id) {
            Some(r) => ExtractionResult::new(r.symbols, r.refs, r.has_errors),
            None => ExtractionResult::empty(),
        }
    }
}
