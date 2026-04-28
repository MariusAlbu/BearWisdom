// =============================================================================
// ecosystem/phoenix_stubs.rs — Phoenix / Ecto / LiveView synthetic stubs
//
// Phoenix and Ecto expose their public API almost entirely through macros that
// become visible via `use SomeModule` / `import SomeModule` expansion:
//
//   defmodule MyController do
//     use Phoenix.Controller          # imports put_flash/3, render/3, redirect/2
//     def create(conn, _params) do
//       conn |> put_flash(:info, ...) |> redirect(to: "/")
//     end
//   end
//
// Tree-sitter sees `put_flash` as a bare call with no qualifier. Even when the
// Hex locator walks deps/phoenix/ and extracts Phoenix.Controller, the macro
// body (defmacro put_flash) isn't callable in the chain walker — macros are
// invisible-side-effect functions that generate code.
//
// This ecosystem synthesizes Phoenix/Ecto/LiveView macros as plain Function
// symbols so the Elixir resolver's by_name lookup step finds them. Fires for
// every Elixir project; synthetic symbols sit harmless when unused.
//
// Scope:
//   Phoenix.Controller       — put_flash, render, redirect, json, text, html, …
//   Phoenix.LiveView         — assign, push_event, push_patch, handle_event, …
//   Phoenix.LiveViewTest     — live, render_click, render_submit, assert_patch…
//   Phoenix.ConnTest         — build_conn, json_response, get, post, …
//   Plug.Conn                — put_status, put_resp_header, halt, send_resp, …
//   Ecto.Schema              — field, belongs_to, has_many, timestamps, …
//   Ecto.Changeset           — cast, validate_*, put_change, unique_constraint, …
//   Ecto.Query               — from, where, select, join, preload, order_by, …
//   Ecto.Repo                — all, one, get, insert, update, transaction, …
//
// Pattern: same as ecosystem/laravel_stubs.rs. Activation: Elixir language
// present.
// =============================================================================

use std::path::Path;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("phoenix-stubs");
const LEGACY_ECOSYSTEM_TAG: &str = "phoenix-stubs";
const LANGUAGES: &[&str] = &["elixir"];

// =============================================================================
// Per-module macro/function inventories
// =============================================================================

const PHOENIX_CONTROLLER: &[&str] = &[
    "action_fallback", "put_flash", "clear_flash", "get_flash",
    "put_layout", "put_new_layout", "put_root_layout", "put_view", "put_new_view",
    "redirect", "render", "json", "text", "html", "send_download",
    "current_path", "current_url", "controller_module", "action_name",
    "endpoint_module", "router_module", "scrub_params", "protect_from_forgery",
    "accepts", "status_message_from_template", "allow_jsonp", "layout",
    "view_module", "view_template",
];

const PHOENIX_LIVE_VIEW: &[&str] = &[
    "assign", "assign_new", "assigns_to_attributes", "attach_hook", "detach_hook",
    "push_event", "push_patch", "push_navigate", "push_redirect",
    "stream", "stream_insert", "stream_delete", "stream_delete_by_dom_id",
    "stream_configure",
    "allow_upload", "disallow_upload", "cancel_upload",
    "consume_uploaded_entries", "consume_uploaded_entry", "uploaded_entries",
    "handle_params", "handle_event", "handle_info", "handle_call", "handle_cast",
    "on_mount", "mount", "render", "terminate",
    "send_update", "send_update_after",
    "connected?", "transport_pid",
    "put_flash", "clear_flash",
    "live_flash", "live_title_tag", "live_file_input", "live_img_preview",
    "live_patch", "live_redirect",
    "put_private", "get_connect_params", "get_connect_info",
    "start_async", "assign_async",
];

const PHOENIX_LIVE_VIEW_TEST: &[&str] = &[
    "live", "live_isolated", "live_redirect", "live_patch",
    "render", "render_click", "render_submit", "render_change",
    "render_keyup", "render_keydown", "render_blur", "render_focus",
    "render_hook", "render_patch", "render_async", "render_upload",
    "assert_patch", "assert_patched", "assert_redirect", "assert_redirected",
    "assert_push_event", "assert_reply", "assert_async_reply",
    "follow_redirect", "follow_trigger_action",
    "element", "form", "file_input",
    "has_element?", "open_browser",
    "put_connect_params", "put_connect_info",
];

const PHOENIX_CONN_TEST: &[&str] = &[
    "conn", "build_conn", "bypass_through", "dispatch",
    "get", "post", "put", "delete", "patch", "options", "head",
    "recycle", "ensure_recycled",
    "json_response", "text_response", "html_response",
    "response", "response_content_type",
    "assert_error_sent", "assert_conn",
    "redirected_params", "redirected_to",
    "init_test_session", "put_session", "get_session",
];

const PLUG_CONN: &[&str] = &[
    "assign", "put_private", "get_private",
    "put_status", "put_resp_header", "delete_resp_header",
    "put_req_header", "delete_req_header",
    "get_req_header", "get_resp_header",
    "put_resp_cookie", "delete_resp_cookie", "put_req_cookie",
    "resp_cookies", "merge_assigns",
    "fetch_session", "fetch_cookies", "fetch_query_params",
    "halt", "halted?",
    "merge_resp_headers", "update_resp_header",
    "send_resp", "send_file", "send_chunked", "chunk", "resp",
    "read_body", "read_urlencoded_body",
    "configure_session", "get_session", "put_session", "delete_session", "clear_session",
    "register_before_send",
    "inform", "push", "informed?",
];

const ECTO_SCHEMA: &[&str] = &[
    "schema", "embedded_schema",
    "field", "belongs_to", "has_one", "has_many", "many_to_many",
    "embeds_one", "embeds_many",
    "timestamps", "primary_key",
    "association", "has_field?",
];

const ECTO_CHANGESET: &[&str] = &[
    "cast", "cast_assoc", "cast_embed",
    "change", "put_change", "put_assoc", "put_embed",
    "delete_change", "force_change",
    "fetch_change", "fetch_field", "get_change", "get_field",
    "fetch_change!", "fetch_field!",
    "apply_changes", "apply_action", "apply_action!",
    "validate_required", "validate_format", "validate_inclusion",
    "validate_exclusion", "validate_length", "validate_number",
    "validate_acceptance", "validate_confirmation", "validate_change",
    "validate_subset",
    "unique_constraint", "foreign_key_constraint", "assoc_constraint",
    "no_assoc_constraint", "check_constraint", "exclusion_constraint",
    "unsafe_validate_unique", "validations", "constraints",
    "prepare_changes", "merge", "traverse_errors", "add_error",
    "update_change", "get_embed", "get_assoc", "get_field",
];

const ECTO_QUERY: &[&str] = &[
    "from", "where", "or_where", "having", "or_having",
    "select", "select_merge",
    "join", "inner_join", "left_join", "right_join", "full_join", "cross_join",
    "preload", "order_by", "group_by", "distinct", "lock",
    "offset", "limit", "subquery", "fragment", "exclude", "update",
    "with_cte", "recursive_ctes",
    "union", "union_all", "intersect", "intersect_all", "except", "except_all",
    "windows", "reverse_order",
    "dynamic", "first", "last", "count",
    "field", "has_named_binding?", "literal", "selected_as",
];

const ECTO_REPO: &[&str] = &[
    "all", "one", "one!",
    "get", "get!", "get_by", "get_by!",
    "insert", "insert!", "insert_or_update", "insert_or_update!",
    "insert_all",
    "update", "update!", "update_all",
    "delete", "delete!", "delete_all",
    "preload", "exists?", "aggregate", "stream",
    "transaction", "rollback", "in_transaction?", "rollback!",
    "reload", "reload!",
    "checkout", "checked_out?", "put_dynamic_repo", "get_dynamic_repo",
    "config", "load", "prepare_query", "default_options",
    "to_sql",
];

// =============================================================================
// Synthesis
// =============================================================================

fn synth_module_file(module_qname: &str, macros: &[&str]) -> ParsedFile {
    let mut symbols = Vec::with_capacity(macros.len() + 1);

    // Namespace symbol for the module itself.
    symbols.push(ExtractedSymbol {
        name: module_qname.rsplit('.').next().unwrap_or(module_qname).to_string(),
        qualified_name: module_qname.to_string(),
        kind: SymbolKind::Namespace,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("defmodule {module_qname}")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });

    for m in macros {
        symbols.push(ExtractedSymbol {
            name: m.to_string(),
            qualified_name: format!("{module_qname}.{m}"),
            kind: SymbolKind::Function,
            visibility: Some(Visibility::Public),
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            signature: Some(format!("def {m}")),
            doc_comment: None,
            scope_path: Some(module_qname.to_string()),
            parent_index: Some(0),
        });
    }

    let path = format!(
        "ext:phoenix-stubs:{}.ex",
        module_qname.to_lowercase().replace('.', "/")
    );
    let n_syms = symbols.len();
    ParsedFile {
        path,
        language: "elixir".to_string(),
        content_hash: format!("phoenix-stubs-{module_qname}-{n_syms}"),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: vec![None; n_syms],
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: vec![false; n_syms],
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
    }
}

fn synthesize_all() -> Vec<ParsedFile> {
    vec![
        synth_module_file("Phoenix.Controller", PHOENIX_CONTROLLER),
        synth_module_file("Phoenix.LiveView", PHOENIX_LIVE_VIEW),
        synth_module_file("Phoenix.LiveViewTest", PHOENIX_LIVE_VIEW_TEST),
        synth_module_file("Phoenix.ConnTest", PHOENIX_CONN_TEST),
        synth_module_file("Plug.Conn", PLUG_CONN),
        synth_module_file("Ecto.Schema", ECTO_SCHEMA),
        synth_module_file("Ecto.Changeset", ECTO_CHANGESET),
        synth_module_file("Ecto.Query", ECTO_QUERY),
        synth_module_file("Ecto.Repo", ECTO_REPO),
    ]
}

// =============================================================================
// Synthetic dep root
// =============================================================================

fn synthetic_dep_root() -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "phoenix-stubs".to_string(),
        version: String::new(),
        root: std::path::PathBuf::from("ext:phoenix-stubs"),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

// =============================================================================
// Ecosystem impl
// =============================================================================

pub struct PhoenixStubsEcosystem;

impl Ecosystem for PhoenixStubsEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("elixir")
    }

    fn locate_roots(&self, _ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        vec![synthetic_dep_root()]
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn parse_metadata_only(&self, _dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        Some(synthesize_all())
    }
}

impl ExternalSourceLocator for PhoenixStubsEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        vec![synthetic_dep_root()]
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        Some(synthesize_all())
    }
}

#[cfg(test)]
#[path = "phoenix_stubs_tests.rs"]
mod tests;
