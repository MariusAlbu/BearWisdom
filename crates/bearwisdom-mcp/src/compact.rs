//! Compact (token-optimized) output format for MCP tool responses.
//!
//! Instead of verbose JSON with repeated field names and file paths, this module
//! produces a pipe-delimited, section-based text format with a file registry that
//! defines paths once and references them by short IDs (F1, F2, …).
//!
//! Estimated savings: 40-60% token reduction vs JSON for typical responses.

use std::collections::HashMap;
use std::fmt::Write;

use bearwisdom::{
    ArchitectureOverview, BlastRadiusResult, CallHierarchyItem, FileSymbol, InvestigateResult,
    PackageStats, ResolutionBreakdown, SearchResult, SymbolDetail, SymbolSummary,
    WorkspaceGraphEdge, WorkspaceOverview,
};
use bearwisdom::query::completion::CompletionItem;
use bearwisdom::query::context::SmartContextResult;
use bearwisdom::query::dead_code::{DeadCodeReport, EntryPointKind, EntryPointsReport};
use bearwisdom::query::diagnostics::FileDiagnostics;
use bearwisdom::search::grep::GrepMatch;
use bearwisdom::types::ReferenceResult;

// ---------------------------------------------------------------------------
// Core formatter
// ---------------------------------------------------------------------------

struct CompactFormatter {
    files: Vec<String>,
    file_idx: HashMap<String, usize>,
    /// When `true`, `fref` returns the path verbatim instead of an `Fn` key,
    /// and `write_files` is a no-op. Used for single-result responses where
    /// the F-table indirection costs more tokens than it saves.
    inline_paths: bool,
}

impl CompactFormatter {
    fn new() -> Self {
        Self {
            files: Vec::new(),
            file_idx: HashMap::new(),
            inline_paths: false,
        }
    }

    /// Single-result mode: paths are inlined into result rows; the `#files`
    /// registry is suppressed.
    fn new_inline() -> Self {
        Self {
            files: Vec::new(),
            file_idx: HashMap::new(),
            inline_paths: true,
        }
    }

    /// Construct a formatter sized for `n` expected results. Inline mode is
    /// chosen automatically when `n == 1`.
    fn for_count(n: usize) -> Self {
        if n == 1 { Self::new_inline() } else { Self::new() }
    }

    /// Register a file path and return its compact ID (e.g. "F1") — or, in
    /// inline mode, the path itself.
    fn fref(&mut self, path: &str) -> String {
        if self.inline_paths {
            return path.to_string();
        }
        if let Some(&idx) = self.file_idx.get(path) {
            return format!("F{}", idx + 1);
        }
        let idx = self.files.len();
        self.files.push(path.to_string());
        self.file_idx.insert(path.to_string(), idx);
        format!("F{}", idx + 1)
    }

    /// Write the `#files` registry section into `out`. No-op in inline mode.
    fn write_files(&self, out: &mut String) {
        if self.inline_paths || self.files.is_empty() {
            return;
        }
        out.push_str("#files\n");
        for (i, path) in self.files.iter().enumerate() {
            let _ = writeln!(out, "F{}:{}", i + 1, path);
        }
    }
}

/// Format version header — prepended to all compact responses.
const FORMAT_HEADER: &str = "#format:compact-v1\n";

/// Compact not-found response with format header.
pub fn not_found() -> String {
    format!("{FORMAT_HEADER}#meta\nnot_found\n")
}

/// Start a compact response with the format header and a meta line.
fn start(meta: &str) -> String {
    let mut s = String::with_capacity(meta.len() + FORMAT_HEADER.len() + 32);
    s.push_str(FORMAT_HEADER);
    s.push_str("#meta\n");
    s.push_str(meta);
    s.push_str("\n\n");
    s
}

/// Append `|truncated:true` to a `count:N` style meta string when the
/// result set was capped by the request limit. Returns the meta unchanged
/// otherwise.
fn meta_with_truncation(meta: String, results_len: usize, requested_limit: usize) -> String {
    if requested_limit > 0 && results_len >= requested_limit {
        format!("{meta}|truncated:true")
    } else {
        meta
    }
}

// ---------------------------------------------------------------------------
// Helper: format a SymbolSummary line (used by multiple tools)
// ---------------------------------------------------------------------------

fn fmt_summary(f: &mut CompactFormatter, s: &SymbolSummary) -> String {
    let fr = f.fref(&s.file_path);
    format!("{}|{}|{}:{}", s.name, s.kind, fr, s.line)
}

fn fmt_call_item(f: &mut CompactFormatter, c: &CallHierarchyItem) -> String {
    let fr = f.fref(&c.file_path);
    format!("{}|{}|{}:{}", c.name, c.kind, fr, c.line)
}

// ---------------------------------------------------------------------------
// Per-tool public format functions
// ---------------------------------------------------------------------------

/// `bw_architecture_overview`
pub fn architecture(overview: &ArchitectureOverview) -> String {
    let mut f = CompactFormatter::new();
    let mut body = String::with_capacity(4096);

    // Languages
    body.push_str("#languages\n");
    for l in &overview.languages {
        let _ = writeln!(body, "{}|{}files|{}sym", l.language, l.file_count, l.symbol_count);
    }

    // Routes
    if !overview.routes.is_empty() {
        body.push_str("\n#routes\n");
        for r in &overview.routes {
            let fr = f.fref(&r.file_path);
            let line = r.line.map_or(String::new(), |l| format!(":{l}"));
            match &r.handler {
                Some(h) if !h.is_empty() => {
                    let _ = writeln!(body, "{} {}→{}|{}{}", r.http_method, r.route_template, h, fr, line);
                }
                _ => {
                    let _ = writeln!(body, "{} {}|{}{}", r.http_method, r.route_template, fr, line);
                }
            }
        }
    }

    // Hotspots
    if !overview.hotspots.is_empty() {
        body.push_str("\n#hotspots\n");
        for h in &overview.hotspots {
            let fr = f.fref(&h.file_path);
            let _ = writeln!(body, "{}|{}|{}|refs:{}", h.name, h.kind, fr, h.incoming_refs);
        }
    }

    // Entry points
    if !overview.entry_points.is_empty() {
        body.push_str("\n#entry_points\n");
        for e in &overview.entry_points {
            let _ = writeln!(body, "{}", fmt_summary(&mut f, e));
        }
    }

    // Assemble: header → meta → files → body
    let mut out = start(&format!(
        "files:{}|symbols:{}|edges:{}",
        overview.total_files, overview.total_symbols, overview.total_edges
    ));
    f.write_files(&mut out);
    if !f.files.is_empty() {
        out.push('\n');
    }
    out.push_str(&body);
    out
}

/// `bw_search`
pub fn search(results: &[SearchResult], requested_limit: usize) -> String {
    let mut f = CompactFormatter::for_count(results.len());
    let mut body = String::with_capacity(2048);

    body.push_str("#results\n");
    for r in results {
        let fr = f.fref(&r.file_path);
        let _ = writeln!(body, "{}|{}|{}:{}|{:.2}", r.name, r.kind, fr, r.start_line, r.score);
    }

    let meta = meta_with_truncation(format!("count:{}", results.len()), results.len(), requested_limit);
    let mut out = start(&meta);
    f.write_files(&mut out);
    if !f.files.is_empty() {
        out.push('\n');
    }
    out.push_str(&body);
    out
}

/// `bw_grep`
pub fn grep(results: &[GrepMatch], requested_limit: usize) -> String {
    let mut f = CompactFormatter::for_count(results.len());
    let mut body = String::with_capacity(4096);

    body.push_str("#matches\n");
    for m in results {
        let fr = f.fref(&m.file_path);
        let _ = writeln!(body, "{}:{}|{}", fr, m.line_number, m.line_content);
    }

    let meta = meta_with_truncation(format!("count:{}", results.len()), results.len(), requested_limit);
    let mut out = start(&meta);
    f.write_files(&mut out);
    if !f.files.is_empty() {
        out.push('\n');
    }
    out.push_str(&body);
    out
}

/// `bw_symbol_info`
pub fn symbol_info(results: &[SymbolDetail]) -> String {
    let mut f = CompactFormatter::for_count(results.len());
    let mut body = String::with_capacity(2048);

    body.push_str("#symbols\n");
    for s in results {
        let fr = f.fref(&s.file_path);
        let _ = write!(
            body,
            "{}|{}|{}:{}-{}|in:{}|out:{}",
            s.name, s.kind, fr, s.start_line, s.end_line, s.incoming_edge_count, s.outgoing_edge_count
        );
        if let Some(v) = &s.visibility {
            let _ = write!(body, "|{v}");
        }
        if let Some(sig) = &s.signature {
            let _ = write!(body, "\n  sig: {sig}");
        }
        if let Some(doc) = &s.doc_comment {
            // Collapse doc to single line
            let one_line: String = doc.lines().map(str::trim).collect::<Vec<_>>().join(" ");
            if !one_line.is_empty() {
                let _ = write!(body, "\n  doc: {one_line}");
            }
        }
        if !s.children.is_empty() {
            for c in &s.children {
                let cfr = f.fref(&c.file_path);
                let _ = write!(body, "\n  └ {}|{}|{}:{}", c.name, c.kind, cfr, c.line);
            }
        }
        body.push('\n');
    }

    let mut out = start(&format!("count:{}", results.len()));
    f.write_files(&mut out);
    if !f.files.is_empty() {
        out.push('\n');
    }
    out.push_str(&body);
    out
}

/// `bw_find_references`
pub fn references(results: &[ReferenceResult], requested_limit: usize) -> String {
    let mut f = CompactFormatter::for_count(results.len());
    let mut body = String::with_capacity(2048);

    body.push_str("#refs\n");
    for r in results {
        let fr = f.fref(&r.file_path);
        let _ = writeln!(
            body,
            "{}|{}|{}:{}|{}",
            r.referencing_symbol, r.referencing_kind, fr, r.line, r.edge_kind
        );
    }

    let meta = meta_with_truncation(format!("count:{}", results.len()), results.len(), requested_limit);
    let mut out = start(&meta);
    f.write_files(&mut out);
    if !f.files.is_empty() {
        out.push('\n');
    }
    out.push_str(&body);
    out
}

/// `bw_call_hierarchy`
pub fn call_hierarchy(results: &[CallHierarchyItem], requested_limit: usize) -> String {
    let mut f = CompactFormatter::for_count(results.len());
    let mut body = String::with_capacity(1024);

    body.push_str("#calls\n");
    for c in results {
        let _ = writeln!(body, "{}", fmt_call_item(&mut f, c));
    }

    let meta = meta_with_truncation(format!("count:{}", results.len()), results.len(), requested_limit);
    let mut out = start(&meta);
    f.write_files(&mut out);
    if !f.files.is_empty() {
        out.push('\n');
    }
    out.push_str(&body);
    out
}

/// `bw_file_symbols`
pub fn file_symbols(results: &[FileSymbol]) -> String {
    let mut body = String::with_capacity(2048);

    body.push_str("#symbols\n");
    for s in results {
        let _ = write!(body, "{}|{}|L{}", s.name, s.kind, s.line);
        if let Some(v) = &s.visibility {
            let _ = write!(body, "|{v}");
        }
        if let Some(sig) = &s.signature {
            let _ = write!(body, "\n  sig: {sig}");
        }
        body.push('\n');
    }

    // File symbols are all in one file — no file registry needed
    let mut out = start(&format!("count:{}", results.len()));
    out.push_str(&body);
    out
}

/// `bw_blast_radius`
pub fn blast_radius(result: &BlastRadiusResult) -> String {
    let mut f = CompactFormatter::new();
    let mut body = String::with_capacity(4096);

    // Center symbol
    body.push_str("#center\n");
    let _ = writeln!(body, "{}", fmt_summary(&mut f, &result.center));

    // Affected symbols
    body.push_str("\n#affected\n");
    for a in &result.affected {
        let fr = f.fref(&a.file_path);
        let _ = write!(body, "{}|{}|{}|d{}|{}", a.name, a.kind, fr, a.depth, a.edge_kind);
        if let Some(pkg) = &a.package {
            let _ = write!(body, "|pkg:{pkg}");
        }
        body.push('\n');
    }

    let trunc = if result.truncated { "|truncated" } else { "" };
    let mut out = start(&format!(
        "total:{}|max_depth:{}{}",
        result.total_affected, result.max_depth, trunc
    ));
    f.write_files(&mut out);
    if !f.files.is_empty() {
        out.push('\n');
    }
    out.push_str(&body);
    out
}

/// `bw_investigate`
pub fn investigate(result: &InvestigateResult) -> String {
    let mut f = CompactFormatter::new();
    let mut body = String::with_capacity(4096);

    // Symbol
    body.push_str("#symbol\n");
    let fr = f.fref(&result.symbol.file_path);
    let _ = write!(body, "{}|{}|{}:{}", result.symbol.name, result.symbol.kind, fr, result.symbol.line);
    if let Some(sig) = &result.symbol.signature {
        let _ = write!(body, "\n  sig: {sig}");
    }
    body.push('\n');

    // Callers
    if !result.callers.is_empty() {
        body.push_str("\n#callers\n");
        for c in &result.callers {
            let _ = writeln!(body, "{}", fmt_call_item(&mut f, c));
        }
    }

    // Callees
    if !result.callees.is_empty() {
        body.push_str("\n#callees\n");
        for c in &result.callees {
            let _ = writeln!(body, "{}", fmt_call_item(&mut f, c));
        }
    }

    // Blast radius
    if let Some(br) = &result.blast_radius {
        body.push_str("\n#blast_radius\n");
        let _ = writeln!(body, "total:{}", br.total_affected);
        for a in &br.affected {
            let afr = f.fref(&a.file_path);
            let _ = write!(body, "{}|{}|{}|d{}|{}", a.name, a.kind, afr, a.depth, a.edge_kind);
            if let Some(pkg) = &a.package {
                let _ = write!(body, "|pkg:{pkg}");
            }
            body.push('\n');
        }
    }

    let mut out = start(&format!(
        "callers:{}|callees:{}",
        result.callers.len(),
        result.callees.len()
    ));
    f.write_files(&mut out);
    if !f.files.is_empty() {
        out.push('\n');
    }
    out.push_str(&body);
    out
}

/// `bw_context`
pub fn smart_context(result: &SmartContextResult) -> String {
    let mut f = CompactFormatter::new();
    let mut body = String::with_capacity(4096);

    // Symbols
    body.push_str("#symbols\n");
    for s in &result.symbols {
        let fr = f.fref(&s.file_path);
        let _ = writeln!(body, "{}|{}|{}:{}|{:.2}|{}", s.name, s.kind, fr, s.line, s.score, s.reason);
    }

    // Key files
    if !result.files.is_empty() {
        body.push_str("\n#key_files\n");
        for path in &result.files {
            let fr = f.fref(path);
            let _ = writeln!(body, "{}", fr);
        }
    }

    // Concepts
    if !result.concepts.is_empty() {
        body.push_str("\n#concepts\n");
        for c in &result.concepts {
            let _ = writeln!(body, "{c}");
        }
    }

    let mut out = start(&format!(
        "task:{}|tokens:{}|symbols:{}|files:{}",
        result.task,
        result.token_estimate,
        result.symbols.len(),
        result.files.len()
    ));
    f.write_files(&mut out);
    if !f.files.is_empty() {
        out.push('\n');
    }
    out.push_str(&body);
    out
}

/// `bw_diagnostics`
pub fn diagnostics(result: &FileDiagnostics) -> String {
    let mut out = start(&format!(
        "file:{}|unresolved:{}|low_confidence:{}",
        result.file_path, result.unresolved_count, result.low_confidence_count
    ));

    out.push_str("#diagnostics\n");
    for d in &result.diagnostics {
        let kind = match d.kind {
            bearwisdom::query::diagnostics::DiagnosticKind::UnresolvedSymbol => "unresolved",
            bearwisdom::query::diagnostics::DiagnosticKind::LowConfidenceEdge => "low_conf",
        };
        let _ = write!(out, "L{}|{}|{}", d.line, kind, d.message);
        if let Some(conf) = d.confidence {
            let _ = write!(out, "|conf:{conf:.2}");
        }
        out.push('\n');
    }
    out
}

/// `bw_dead_code`
pub fn dead_code(report: &DeadCodeReport) -> String {
    let mut f = CompactFormatter::new();
    let mut body = String::with_capacity(4096);

    body.push_str("#dead_code\n");
    for d in &report.dead_candidates {
        let fr = f.fref(&d.file_path);
        let reason = match d.reason {
            bearwisdom::query::dead_code::DeadCodeReason::NoIncomingEdges => "no_refs",
            bearwisdom::query::dead_code::DeadCodeReason::OnlyLowConfidenceEdges => "low_conf",
        };
        let _ = write!(
            body,
            "{}|{}|{}:{}|{:.1}|{}",
            d.name, d.kind, fr, d.line, d.confidence, reason
        );
        if let Some(v) = &d.visibility {
            let _ = write!(body, "|{v}");
        }
        body.push('\n');
    }

    let mut out = start(&format!(
        "checked:{}|dead:{}|entry_excluded:{}|test_excluded:{}|resolution:{:.1}%|trust:{}",
        report.total_symbols_checked,
        report.dead_candidates.len(),
        report.entry_points_excluded,
        report.test_symbols_excluded,
        report.resolution_health.resolution_rate,
        report.resolution_health.trust_tier.as_str(),
    ));
    f.write_files(&mut out);
    if !f.files.is_empty() {
        out.push('\n');
    }
    out.push_str(&body);
    out
}

/// `bw_entry_points`
pub fn entry_points(report: &EntryPointsReport) -> String {
    let mut f = CompactFormatter::new();
    let mut body = String::with_capacity(2048);

    body.push_str("#entry_points\n");
    for e in &report.entry_points {
        let fr = f.fref(&e.file_path);
        let ek = match e.entry_kind {
            EntryPointKind::Main => "main",
            EntryPointKind::RouteHandler => "route_handler",
            EntryPointKind::EventHandler => "event_handler",
            EntryPointKind::TestFunction => "test_function",
            EntryPointKind::ExportedApi => "exported_api",
            EntryPointKind::LifecycleHook => "lifecycle_hook",
            EntryPointKind::DiRegistered => "di_registered",
        };
        let _ = writeln!(body, "{}|{}|{}:{}|{}", e.name, e.kind, fr, e.line, ek);
    }

    let mut out = start(&format!("total:{}", report.total));
    f.write_files(&mut out);
    if !f.files.is_empty() {
        out.push('\n');
    }
    out.push_str(&body);
    out
}

/// `bw_complete`
pub fn completions(results: &[CompletionItem]) -> String {
    let mut f = CompactFormatter::new();
    let mut body = String::with_capacity(1024);

    body.push_str("#completions\n");
    for c in results {
        let fr = f.fref(&c.file_path);
        let _ = write!(body, "{}|{}|{}|d{}", c.name, c.kind, fr, c.scope_distance);
        if let Some(sig) = &c.signature {
            let _ = write!(body, "|sig:{sig}");
        }
        body.push('\n');
    }

    let mut out = start(&format!("count:{}", results.len()));
    f.write_files(&mut out);
    if !f.files.is_empty() {
        out.push('\n');
    }
    out.push_str(&body);
    out
}

/// `bw_packages`
pub fn packages(results: &[PackageStats]) -> String {
    let mut out = start(&format!("count:{}", results.len()));
    out.push_str("#packages\n");
    for p in results {
        let kind = p.kind.as_deref().unwrap_or("-");
        let resolution = match p.resolved_pct {
            Some(pct) => format!("|resolved:{:.1}%({}/{})", pct * 100.0, p.resolved_refs, p.resolved_refs + p.unresolved_refs),
            None => String::new(),
        };
        let _ = writeln!(
            out,
            "{}|{}|{}|{}files|{}sym|{}edges{}",
            p.name, p.path, kind, p.file_count, p.symbol_count, p.edge_count, resolution
        );
    }
    out
}

/// `bw_workspace_overview`
pub fn workspace(overview: &WorkspaceOverview) -> String {
    let mut f = CompactFormatter::new();
    let mut body = String::with_capacity(2048);

    body.push_str("#packages\n");
    for p in &overview.packages {
        let kind = p.kind.as_deref().unwrap_or("-");
        let _ = writeln!(
            body,
            "{}|{}|{}|{}files|{}sym|{}edges",
            p.name, p.path, kind, p.file_count, p.symbol_count, p.edge_count
        );
    }

    if !overview.shared_hotspots.is_empty() {
        body.push_str("\n#shared_hotspots\n");
        for h in &overview.shared_hotspots {
            let fr = f.fref(&h.file_path);
            let _ = writeln!(body, "{}|{}|{}|refs:{}", h.name, h.kind, fr, h.incoming_refs);
        }
    }

    let mut out = start(&format!(
        "packages:{}|cross_pkg_edges:{}",
        overview.packages.len(),
        overview.total_cross_package_edges
    ));
    f.write_files(&mut out);
    if !f.files.is_empty() {
        out.push('\n');
    }
    out.push_str(&body);
    out
}

/// `bw_workspace_graph`
///
/// Format: one line per (src, tgt) pair. `code` and `flow` use `kind:count`
/// tuples separated by `+`; a trailing `declared` flag surfaces manifest
/// intent. Lines sort by total edges descending (matches the query order).
pub fn workspace_graph(edges: &[WorkspaceGraphEdge]) -> String {
    let mut out = start(&format!("edges:{}", edges.len()));
    for e in edges {
        let code_parts = if e.code_by_kind.is_empty() {
            "-".to_string()
        } else {
            e.code_by_kind
                .iter()
                .map(|(k, n)| format!("{k}:{n}"))
                .collect::<Vec<_>>()
                .join("+")
        };
        let flow_parts = if e.flow_by_kind.is_empty() {
            "-".to_string()
        } else {
            e.flow_by_kind
                .iter()
                .map(|(k, n)| format!("{k}:{n}"))
                .collect::<Vec<_>>()
                .join("+")
        };
        let declared = if e.declared_dep { "declared" } else { "-" };
        let _ = writeln!(
            out,
            "{}->{}|code:{}|flow:{}|{}|total:{}",
            e.source_package, e.target_package, code_parts, flow_parts, declared, e.total_edges
        );
    }
    out
}

/// `bw_quality_check` — resolution-rate dashboard for the indexed project.
///
/// Surface: headline rate, per-language file counts, per-(lang,kind)
/// unresolved breakdown, top unresolved targets. Drives the "which
/// extractor / resolver is leaking?" question without leaving MCP.
pub fn quality_check(rb: &ResolutionBreakdown) -> String {
    let mut body = String::with_capacity(2048);

    if !rb.languages.is_empty() {
        body.push_str("#languages\n");
        for (lang, files) in &rb.languages {
            let _ = writeln!(body, "{lang}|{files}files");
        }
    }
    if !rb.unresolved_by_lang_kind.is_empty() {
        body.push_str("\n#unresolved_by_lang_kind\n");
        for (key, count) in &rb.unresolved_by_lang_kind {
            let _ = writeln!(body, "{key}|{count}");
        }
    }
    if !rb.unresolved_by_origin_language.is_empty() {
        body.push_str("\n#unresolved_by_origin_lang\n");
        for (lang, count) in &rb.unresolved_by_origin_language {
            let key = if lang.is_empty() { "<host>" } else { lang.as_str() };
            let _ = writeln!(body, "{key}|{count}");
        }
    }
    if !rb.unresolved_by_package.is_empty() {
        body.push_str("\n#unresolved_by_package\n");
        for (pkg, count) in &rb.unresolved_by_package {
            let key = if pkg.is_empty() { "<no_pkg>" } else { pkg.as_str() };
            let _ = writeln!(body, "{key}|{count}");
        }
    }
    if !rb.resolved_by_strategy.is_empty() {
        body.push_str("\n#resolved_by_strategy\n");
        for (strategy, count) in &rb.resolved_by_strategy {
            let key = if strategy.is_empty() { "<unknown>" } else { strategy.as_str() };
            let _ = writeln!(body, "{key}|{count}");
        }
    }
    if !rb.top_unresolved_targets.is_empty() {
        body.push_str("\n#top_unresolved\n");
        for t in &rb.top_unresolved_targets {
            let _ = writeln!(body, "{}|{}|{}|{}", t.target_name, t.language, t.kind, t.count);
        }
    }

    let mut out = start(&format!(
        "resolution_rate:{:.2}%|internal_edges:{}|internal_unresolved:{}|low_conf:{}|low_conf_threshold:{:.2}|code_chunks:{}",
        rb.internal_resolution_rate,
        rb.internal_edges,
        rb.internal_unresolved,
        rb.low_confidence_edges,
        rb.low_confidence_threshold,
        rb.code_chunks,
    ));
    out.push_str(&body);
    out
}

#[cfg(test)]
#[path = "compact_tests.rs"]
mod tests;
