// =============================================================================
// report.rs  —  Generate benchmark.md from scored results
//
// The output is a structured comparison of `UseBearWisdom`, `UseBearWisdomCli`,
// and `NoBearWisdom` across all (project × task × run) tuples loaded from disk.
//
// Each (project, condition, task_id) group is reduced to a median and IQR over
// its repeated runs; per-language and per-category sections aggregate further.
// =============================================================================

use anyhow::{Context, Result};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::runner::{Condition, RunResult};
use crate::scorer::{score_run, TaskScore};
use crate::task::BenchmarkTask;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn generate_report(
    tasks: &[BenchmarkTask],
    results: &[RunResult],
    output_path: &Path,
) -> Result<()> {
    let task_map: HashMap<&str, &BenchmarkTask> =
        tasks.iter().map(|t| (t.id.as_str(), t)).collect();

    let scores: Vec<TaskScore> = results
        .iter()
        .filter_map(|r| task_map.get(r.task_id.as_str()).map(|t| score_run(t, r)))
        .collect();

    // Persist raw scores alongside the markdown.
    let json_path = output_path.with_extension("json");
    let json_content =
        serde_json::to_string_pretty(&scores).context("Failed to serialise scores")?;
    std::fs::write(&json_path, json_content)
        .with_context(|| format!("Failed to write {}", json_path.display()))?;

    let md = build_benchmark_md(&scores, results);
    std::fs::write(output_path, &md)
        .with_context(|| format!("Failed to write {}", output_path.display()))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Aggregation primitives
// ---------------------------------------------------------------------------

/// Median + IQR for a slice of numeric values.
fn quantiles(mut xs: Vec<f64>) -> (f64, f64, f64) {
    if xs.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pick = |q: f64| -> f64 {
        let idx = ((xs.len() as f64 - 1.0) * q).round() as usize;
        xs[idx.min(xs.len() - 1)]
    };
    (pick(0.25), pick(0.5), pick(0.75))
}

#[derive(Debug, Default, Clone)]
struct ConditionStats {
    runs: usize,
    completed: usize,
    max_iterations: usize,
    api_errors: usize,
    f1: Vec<f64>,
    iterations: Vec<f64>,
    tool_calls: Vec<f64>,
    wall_ms: Vec<f64>,
    input_tokens: Vec<f64>,
    output_tokens: Vec<f64>,
    total_tokens: Vec<f64>,
}

impl ConditionStats {
    fn push(&mut self, s: &TaskScore) {
        self.runs += 1;
        match s.outcome.as_str() {
            "completed" => self.completed += 1,
            "max_iterations" => self.max_iterations += 1,
            "api_error" => self.api_errors += 1,
            _ => {}
        }
        self.f1.push(s.f1);
        self.iterations.push(s.iterations as f64);
        self.tool_calls.push(s.tool_call_count as f64);
        self.wall_ms.push(s.wall_time_ms as f64);
        self.input_tokens.push(s.input_tokens as f64);
        self.output_tokens.push(s.output_tokens as f64);
        self.total_tokens
            .push((s.input_tokens + s.output_tokens) as f64);
    }
}

/// `(project, condition) → ConditionStats` keyed for stable ordering.
type ProjectStats = BTreeMap<(String, String), ConditionStats>;

fn collect_project_stats(scores: &[TaskScore]) -> ProjectStats {
    let mut map: ProjectStats = BTreeMap::new();
    for s in scores {
        let project = if s.project.is_empty() {
            "unknown".to_owned()
        } else {
            s.project.clone()
        };
        let entry = map.entry((project, s.condition.clone())).or_default();
        entry.push(s);
    }
    map
}

/// Map a project tag to a display language. Falls back to "Other" when the
/// tag has no known language prefix (e.g. self-host repo names).
fn language_for_project(project: &str) -> &'static str {
    let lower = project.to_ascii_lowercase();
    if lower.starts_with("dotnet-") || lower.starts_with("csharp-") {
        "C#"
    } else if lower.starts_with("java-") {
        "Java"
    } else if lower.starts_with("kotlin-") {
        "Kotlin"
    } else if lower.starts_with("python-") {
        "Python"
    } else if lower.starts_with("rust-") {
        "Rust"
    } else if lower.starts_with("ts-") || lower.starts_with("typescript-") {
        "TypeScript"
    } else if lower.starts_with("vue-") {
        "Vue"
    } else if lower.starts_with("javascript-") {
        "JavaScript"
    } else if lower == "bearwisdom" {
        "Rust (self-host)"
    } else {
        "Other"
    }
}

// ---------------------------------------------------------------------------
// Markdown builder
// ---------------------------------------------------------------------------

fn build_benchmark_md(scores: &[TaskScore], results: &[RunResult]) -> String {
    let mut md = String::new();
    md.push_str("# BearWisdom MCP vs native LLM tooling — benchmark\n\n");
    md.push_str(&format!(
        "_Generated: {}_\n\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    ));

    let stats = collect_project_stats(scores);
    let projects: Vec<String> = stats
        .keys()
        .map(|(p, _)| p.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    let model = results
        .first()
        .map(|r| r.model.clone())
        .unwrap_or_else(|| "unknown".to_owned());
    let total_runs = scores.len();
    let total_projects = projects.len();

    // ---------------- Headline ----------------
    md.push_str("## Headline\n\n");
    write_headline(&mut md, &stats, &projects);

    // ---------------- Methodology ----------------
    md.push_str("## Methodology\n\n");
    md.push_str(&format!(
        "- **Model:** `{model}`\n\
         - **Total runs:** {total_runs} across {total_projects} project(s)\n\
         - **Conditions:** `UseBearWisdom` (BW MCP tools, in-process), \
           `UseBearWisdomCli` (Claude CLI + bw-mcp over stdio), \
           `NoBearWisdom` (native Read/Grep/Glob/list_dir only).\n\
         - **Repeats:** see Reproducibility section for per-(project, condition) run counts.\n\
         - **Aggregation:** each (project, condition, task_id) group reduced to median + IQR \
           over its repeated runs, then aggregated for the per-language and per-category tables.\n\
         - **F1:** answer-text substring match against `expected_items` (file paths + symbol names) \
           in the task's ground truth.\n\
         - **Outcome:** `completed` = end_turn; `max_iterations` = ran out of tool-call iterations; \
           `api_error` = transport or upstream failure.\n\n"
    ));

    // ---------------- Per-language results ----------------
    md.push_str("## Per-language results\n\n");

    // Group projects by language for stable section order.
    let mut by_lang: BTreeMap<&'static str, Vec<&str>> = BTreeMap::new();
    for p in &projects {
        by_lang
            .entry(language_for_project(p))
            .or_default()
            .push(p.as_str());
    }

    for (lang, lang_projects) in &by_lang {
        md.push_str(&format!("### {lang}\n\n"));
        for project in lang_projects {
            md.push_str(&format!("**Project:** `{project}`\n\n"));
            write_project_table(&mut md, &stats, project);
        }
    }

    // ---------------- Per-task-category breakdown ----------------
    md.push_str("## Per-task-category breakdown\n\n");
    write_category_breakdown(&mut md, scores);

    // ---------------- Failure-mode analysis ----------------
    md.push_str("## Failure-mode analysis\n\n");
    write_failure_modes(&mut md, &stats, &projects);

    // ---------------- Reproducibility ----------------
    md.push_str("## Reproducibility\n\n");
    md.push_str(&format!(
        "- Run counts per (project, condition):\n\n"
    ));
    md.push_str("| Project | Condition | Runs | Completed | MaxIter | ApiErr |\n");
    md.push_str("|---|---|---:|---:|---:|---:|\n");
    for ((project, cond), st) in &stats {
        md.push_str(&format!(
            "| {project} | {cond} | {} | {} | {} | {} |\n",
            st.runs, st.completed, st.max_iterations, st.api_errors
        ));
    }
    md.push_str("\n- Reproduce with:\n\n```bash\n");
    md.push_str("bw-bench full \\\n");
    md.push_str("    --projects <p1>,<p2>,... \\\n");
    md.push_str(&format!("    --model {model} \\\n"));
    md.push_str("    --output bench-results/ \\\n");
    md.push_str("    --count 10 \\\n");
    md.push_str("    --repeat 3\n```\n");

    md
}

// ---------------------------------------------------------------------------
// Section writers
// ---------------------------------------------------------------------------

fn write_headline(
    md: &mut String,
    stats: &ProjectStats,
    projects: &[String],
) {
    // Aggregate (project median) → cross-project mean per condition.
    // Numbers: median total tokens, median iterations, median wall_ms, mean F1, completion rate.
    let mut agg: HashMap<&str, ConditionAgg> = HashMap::new();
    for cond in Condition::all() {
        let cond_str = cond.to_string();
        agg.insert(box_leak(cond_str.clone()), ConditionAgg::default());
    }

    for project in projects {
        for cond in Condition::all() {
            let cond_str = cond.to_string();
            if let Some(st) = stats.get(&(project.clone(), cond_str.clone())) {
                let entry = agg.get_mut(cond_str.as_str()).unwrap();
                let (_, m_tok, _) = quantiles(st.total_tokens.clone());
                let (_, m_it, _) = quantiles(st.iterations.clone());
                let (_, m_wall, _) = quantiles(st.wall_ms.clone());
                let mean_f1: f64 =
                    st.f1.iter().sum::<f64>() / (st.f1.len().max(1) as f64);
                let comp_rate = st.completed as f64 / (st.runs.max(1) as f64);
                entry.tokens.push(m_tok);
                entry.iterations.push(m_it);
                entry.wall_ms.push(m_wall);
                entry.f1.push(mean_f1);
                entry.completion.push(comp_rate);
            }
        }
    }

    fn fmt_pct_delta(bw: f64, native: f64) -> String {
        if native <= 0.0 {
            return "—".to_owned();
        }
        let delta = (native - bw) / native * 100.0;
        format!("{delta:+.1}%")
    }

    let bw = agg.get("use_bearwisdom").cloned().unwrap_or_default();
    let bw_cli = agg.get("use_bearwisdom_cli").cloned().unwrap_or_default();
    let native = agg.get("no_bearwisdom").cloned().unwrap_or_default();

    md.push_str("Cross-project medians, per condition:\n\n");
    md.push_str("| Condition | Median total tokens | Median LLM iterations | Median wall (ms) | Mean F1 | Completion rate |\n");
    md.push_str("|---|---:|---:|---:|---:|---:|\n");
    md.push_str(&format!(
        "| UseBearWisdom (MCP) | {:.0} | {:.1} | {:.0} | {:.3} | {:.0}% |\n",
        median_or_zero(&bw.tokens),
        median_or_zero(&bw.iterations),
        median_or_zero(&bw.wall_ms),
        mean_or_zero(&bw.f1),
        mean_or_zero(&bw.completion) * 100.0
    ));
    md.push_str(&format!(
        "| UseBearWisdomCli (CLI+MCP) | {:.0} | {:.1} | {:.0} | {:.3} | {:.0}% |\n",
        median_or_zero(&bw_cli.tokens),
        median_or_zero(&bw_cli.iterations),
        median_or_zero(&bw_cli.wall_ms),
        mean_or_zero(&bw_cli.f1),
        mean_or_zero(&bw_cli.completion) * 100.0
    ));
    md.push_str(&format!(
        "| NoBearWisdom (native) | {:.0} | {:.1} | {:.0} | {:.3} | {:.0}% |\n\n",
        median_or_zero(&native.tokens),
        median_or_zero(&native.iterations),
        median_or_zero(&native.wall_ms),
        mean_or_zero(&native.f1),
        mean_or_zero(&native.completion) * 100.0
    ));

    md.push_str("**Deltas (BW vs native):**\n\n");
    md.push_str(&format!(
        "- Token reduction: {}\n",
        fmt_pct_delta(median_or_zero(&bw.tokens), median_or_zero(&native.tokens))
    ));
    md.push_str(&format!(
        "- Iteration reduction: {}\n",
        fmt_pct_delta(
            median_or_zero(&bw.iterations),
            median_or_zero(&native.iterations)
        )
    ));
    md.push_str(&format!(
        "- Wall-time reduction: {}\n",
        fmt_pct_delta(median_or_zero(&bw.wall_ms), median_or_zero(&native.wall_ms))
    ));
    md.push_str(&format!(
        "- F1 absolute delta: {:+.3}\n",
        mean_or_zero(&bw.f1) - mean_or_zero(&native.f1)
    ));
    md.push_str(&format!(
        "- Completion rate delta: {:+.0} pp\n\n",
        (mean_or_zero(&bw.completion) - mean_or_zero(&native.completion)) * 100.0
    ));
}

#[derive(Default, Clone)]
struct ConditionAgg {
    tokens: Vec<f64>,
    iterations: Vec<f64>,
    wall_ms: Vec<f64>,
    f1: Vec<f64>,
    completion: Vec<f64>,
}

fn median_or_zero(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    quantiles(v.to_vec()).1
}

fn mean_or_zero(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().sum::<f64>() / (v.len() as f64)
}

fn write_project_table(md: &mut String, stats: &ProjectStats, project: &str) {
    md.push_str("| Condition | Runs | Tokens (med · IQR) | Iter (med) | Tool calls (med) | Wall ms (med · IQR) | Mean F1 | Completed |\n");
    md.push_str("|---|---:|---|---:|---:|---|---:|---:|\n");
    for cond in Condition::all() {
        let cond_str = cond.to_string();
        if let Some(st) = stats.get(&(project.to_owned(), cond_str.clone())) {
            let (q1_t, m_t, q3_t) = quantiles(st.total_tokens.clone());
            let (_, m_it, _) = quantiles(st.iterations.clone());
            let (_, m_tc, _) = quantiles(st.tool_calls.clone());
            let (q1_w, m_w, q3_w) = quantiles(st.wall_ms.clone());
            let mean_f1: f64 = st.f1.iter().sum::<f64>() / (st.f1.len().max(1) as f64);
            let comp_pct = st.completed as f64 / (st.runs.max(1) as f64) * 100.0;
            md.push_str(&format!(
                "| {cond_str} | {} | {:.0} · [{:.0}–{:.0}] | {:.1} | {:.1} | {:.0} · [{:.0}–{:.0}] | {:.3} | {:.0}% |\n",
                st.runs, m_t, q1_t, q3_t, m_it, m_tc, m_w, q1_w, q3_w, mean_f1, comp_pct
            ));
        }
    }
    md.push('\n');
}

fn write_category_breakdown(md: &mut String, scores: &[TaskScore]) {
    let mut categories: Vec<String> = scores
        .iter()
        .map(|s| s.category.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    categories.sort();

    for category in &categories {
        md.push_str(&format!("### {category}\n\n"));
        md.push_str("| Condition | Runs | Mean F1 | Median tokens | Median iter | Median tool calls | Completed |\n");
        md.push_str("|---|---:|---:|---:|---:|---:|---:|\n");
        for cond in Condition::all() {
            let cond_str = cond.to_string();
            let subset: Vec<&TaskScore> = scores
                .iter()
                .filter(|s| s.condition == cond_str && &s.category == category)
                .collect();
            if subset.is_empty() {
                continue;
            }
            let n = subset.len();
            let mean_f1 =
                subset.iter().map(|s| s.f1).sum::<f64>() / (n as f64);
            let med_tokens = median_or_zero(
                &subset
                    .iter()
                    .map(|s| (s.input_tokens + s.output_tokens) as f64)
                    .collect::<Vec<_>>(),
            );
            let med_iter = median_or_zero(
                &subset.iter().map(|s| s.iterations as f64).collect::<Vec<_>>(),
            );
            let med_tc = median_or_zero(
                &subset
                    .iter()
                    .map(|s| s.tool_call_count as f64)
                    .collect::<Vec<_>>(),
            );
            let comp = subset.iter().filter(|s| s.outcome == "completed").count();
            let comp_pct = comp as f64 / (n as f64) * 100.0;
            md.push_str(&format!(
                "| {cond_str} | {n} | {mean_f1:.3} | {med_tokens:.0} | {med_iter:.1} | {med_tc:.1} | {comp_pct:.0}% |\n"
            ));
        }
        md.push('\n');
    }
}

fn write_failure_modes(md: &mut String, stats: &ProjectStats, projects: &[String]) {
    md.push_str("Per-project completion (`completed` / `max_iterations` / `api_error`) by condition:\n\n");
    md.push_str("| Project | Condition | Completed | MaxIter | ApiErr | Total |\n");
    md.push_str("|---|---|---:|---:|---:|---:|\n");
    for project in projects {
        for cond in Condition::all() {
            let cond_str = cond.to_string();
            if let Some(st) = stats.get(&(project.clone(), cond_str.clone())) {
                md.push_str(&format!(
                    "| {project} | {cond_str} | {} | {} | {} | {} |\n",
                    st.completed, st.max_iterations, st.api_errors, st.runs
                ));
            }
        }
    }
    md.push('\n');
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Leak a `String` so it can serve as a `&'static str` lookup key.
/// Used only for the small fixed condition set; bounded growth.
fn box_leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}
