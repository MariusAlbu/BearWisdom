// =============================================================================
// languages/haskell/servant.rs  —  Servant API type-alias route extraction
//
// A Servant API is a type alias whose right-hand side composes path
// fragments and verb terminals using the `:>` and `:<|>` type operators:
//
//   type API =
//          "users" :> Get '[JSON] [User]
//     :<|> "users" :> Capture "id" Int :> Get '[JSON] User
//     :<|> "users" :> ReqBody '[JSON] User :> Post '[JSON] User
//
// Each `:<|>` alternative is an independent route. Within an alternative
// the `:>` chain ends in a verb terminal (`Get`, `Post`, `Put`, `Patch`,
// `Delete`, `Head`, `Options`). Path segments come from:
//   - String literals → literal path segment
//   - `Capture "name" Type` → `{name}` parametric segment
//   - `CaptureAll "name" [Type]` → `{name}` parametric segment
//   - `QueryParam`, `Header`, `ReqBody` → metadata, not part of the URL
//
// Implementation is text-based on the type-alias source. The AST shape
// changes between tree-sitter-haskell versions, but the textual operators
// `:>` and `:<|>` are stable.
// =============================================================================

use crate::types::ExtractedRoute;

pub(crate) fn extract_servant_routes(
    synonym_src: &str,
    handler_symbol_index: usize,
    routes: &mut Vec<ExtractedRoute>,
) {
    // Strip the `type Name = ` prefix so we're left with just the body.
    let body = match synonym_src.find('=') {
        Some(i) => &synonym_src[i + 1..],
        None => synonym_src,
    };
    // Quick reject: no Servant operators present.
    if !body.contains(":>") && !body.contains(":<|>") {
        return;
    }
    for alt in body.split(":<|>") {
        if let Some(route) = parse_servant_alternative(alt, handler_symbol_index) {
            routes.push(route);
        }
    }
}

fn parse_servant_alternative(alt: &str, handler_symbol_index: usize) -> Option<ExtractedRoute> {
    let mut segments: Vec<String> = Vec::new();
    let mut http_method: Option<&'static str> = None;
    for raw_seg in alt.split(":>") {
        let seg = raw_seg.trim();
        if seg.is_empty() {
            continue;
        }
        // String literal: `"users"` or `"api/v1"`.
        if let Some(lit) = seg.strip_prefix('"').and_then(|s| s.split('"').next()) {
            if !lit.is_empty() {
                segments.push(lit.to_string());
            }
            continue;
        }
        // `Capture "id" Int` or `Capture' '[Strict] "id" Int`.
        if seg.starts_with("Capture") {
            if let Some(name) = extract_quoted_name(seg) {
                segments.push(format!("{{{}}}", name));
            }
            continue;
        }
        // `CaptureAll "rest" Text`.
        if seg.starts_with("CaptureAll") {
            if let Some(name) = extract_quoted_name(seg) {
                segments.push(format!("{{{}}}", name));
            }
            continue;
        }
        // Verb terminal: `Get '[JSON] User`, `Post '[JSON] User`, etc.
        if let Some(verb) = servant_verb_prefix(seg) {
            http_method = Some(verb);
            continue;
        }
        // Other combinators (`ReqBody`, `QueryParam`, `Header`, `Summary`,
        // `Description`) carry metadata, not URL structure — ignore.
    }
    let method = http_method?;
    let template = if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segments.join("/"))
    };
    Some(ExtractedRoute {
        handler_symbol_index,
        http_method: method.to_string(),
        template,
    })
}

fn servant_verb_prefix(seg: &str) -> Option<&'static str> {
    // Match the leading identifier, treating non-letter chars as boundaries.
    let head: String = seg.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    match head.as_str() {
        "Get" => Some("GET"),
        "Post" => Some("POST"),
        "Put" => Some("PUT"),
        "Patch" => Some("PATCH"),
        "Delete" => Some("DELETE"),
        "Head" => Some("HEAD"),
        "Options" => Some("OPTIONS"),
        // `Verb 'GET 200 '[JSON] X` — explicit Verb form.
        "Verb" => {
            let after = seg.trim_start_matches("Verb").trim_start();
            let stripped = after.trim_start_matches('\'');
            let v: String = stripped
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            match v.as_str() {
                "GET" => Some("GET"),
                "POST" => Some("POST"),
                "PUT" => Some("PUT"),
                "PATCH" => Some("PATCH"),
                "DELETE" => Some("DELETE"),
                "HEAD" => Some("HEAD"),
                "OPTIONS" => Some("OPTIONS"),
                _ => None,
            }
        }
        _ => None,
    }
}

fn extract_quoted_name(seg: &str) -> Option<String> {
    let rest = seg.split_once('"')?.1;
    let name = rest.split('"').next()?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

#[cfg(test)]
#[path = "servant_tests.rs"]
mod servant_tests;
