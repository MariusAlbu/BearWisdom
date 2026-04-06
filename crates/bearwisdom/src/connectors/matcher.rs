// =============================================================================
// connectors/matcher.rs — Protocol-specific matching strategies
//
// `ProtocolMatcher::match_protocol` is the central dispatch: given a protocol
// and two slices of connection points (starts and stops), it returns all
// matched `ResolvedFlow` pairs.
//
// REST matching reuses `normalise_route` and `routes_match` from http_api.rs.
// All other protocols use exact-key matching, with MessageQueue additionally
// requiring the same framework discriminator.
// =============================================================================

use super::http_api::{normalise_route, routes_match};
use super::types::{ConnectionPoint, FlowDirection, Protocol, ResolvedFlow};

pub struct ProtocolMatcher;

impl ProtocolMatcher {
    /// Match starts against stops for the given protocol.
    ///
    /// Dispatches to the per-protocol strategy below.  Callers should ensure
    /// `starts` and `stops` are already filtered to the correct protocol and
    /// direction — this function trusts that invariant.
    pub fn match_protocol(
        protocol: Protocol,
        starts: &[&ConnectionPoint],
        stops: &[&ConnectionPoint],
    ) -> Vec<ResolvedFlow> {
        match protocol {
            Protocol::Rest => match_rest(starts, stops),
            Protocol::Grpc => match_exact(starts, stops, "grpc_call", false),
            Protocol::GraphQl => match_exact(starts, stops, "graphql_call", false),
            Protocol::MessageQueue => match_exact(starts, stops, "message_queue", true),
            Protocol::EventBus => match_exact(starts, stops, "event_handler", false),
            Protocol::WebSocket => match_exact(starts, stops, "websocket", false),
            Protocol::Ffi => match_exact(starts, stops, "ffi_call", false),
            Protocol::Ipc => match_exact(starts, stops, "ipc_call", false),
            Protocol::Di => match_exact(starts, stops, "di_binding", false),
            // Infrastructure connectors (DockerCompose, Kubernetes) always supply a
            // custom_match, so this arm is never reached in practice.  Fall back to
            // exact-key matching as a safe default.
            Protocol::Infrastructure => {
                match_exact(starts, stops, "infrastructure_dependency", false)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// REST
// ---------------------------------------------------------------------------

fn match_rest(starts: &[&ConnectionPoint], stops: &[&ConnectionPoint]) -> Vec<ResolvedFlow> {
    let mut flows = Vec::new();

    // Pre-normalise stop keys once.
    let norm_stops: Vec<(&ConnectionPoint, String)> = stops
        .iter()
        .map(|cp| (*cp, normalise_route(&cp.key)))
        .filter(|(_, norm)| has_literal_segment(norm))
        .collect();

    for start in starts {
        let norm_start = normalise_route(&start.key);

        // Reject starts that are entirely wildcards or empty after normalisation.
        if !has_literal_segment(&norm_start) {
            continue;
        }

        for (stop, norm_stop) in &norm_stops {
            // HTTP method must match (case-insensitive) unless one side is empty.
            let method_ok = start.method.is_empty()
                || stop.method.is_empty()
                || start.method.eq_ignore_ascii_case(&stop.method);

            if method_ok && routes_match_directed(&norm_start, norm_stop) {
                flows.push(make_flow(start, stop, "http_call", 0.8));
            }
        }
    }

    flows
}

/// Directional route match: start (caller) against stop (handler).
///
/// - A literal segment in the start must match the same literal or `{*}` in
///   the stop (the handler's wildcard catches concrete paths).
/// - A `{*}` in the start only matches `{*}` in the stop (a parameterised
///   caller won't reliably hit a specific literal handler route like `/me`).
fn routes_match_directed(start: &str, stop: &str) -> bool {
    let a: Vec<&str> = start.split('/').collect();
    let b: Vec<&str> = stop.split('/').collect();

    if a.len() != b.len() {
        return false;
    }

    a.iter().zip(b.iter()).all(|(s_seg, h_seg)| {
        if *s_seg == "{*}" {
            // Start wildcard only matches handler wildcard.
            *h_seg == "{*}"
        } else if *h_seg == "{*}" {
            // Handler wildcard matches any start literal.
            true
        } else {
            // Both literal — must match exactly.
            s_seg == h_seg
        }
    })
}

/// Returns true if the normalised route has at least one non-wildcard segment.
fn has_literal_segment(route: &str) -> bool {
    if route.is_empty() {
        return false;
    }
    route.split('/').any(|seg| !seg.is_empty() && seg != "{*}")
}

// ---------------------------------------------------------------------------
// Exact-key matching
// ---------------------------------------------------------------------------

/// Match by exact `key` equality.  When `require_framework` is true, both
/// sides must also share the same non-empty `framework` value.
fn match_exact(
    starts: &[&ConnectionPoint],
    stops: &[&ConnectionPoint],
    edge_type: &str,
    require_framework: bool,
) -> Vec<ResolvedFlow> {
    let mut flows = Vec::new();

    for start in starts {
        for stop in stops {
            if start.key != stop.key {
                continue;
            }

            if require_framework
                && !start.framework.is_empty()
                && !stop.framework.is_empty()
                && start.framework != stop.framework
            {
                continue;
            }

            flows.push(make_flow(start, stop, edge_type, 0.9));
        }
    }

    flows
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_flow(
    start: &&ConnectionPoint,
    stop: &&ConnectionPoint,
    edge_type: &str,
    confidence: f64,
) -> ResolvedFlow {
    // Sanity: callers are responsible for passing correctly-directed points,
    // but assert in debug builds to catch programming errors early.
    debug_assert_eq!(start.direction, FlowDirection::Start);
    debug_assert_eq!(stop.direction, FlowDirection::Stop);

    ResolvedFlow {
        start: (*start).clone(),
        stop: (*stop).clone(),
        confidence,
        edge_type: edge_type.to_string(),
    }
}
