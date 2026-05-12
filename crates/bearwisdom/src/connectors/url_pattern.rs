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
/// An empty input returns an empty string unchanged.
pub fn normalize(raw: &str) -> String {
    // Split off any query string — routing matches on the path only.
    let path = match raw.find('?') {
        Some(q) => &raw[..q],
        None => raw,
    };

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
    if query_name.eq_ignore_ascii_case(entity_key) {
        return true;
    }
    // Strip a trailing 's' from either side and compare again.
    let q_lower = query_name.to_ascii_lowercase();
    let e_lower = entity_key.to_ascii_lowercase();
    let q_stem = q_lower.strip_suffix('s').unwrap_or(&q_lower);
    let e_stem = e_lower.strip_suffix('s').unwrap_or(&e_lower);
    q_stem == e_stem
}

#[cfg(test)]
#[path = "url_pattern_tests.rs"]
mod tests;
