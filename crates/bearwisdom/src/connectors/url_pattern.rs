// =============================================================================
// connectors/url_pattern.rs — URL-path pattern normalizer
//
// Reduces framework-specific path-parameter syntax to a canonical form so
// producer and consumer sides can pair regardless of which framework emitted
// the pattern:
//
//   Express / NestJS : /api/users/:id
//   Django           : /api/users/<id>
//   FastAPI / Flask  : /api/users/{id}
//   Internal emit    : /api/users/{}   (already canonical)
//
// All four normalize to  /api/users/{}.
//
// Query strings are stripped before normalization (only the path matters for
// routing).  A trailing slash is removed so /api/users/ and /api/users match.
// =============================================================================

/// Normalize a URL-path pattern to canonical form.
///
/// Canonical form uses `{}` for every path parameter segment, regardless of
/// the original framework syntax (`:id`, `<id>`, `{id}`, `{}`).
/// Query strings are stripped.  A trailing slash is removed.
/// Template-literal backticks, `scheme://host[:port]` prefixes, and a
/// leading `{}/` placeholder (template-substituted base URL) are stripped
/// before path-segment normalization so producer-side template literals
/// like `` `${base}/users` `` line up with bare consumer routes like
/// `/users`.
/// An empty input returns `/` for consistency with single-slash inputs.
pub fn normalize(raw: &str) -> String {
    // Strip template-literal backticks left over from the extractor's
    // template-string capture, e.g. `` `http://x/hello` `` → `http://x/hello`.
    let raw = raw.trim_matches('`');

    // Split off any query string — routing matches on the path only.
    let path = match raw.find('?') {
        Some(q) => &raw[..q],
        None => raw,
    };

    // Strip an absolute scheme://host[:port] prefix — only the path
    // participates in route matching.  Anything from the first `/` after
    // `scheme://host[:port]` onward becomes the new path; if there is no
    // such `/`, the URL is host-only and the path is `/`.
    let path = strip_scheme_authority(path);

    // Strip a leading template-substitution placeholder when it looks like
    // the host slot of a `${base}/rest` producer.  We only drop the
    // placeholder when it is the first segment AND something follows, so a
    // legitimate consumer route like `/{}/posts` (with `{}` as a path
    // parameter from a `:id` capture) is preserved.
    let path = strip_leading_host_placeholder(path);

    // Remove a trailing slash unless it is the root path itself.
    let path = if path.len() > 1 && path.ends_with('/') {
        &path[..path.len() - 1]
    } else {
        path
    };

    // Walk segment by segment and rewrite any path-parameter segment to `{}`.
    let mut out = String::with_capacity(path.len());
    for segment in path.split('/') {
        out.push('/');
        out.push_str(&normalize_segment(segment));
    }

    // The split above produces a leading '/' already; strip the extra one that
    // the loop prepended before the first (possibly empty) segment.
    if out.starts_with("//") {
        out.remove(0);
    }
    // Edge case: empty path → return "/" as-is.
    if out.is_empty() {
        out.push('/');
    }

    out
}

/// Strip a `scheme://host[:port]` prefix and return the remaining path.
/// Returns the input unchanged when no `://` separator is present.  When
/// the URL is host-only with no path, returns `/`.
fn strip_scheme_authority(path: &str) -> &str {
    let Some(after_scheme) = path.find("://").map(|i| &path[i + 3..]) else {
        return path;
    };
    match after_scheme.find('/') {
        Some(i) => &after_scheme[i..],
        None => "/",
    }
}

/// Strip a leading `{}/` template-substitution host slot, e.g. `{}/users`
/// → `/users`.  Only fires when `{}/` is the literal first segment AND at
/// least one more segment follows — `/{}/foo` (with a real path parameter
/// in the first slot) is preserved by the leading-`/` guard.
fn strip_leading_host_placeholder(path: &str) -> &str {
    if let Some(rest) = path.strip_prefix("{}/") {
        if !rest.is_empty() {
            // Re-prepend a single `/` so segment splitting still behaves
            // as if the path started with the host-less form.
            // We can't allocate here without changing the return type — but
            // the next caller prepends `/` per segment anyway, so simply
            // returning the rest works because the `split('/')` below
            // produces a leading empty segment when the input starts with
            // `/`, and an absent leading slash here has the same downstream
            // effect (the loop prepends `/` before every segment).
            return rest;
        }
    }
    path
}

/// Normalise one path segment.
///
/// Recognises:
///   - `:name`         — Express / NestJS / Rails
///   - `<name>`        — Django / Werkzeug
///   - `{name}`        — FastAPI / OpenAPI / Spring
///   - `{}`            — already canonical
///
/// Literal segments are returned unchanged.
fn normalize_segment(seg: &str) -> &str {
    if seg.is_empty() {
        return seg;
    }
    // :param  — Express-style
    if seg.starts_with(':') {
        return "{}";
    }
    // <param>  — Django-style (also catches typed variants like <int:pk>)
    if seg.starts_with('<') && seg.ends_with('>') {
        return "{}";
    }
    // {param} or {}  — OpenAPI / FastAPI / Spring / already canonical
    if seg.starts_with('{') && seg.ends_with('}') {
        return "{}";
    }
    seg
}

// ---------------------------------------------------------------------------
// HTTP-method compatibility predicate
// ---------------------------------------------------------------------------

/// Returns true when a producer's HTTP method is compatible with a consumer's
/// HTTP method for pairing purposes.
///
/// `Any` on either side is a wildcard: it matches any concrete method and any
/// other wildcard.  Two distinct concrete methods (e.g. `GET` and `POST`) are
/// not compatible.
pub fn http_methods_compatible(producer: Option<&str>, consumer: Option<&str>) -> bool {
    match (producer, consumer) {
        // Either side absent → treat as Any (wildcard matches everything).
        (None, _) | (_, None) => true,
        // Explicit wildcard token.
        (Some("*"), _) | (_, Some("*")) => true,
        // Both present and concrete — must agree.
        (Some(p), Some(c)) => p.eq_ignore_ascii_case(c),
    }
}

// ---------------------------------------------------------------------------
// Entity-name compatibility for DbQuery ↔ DbEntity pairing
// ---------------------------------------------------------------------------

/// Returns true when a `DbQuery.entity_name` matches a `DbEntity` key.
///
/// Matching is case-insensitive and allows simple plural/singular differences:
///   - `User`  ↔ `users`  (class name vs lowercase-plural table name)
///   - `users` ↔ `user`   (table name vs singular class name)
///
/// The algorithm is intentionally conservative — only suffix-`s` pluralization
/// is handled, which covers the overwhelming majority of ORM entity names in
/// the test corpus.  More complex inflections (e.g. `Person`/`people`) are
/// left to fall through to exact-match failure; false positives from over-eager
/// inflection heuristics would be worse than misses here.
pub fn entity_names_match(query_name: &str, entity_key: &str) -> bool {
    // Strip language prefixes so cross-language entities pair correctly.
    // Producers tag entities with their lang prefix (e.g. `py.User`, `rs.User`,
    // `cs.Users`, `java.user`); the pairer ignores the prefix when matching.
    let q = strip_lang_prefix(query_name);
    let e = strip_lang_prefix(entity_key);
    if q.eq_ignore_ascii_case(e) {
        return true;
    }
    // Wildcard entity (`*`) on either side matches anything from the other.
    if q == "*" || e == "*" {
        return true;
    }
    // Strip a trailing 's' from either side and compare again
    // (pluralization tolerance: `User` ~ `users`).
    let q_lower = q.to_ascii_lowercase();
    let e_lower = e.to_ascii_lowercase();
    let q_stem = q_lower.strip_suffix('s').unwrap_or(&q_lower);
    let e_stem = e_lower.strip_suffix('s').unwrap_or(&e_lower);
    q_stem == e_stem
}

/// Strip a known language prefix (`py.`, `rs.`, `cs.`, `java.`, `kt.`, …)
/// from an entity name so cross-language pairing ignores the producer's
/// language tag. Returns the input unchanged if no recognised prefix is
/// present.
fn strip_lang_prefix(s: &str) -> &str {
    const PREFIXES: &[&str] = &[
        "py.", "rs.", "cs.", "java.", "kt.", "go.", "php.", "rb.", "ex.", "scala.", "swift.",
        "fs.", "dart.", "clj.", "hs.", "ml.", "groovy.", "pl.", "gleam.", "nim.", "lua.", "c.",
        "pas.", "ada.", "cobol.", "matlab.", "r.", "erl.", "ts.", "js.",
    ];
    for p in PREFIXES {
        if let Some(rest) = s.strip_prefix(p) {
            return rest;
        }
    }
    s
}

#[cfg(test)]
#[path = "url_pattern_tests.rs"]
mod tests;
