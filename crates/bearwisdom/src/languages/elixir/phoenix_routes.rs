// =============================================================================
// languages/elixir/phoenix_routes.rs — Phoenix route helper synthesis
//
// Phoenix's `use Phoenix.Router` macro walks the router DSL at compile time
// and injects a family of `*_path` / `*_url` helper functions onto an
// internal `Helpers` module (accessed via `Routes` alias in most projects).
// These helpers never appear as source-defined symbols — they're purely
// compile-time artefacts of the Phoenix macro system.
//
// BearWisdom doesn't execute Elixir macros, so without synthesis every
// call like `Routes.user_index_path(conn, :index)` would stay unresolved.
// Sprint A.5 worked around this by externalising the literal `Routes`
// module name, which dropped the top receiver target but left the leaf
// helper names dangling.
//
// This module parses router.ex files via regex (no AST — the
// tree-sitter-elixir grammar doesn't expose the DSL clearly enough) and
// synthesises one `Function` symbol per derived helper name. Synthesis is
// driven by the Elixir extractor's post-visit hook when the source module
// imports Phoenix.Router.
//
// Helper name derivation mirrors Phoenix.Router's own internal logic:
//
//   scope "/admin", MyAppWeb.Admin, as: :admin do
//     resources "/users", UserController, only: [:index, :show]
//     get "/login", SessionController, :new, as: :login
//   end
//
// produces:
//   admin_user_index_path   admin_user_index_url      (from resources)
//   admin_user_show_path    admin_user_show_url       (from resources)
//   admin_login_path        admin_login_url           (from as: :login)
//
// The synthesis is a best-effort pass — it covers the common verb/resources/
// live macros with optional `as:` aliases and nested scopes, but doesn't
// attempt to handle pipe_through, forward, or custom DSL extensions. That's
// acceptable because we only need the names to exist in the symbol index;
// the resolver matches by leaf name.
// =============================================================================

use regex::Regex;

use crate::types::{ExtractedSymbol, SymbolKind, Visibility};

/// Parse `source` for Phoenix route declarations and append synthesised
/// helper symbols (`*_path` and `*_url`) to `symbols`. Called by the Elixir
/// extractor immediately after the main tree-sitter walk, gated on the
/// source containing `use Phoenix.Router` or `Phoenix.Router` aliased in
/// scope.
pub fn synthesize_route_helpers(source: &str, symbols: &mut Vec<ExtractedSymbol>) {
    let re_scope_as = Regex::new(
        r#"(?x)
        ^\s*scope\s+
        (?:"[^"]*"\s*,\s*)?            # optional "/path" arg
        [^,]+,\s*                       # alias module
        as:\s*:(\w+)                    # as: :name
        "#,
    ).expect("phoenix scope-as regex");
    let re_scope_bare = Regex::new(r#"^\s*scope\s+"#).expect("phoenix scope regex");
    let re_end = Regex::new(r"^\s*end\s*$").expect("phoenix end regex");

    // `get "/path", Controller, :action [, as: :custom]`
    let re_verb = Regex::new(
        r#"(?x)
        ^\s*
        (?:get|post|put|patch|delete|options|head)    # HTTP verb
        \s+"[^"]*"\s*,\s*                              # path string
        ([A-Z][\w.]*Controller)\s*,\s*                 # Controller module
        :(\w+)                                         # :action
        (?:.*?as:\s*:(\w+))?                           # optional as: :alias
        "#,
    ).expect("phoenix verb regex");

    // `resources "/path", Controller [, only/except: [...]] [, as: :alias] [do]`
    // The trailing `do` (with or without `end` on the same line) indicates a
    // nested block — child routes declared inside use the parent resource name
    // as an additional prefix segment. Phoenix generates e.g.
    //   podcast_episode_path  from  resources "/podcasts", PodcastController do
    //                                 resources "/episodes", EpisodeController
    //                               end
    //
    // We use two separate regexes: one for the controller name, and one each
    // for `only:` and `as:` options. This avoids ordering problems where `.*?`
    // in a combined regex can consume one option before the other is captured.
    let re_resources = Regex::new(
        r#"(?x)
        ^\s*resources\s+
        "[^"]*"\s*,\s*                                 # path
        ([A-Z][\w.]*Controller)                        # Controller module
        "#,
    ).expect("phoenix resources regex");

    // Option-specific regexes applied to the full line independently.
    let re_resources_only = Regex::new(r#"(?x)\bonly:\s*\[([^\]]*)\]"#)
        .expect("resources-only regex");
    let re_resources_as = Regex::new(r#"(?x)\bas:\s*:(\w+)"#)
        .expect("resources-as regex");

    // Detects whether a `resources` line (or any route line) ends with a bare
    // `do` keyword that opens a nested block.  We check separately because the
    // controller-capture regex uses `.*?` optional groups that may consume the
    // trailing ` do` before the named capture sees it when the line also
    // contains `except:` or other options.
    let re_line_opens_block = Regex::new(r"\bdo\s*$").expect("do-at-eol regex");

    // `live "/path", LiveModule, :action [, as: :alias]`
    let re_live = Regex::new(
        r#"(?x)
        ^\s*live\s+
        "[^"]*"\s*,\s*
        ([A-Z][\w.]*(?:Live|LiveView))(?:\s*,\s*:(\w+))?
        (?:.*?as:\s*:(\w+))?
        "#,
    ).expect("phoenix live regex");

    // Scope stack — each entry is `(depth, name_segment)`:
    //   - `depth` is the do/end nesting depth at which this entry was pushed.
    //   - `name_segment` is the path prefix contributed by this scope level
    //     (empty for bare `scope "..."` without `as:`).
    //
    // We track a global nesting depth counter separately.  Every line that
    // opens a `do` block (scope, resources, pipeline, for, defmodule, etc.)
    // increments the depth.  Every standalone `end` decrements it.  Scope /
    // resources entries are popped when the depth falls back to their push
    // depth — this way `for ... do ... end` and `pipeline ... do ... end`
    // blocks nested inside a scope don't accidentally pop the scope entry.
    let mut nesting_depth: i32 = 0;
    let mut scope_stack: Vec<(i32, String)> = Vec::new();
    let mut synthesized = std::collections::HashSet::new();

    // Regex for any line that ends with `do` (opens a block).
    // Used to track nesting depth for non-scope do/end pairs.
    let re_any_do = Regex::new(r"\bdo\s*$").expect("any-do regex");

    for line in source.lines() {
        // scope "/admin", Mod, as: :admin do
        if let Some(cap) = re_scope_as.captures(line) {
            nesting_depth += 1;
            scope_stack.push((nesting_depth, cap[1].to_string()));
            continue;
        }
        if re_scope_bare.is_match(line) {
            // Bare scope without `as:` — still opens a depth level.
            nesting_depth += 1;
            scope_stack.push((nesting_depth, String::new()));
            continue;
        }
        if re_end.is_match(line) {
            // Pop any scope entries that were pushed at this depth.
            scope_stack.retain(|(d, _)| *d != nesting_depth);
            nesting_depth = (nesting_depth - 1).max(0);
            continue;
        }
        let scope_prefix: String = scope_stack
            .iter()
            .filter(|(_, s)| !s.is_empty())
            .map(|(_, s)| s.as_str())
            .collect::<Vec<_>>()
            .join("_");

        // resources "/users", UserController [do]
        if let Some(cap) = re_resources.captures(line) {
            let controller = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            // Capture `only:` and `as:` independently from the full line so
            // that their relative order doesn't matter.
            let only = re_resources_only.captures(line)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str())
                .unwrap_or("");
            let explicit_as = re_resources_as.captures(line)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_string());
            // Use a separate check for trailing `do` — the capture regex's
            // optional `.*?` groups can consume ` do` when the line also
            // contains `except:` or similar options before `do`.
            let opens_block = re_line_opens_block.is_match(line);

            let base = explicit_as.unwrap_or_else(|| resource_singular(controller));
            let prefixed = join_scope(&scope_prefix, &base);

            // If `only:` is present, only emit the listed actions. Otherwise
            // emit the full RESTful 7 standard Phoenix helpers.
            let actions: Vec<&str> = if only.is_empty() {
                vec!["index", "show", "new", "edit", "create", "update", "delete"]
            } else {
                parse_action_list(only)
            };

            for action in actions {
                // Phoenix naming convention:
                //   index  -> <resource>_path
                //   show   -> <resource>_path (takes id)
                //   new    -> <resource>_new_path
                //   edit   -> <resource>_edit_path
                //   create -> <resource>_path
                //   update -> <resource>_path
                //   delete -> <resource>_path
                // Several actions share the base helper name; we always emit
                // `<resource>_path` once, and the distinguishing `_new_path`
                // / `_edit_path` variants when those actions are present.
                let name = match action {
                    "new" => format!("{prefixed}_new_path"),
                    "edit" => format!("{prefixed}_edit_path"),
                    _ => format!("{prefixed}_path"),
                };
                push_helper(&name, &mut synthesized, symbols);
            }

            // Push the resource name as a scope prefix for nested routes declared
            // inside this `resources ... do` block. Phoenix folds the parent
            // resource name into child helper names:
            //   resources "/podcasts", PodcastController do
            //     resources "/episodes", EpisodeController
            //   end
            // → podcast_episode_path (no admin prefix) or admin_podcast_episode_path
            //   when the outer scope has as: :admin.
            if opens_block {
                nesting_depth += 1;
                scope_stack.push((nesting_depth, base.clone()));
            }
            continue;
        }

        // get/post/put/... "/path", Controller, :action [, as: :alias]
        if let Some(cap) = re_verb.captures(line) {
            let controller = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            let action = cap.get(2).map(|m| m.as_str()).unwrap_or("");
            let explicit_as = cap.get(3).map(|m| m.as_str().to_string());

            let base = explicit_as.unwrap_or_else(|| derive_verb_helper(controller, action));
            let prefixed = join_scope(&scope_prefix, &base);
            let name = format!("{prefixed}_path");
            push_helper(&name, &mut synthesized, symbols);
            continue;
        }

        // live "/path", LiveModule, :action
        if let Some(cap) = re_live.captures(line) {
            let module = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            let explicit_as = cap.get(3).map(|m| m.as_str().to_string());
            let base = explicit_as.unwrap_or_else(|| live_helper_name(module));
            let prefixed = join_scope(&scope_prefix, &base);
            let name = format!("{prefixed}_path");
            push_helper(&name, &mut synthesized, symbols);
            // `live` routes don't open nested blocks in practice; no continue needed
            // for depth tracking — fall through to `re_any_do` which won't match
            // since `live` DSL lines don't typically end in `do`.
            continue;
        }

        // Track depth for any other `do`-block openers (pipeline, for, if, etc.)
        // that we don't push named entries for — they still affect nesting depth.
        // This MUST come last so that route-handling arms above (which use
        // `continue`) have already exited before we get here. Scope lines
        // (`scope ... do`) are handled earlier via `continue` too.
        if re_any_do.is_match(line) {
            nesting_depth += 1;
        }
    }
}

fn push_helper(
    name: &str,
    seen: &mut std::collections::HashSet<String>,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    // Emit both `*_path` (passed in) and the `*_url` twin.
    if seen.insert(name.to_string()) {
        symbols.push(make_helper_symbol(name));
    }
    let url = name.replace("_path", "_url");
    if url != name && seen.insert(url.clone()) {
        symbols.push(make_helper_symbol(&url));
    }
}

fn make_helper_symbol(name: &str) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("def {name}(conn, params \\\\ %{{}})")),
        doc_comment: Some(
            "Auto-synthesised Phoenix route helper (compile-time macro).".to_string(),
        ),
        scope_path: Some("Routes".to_string()),
        parent_index: None,
    }
}

/// Derive the singular resource name from a controller module name.
/// `UserController`            → `user`
/// `Admin.PostController`      → `post`
/// `Admin.Blog.EntryController` → `entry`
fn resource_singular(controller: &str) -> String {
    // Take the last dotted segment.
    let leaf = controller.rsplit('.').next().unwrap_or(controller);
    let stem = leaf.strip_suffix("Controller").unwrap_or(leaf);
    to_snake_case(stem)
}

/// For bare verb routes without `as:`, Phoenix uses `<singular>_<action>`.
/// e.g. `get "/login", SessionController, :new` → `session_new`.
fn derive_verb_helper(controller: &str, action: &str) -> String {
    let stem = resource_singular(controller);
    format!("{stem}_{action}")
}

/// LiveView modules usually derive helpers by the module leaf name.
fn live_helper_name(module: &str) -> String {
    let leaf = module.rsplit('.').next().unwrap_or(module);
    let stem = leaf
        .strip_suffix("LiveView")
        .or_else(|| leaf.strip_suffix("Live"))
        .unwrap_or(leaf);
    to_snake_case(stem)
}

/// CamelCase → snake_case. `UserProfile` → `user_profile`.
fn to_snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(ch.to_lowercase());
    }
    out
}

/// Parse an `only: [:index, :show]`-style action list. Atoms with leading
/// colons get stripped. Whitespace around commas is ignored.
fn parse_action_list(list_src: &str) -> Vec<&str> {
    list_src
        .split(',')
        .map(|s| s.trim().trim_start_matches(':'))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Join an optional scope prefix with a base helper name. Either may be
/// empty; an empty scope returns the base unchanged.
fn join_scope(scope_prefix: &str, base: &str) -> String {
    if scope_prefix.is_empty() {
        base.to_string()
    } else {
        format!("{scope_prefix}_{base}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn helper_names(source: &str) -> Vec<String> {
        let mut symbols = Vec::new();
        synthesize_route_helpers(source, &mut symbols);
        let mut names: Vec<String> = symbols.into_iter().map(|s| s.name).collect();
        names.sort();
        names
    }

    #[test]
    fn bare_verb_route() {
        let src = r#"
defmodule MyAppWeb.Router do
  use Phoenix.Router
  get "/login", SessionController, :new
end
"#;
        let names = helper_names(src);
        assert!(names.contains(&"session_new_path".to_string()));
        assert!(names.contains(&"session_new_url".to_string()));
    }

    #[test]
    fn verb_route_with_as_alias() {
        let src = r#"
defmodule MyAppWeb.Router do
  use Phoenix.Router
  get "/login", SessionController, :new, as: :login
end
"#;
        let names = helper_names(src);
        assert!(names.contains(&"login_path".to_string()));
        assert!(names.contains(&"login_url".to_string()));
    }

    #[test]
    fn resources_macro_emits_seven_helpers() {
        let src = r#"
defmodule MyAppWeb.Router do
  use Phoenix.Router
  resources "/users", UserController
end
"#;
        let names = helper_names(src);
        // index/show/create/update/delete collapse to user_path; new and edit
        // get their own variants.
        assert!(names.contains(&"user_path".to_string()));
        assert!(names.contains(&"user_url".to_string()));
        assert!(names.contains(&"user_new_path".to_string()));
        assert!(names.contains(&"user_edit_path".to_string()));
    }

    #[test]
    fn resources_with_only_filter() {
        let src = r#"
defmodule MyAppWeb.Router do
  use Phoenix.Router
  resources "/posts", PostController, only: [:index, :show]
end
"#;
        let names = helper_names(src);
        assert!(names.contains(&"post_path".to_string()));
        assert!(!names.contains(&"post_new_path".to_string()));
    }

    #[test]
    fn scoped_route_prepends_scope_alias() {
        let src = r#"
defmodule MyAppWeb.Router do
  use Phoenix.Router
  scope "/admin", MyAppWeb.Admin, as: :admin do
    resources "/users", UserController, only: [:index, :show]
    get "/login", SessionController, :new, as: :login
  end
end
"#;
        let names = helper_names(src);
        assert!(names.contains(&"admin_user_path".to_string()));
        assert!(names.contains(&"admin_login_path".to_string()));
        assert!(names.contains(&"admin_login_url".to_string()));
    }

    #[test]
    fn nested_resources_generates_compound_helpers() {
        let src = r#"
defmodule ChangelogWeb.Router do
  use Phoenix.Router
  scope "/admin", ChangelogWeb.Admin, as: :admin do
    resources "/podcasts", PodcastController do
      resources "/episodes", EpisodeController
      resources "/episode_requests", EpisodeRequestController
    end
  end
end
"#;
        let names = helper_names(src);
        // Parent resource helpers still emitted
        assert!(names.contains(&"admin_podcast_path".to_string()));
        // Nested: admin + podcast + episode
        assert!(names.contains(&"admin_podcast_episode_path".to_string()));
        assert!(names.contains(&"admin_podcast_episode_url".to_string()));
        assert!(names.contains(&"admin_podcast_episode_request_path".to_string()));
    }

    #[test]
    fn non_scope_do_end_does_not_break_scope_stack() {
        // `pipeline` and `for` blocks have their own `do`/`end` — they must not
        // pop the enclosing `scope` entry off the stack.
        let src = r#"
defmodule ChangelogWeb.Router do
  use Phoenix.Router

  pipeline :browser do
    plug :accepts, ["html"]
  end

  scope "/admin", ChangelogWeb.Admin, as: :admin do
    pipe_through [:browser, :admin]

    resources "/news/items", NewsItemController, except: [:show] do
      resources "/subscriptions", NewsItemSubscriptionController, as: :subscription, only: [:index]
    end

    for provider <- ~w(github) do
      get "/auth/#{provider}", AuthController, :request
    end

    resources "/podcasts", PodcastController do
      resources "/episodes", EpisodeController
    end
  end
end
"#;
        let names = helper_names(src);
        // Scoped top-level resources
        assert!(names.contains(&"admin_news_item_path".to_string()), "admin_news_item_path missing; got: {names:?}");
        // Nested resources inside `resources ... do`
        assert!(names.contains(&"admin_news_item_subscription_path".to_string()), "admin_news_item_subscription_path missing");
        assert!(names.contains(&"admin_podcast_path".to_string()), "admin_podcast_path missing");
        assert!(names.contains(&"admin_podcast_episode_path".to_string()), "admin_podcast_episode_path missing");
        assert!(names.contains(&"admin_podcast_episode_url".to_string()), "admin_podcast_episode_url missing");
    }

    #[test]
    fn no_helpers_without_phoenix_router() {
        let src = "defmodule NotARouter do\n  def hello, do: :world\nend\n";
        assert!(helper_names(src).is_empty());
    }

    #[test]
    fn snake_case_conversion() {
        assert_eq!(to_snake_case("User"), "user");
        assert_eq!(to_snake_case("UserProfile"), "user_profile");
        assert_eq!(to_snake_case("HTMLParser"), "h_t_m_l_parser");
    }
}
