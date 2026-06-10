// =============================================================================
// indexer/resolve/adapters/nextjs.rs — Next.js route file Consumer emissions
//
// Recognises App Router (`**/app/**/route.{ts,tsx,js,jsx}`) and Pages
// Router (`**/pages/api/**`) route files and emits one Consumer
// NamedChannel HttpCall per discovered route. The URL pattern is the
// project-relative path with the framework-specific prefix stripped and
// `[id]` / `[...slug]` / `[[...slug]]` segments normalised to `{}`
// so it pairs cleanly against generic-URL Producers from any language.
// =============================================================================

use super::super::flow_emit;

/// Recognise Next.js HTTP route files and return Consumer NamedChannel
/// HttpCall emissions for them. Two route conventions are supported:
///
/// - **App Router** (`**/app/**/route.{ts,tsx,js,jsx}`): one Consumer per
///   exported HTTP-verb function (`GET` / `POST` / `PUT` / `PATCH` /
///   `DELETE` / `HEAD` / `OPTIONS`). The URL pattern is the file's path with
///   the `app/` ancestor stripped, the `route.<ext>` basename dropped, and
///   `[id]` / `[...slug]` / `[[...slug]]` segments rewritten to `{}`.
/// - **Pages Router** (`**/pages/api/**/*.{ts,…}`): a single Any-method
///   Consumer per file (Pages Router handlers dispatch internally on
///   `req.method`). URL pattern excludes the `.ts` extension and rewrites
///   `[id]` segments the same way; the special `index` basename is
///   collapsed to the directory.
///
/// Returns an empty Vec when the path doesn't match either convention or
/// when the App Router file exports no recognised HTTP-verb handlers.
pub(crate) fn nextjs_route_consumer_emissions(
    path: &str,
    symbols: &[crate::types::ExtractedSymbol],
) -> Vec<flow_emit::FlowEmission> {
    let segs: Vec<&str> = path.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return Vec::new();
    }
    let basename = *segs.last().unwrap();

    // App Router: `app/**/route.{ts,tsx,js,jsx}` — exports name the verbs.
    if matches!(
        basename,
        "route.ts" | "route.tsx" | "route.js" | "route.jsx"
    ) {
        let Some(app_idx) = segs.iter().rposition(|s| *s == "app") else {
            return Vec::new();
        };
        // Drop the `route.<ext>` basename and rewrite dynamic segments.
        let url_segs: Vec<String> = segs[app_idx + 1..segs.len() - 1]
            .iter()
            .map(|s| rewrite_nextjs_segment(s))
            .collect();
        let url = if url_segs.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", url_segs.join("/"))
        };
        let verbs = exported_http_verb_handlers(symbols);
        if !verbs.is_empty() {
            return verbs
                .into_iter()
                .map(|method| flow_emit::FlowEmission::NamedChannel {
                    kind: flow_emit::NamedChannelKind::HttpCall,
                    name: url.clone(),
                    role: flow_emit::ChannelRole::Consumer,
                    method: Some(method),
                    streaming: None,
                })
                .collect();
        }
        // Fallback: Next.js App Router requires route files to export at
        // least one HTTP verb. Re-export-rename patterns
        // (`export { handler as GET, handler as POST }`) currently aren't
        // synthesised as named symbols when the original is a local
        // declaration. Emit a single Any-method Consumer so the file still
        // pairs against any-method Producers (incl. tRPC client emissions).
        return vec![flow_emit::FlowEmission::NamedChannel {
            kind: flow_emit::NamedChannelKind::HttpCall,
            name: url,
            role: flow_emit::ChannelRole::Consumer,
            method: Some(flow_emit::HttpMethod::Any),
            streaming: None,
        }];
    }

    // Pages Router: `pages/api/**` with any TS/JS extension. Skip files
    // whose basename starts with `_` (Next.js convention for non-route).
    let pages_idx = segs.iter().rposition(|s| *s == "pages");
    let api_after_pages = pages_idx.and_then(|p_idx| {
        segs.get(p_idx + 1)
            .filter(|s| **s == "api")
            .map(|_| p_idx + 1)
    });
    if let Some(api_idx) = api_after_pages {
        if basename.starts_with('_') {
            return Vec::new();
        }
        let stem = basename
            .rsplit_once('.')
            .map(|(s, _)| s)
            .unwrap_or(basename);
        if !matches!(
            basename.rsplit_once('.').map(|(_, ext)| ext).unwrap_or(""),
            "ts" | "tsx" | "js" | "jsx" | "mjs"
        ) {
            return Vec::new();
        }
        let mut url_segs: Vec<String> = segs[api_idx..segs.len() - 1]
            .iter()
            .map(|s| rewrite_nextjs_segment(s))
            .collect();
        // Pages convention: `pages/api/users/index.ts` → `/api/users`.
        if stem != "index" {
            url_segs.push(rewrite_nextjs_segment(stem));
        }
        let url = format!("/{}", url_segs.join("/"));
        return vec![flow_emit::FlowEmission::NamedChannel {
            kind: flow_emit::NamedChannelKind::HttpCall,
            name: url,
            role: flow_emit::ChannelRole::Consumer,
            method: Some(flow_emit::HttpMethod::Any),
            streaming: None,
        }];
    }

    Vec::new()
}

/// Rewrite a single Next.js route segment for URL normalization:
/// - `[id]` → `{}`           (single dynamic segment)
/// - `[...slug]` → `{}`      (catch-all)
/// - `[[...slug]]` → `{}`    (optional catch-all)
/// - `(group)` → ``          (route group; consumed by the join, see below)
/// - literal segments returned unchanged.
///
/// Route groups (`(marketing)`) are returned as empty strings; the caller is
/// expected to filter empties out of the joined URL.
fn rewrite_nextjs_segment(seg: &str) -> String {
    if seg.starts_with('(') && seg.ends_with(')') {
        return String::new();
    }
    if seg.starts_with("[[...") && seg.ends_with("]]") {
        return "{}".to_string();
    }
    if seg.starts_with("[...") && seg.ends_with(']') {
        return "{}".to_string();
    }
    if seg.starts_with('[') && seg.ends_with(']') {
        return "{}".to_string();
    }
    seg.to_string()
}

/// Return the set of HTTP methods exported as named functions from a file.
/// Recognises the canonical Next.js App Router handler names (uppercase
/// HTTP verbs). Order is fixed (Get, Post, Put, Patch, Delete, Head, Options)
/// for deterministic emission across runs.
fn exported_http_verb_handlers(
    symbols: &[crate::types::ExtractedSymbol],
) -> Vec<flow_emit::HttpMethod> {
    use flow_emit::HttpMethod;
    let mut found = [false; 7];
    for sym in symbols {
        match sym.name.as_str() {
            "GET" => found[0] = true,
            "POST" => found[1] = true,
            "PUT" => found[2] = true,
            "PATCH" => found[3] = true,
            "DELETE" => found[4] = true,
            "HEAD" => found[5] = true,
            "OPTIONS" => found[6] = true,
            _ => {}
        }
    }
    let order = [
        HttpMethod::Get,
        HttpMethod::Post,
        HttpMethod::Put,
        HttpMethod::Patch,
        HttpMethod::Delete,
        HttpMethod::Head,
        HttpMethod::Options,
    ];
    found
        .iter()
        .zip(order.iter())
        .filter_map(|(f, m)| if *f { Some(*m) } else { None })
        .collect()
}
