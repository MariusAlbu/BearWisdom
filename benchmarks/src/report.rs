// =============================================================================
// report.rs  —  Generate markdown + JSON reports from scored results
// =============================================================================

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

use crate::runner::{Condition, RunResult};
use crate::scorer::{score_run, TaskScore};
use crate::task::BenchmarkTask;

// ---------------------------------------------------------------------------
// Report generation
// ---------------------------------------------------------------------------

pub fn generate_report(
    tasks: &[BenchmarkTask],
    results: &[RunResult],
    output_path: &Path,
) -> Result<()> {
    // Build a lookup map: task_id → &BenchmarkTask
    let task_map: HashMap<&str, &BenchmarkTask> = tasks
        .iter()
        .map(|t| (t.id.as_str(), t))
        .collect();

    // Score every result.
    let scores: Vec<TaskScore> = results
        .iter()
        .filter_map(|r| {
            task_map
                .get(r.task_id.as_str())
                .map(|t| score_run(t, r))
        })
        .collect();

    // Write JSON.
    let json_path = output_path.with_extension("json");
    let json_content = serde_json::to_string_pretty(&scores)
        .context("Failed to serialise scores")?;
    std::fs::write(&json_path, json_content)
        .with_context(|| format!("Failed to write {}", json_path.display()))?;

    // Write Markdown.
    let md = build_markdown(&scores);
    std::fs::write(output_path, &md)
        .with_context(|| format!("Failed to write {}", output_path.display()))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Markdown builder
// ---------------------------------------------------------------------------

fn build_markdown(scores: &[TaskScore]) -> String {
    let mut md = String::new();

    md.push_str("# BearWisdom Utility Benchmark Report\n\n");
    md.push_str(&format!(
        "_Generated: {}_\n\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    ));

    // ---- Summary table ----
    md.push_str("## Summary by Condition\n\n");
    md.push_str(
        "| Condition | Tasks | Avg Precision | Avg Recall | Avg F1 | Avg Efficiency | \
         Avg Composite | Avg Tool Calls | Avg Input Tokens | Avg Output Tokens |\n",
    );
    md.push_str(
        "|-----------|-------|---------------|------------|--------|----------------|---------------|----------------|------------------|-------------------|\n",
    );

    for condition in &[Condition::UseBearWisdom, Condition::NoBearWisdom] {
        let cond_str = condition.to_string();
        let subset: Vec<&TaskScore> = scores.iter().filter(|s| s.condition == cond_str).collect();
        if subset.is_empty() {
            continue;
        }
        let n = subset.len() as f64;
        let avg = |f: fn(&TaskScore) -> f64| subset.iter().map(|s| f(s)).sum::<f64>() / n;

        md.push_str(&format!(
            "| {cond_str} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.1} | {:.0} | {:.0} |\n",
            subset.len(),
            avg(|s| s.precision),
            avg(|s| s.recall),
            avg(|s| s.f1),
            avg(|s| s.efficiency),
            avg(|s| s.composite),
            avg(|s| s.tool_call_count as f64),
            avg(|s| s.input_tokens as f64),
            avg(|s| s.output_tokens as f64),
        ));
    }

    md.push('\n');

    // ---- Per-category breakdown ----
    md.push_str("## Per-Category Breakdown\n\n");

    let mut categories: Vec<String> = scores
        .iter()
        .map(|s| s.category.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    categories.sort();

    for category in &categories {
        md.push_str(&format!("### {category}\n\n"));
        md.push_str(
            "| Condition | Tasks | Avg Precision | Avg Recall | Avg F1 | Avg Tool Calls | Avg Composite |\n",
        );
        md.push_str(
            "|-----------|-------|---------------|------------|--------|----------------|---------------|\n",
        );

        for condition in &[Condition::UseBearWisdom, Condition::NoBearWisdom] {
            let cond_str = condition.to_string();
            let subset: Vec<&TaskScore> = scores
                .iter()
                .filter(|s| s.condition == cond_str && &s.category == category)
                .collect();
            if subset.is_empty() {
                continue;
            }
            let n = subset.len() as f64;
            let avg = |f: fn(&TaskScore) -> f64| subset.iter().map(|s| f(s)).sum::<f64>() / n;

            md.push_str(&format!(
                "| {cond_str} | {} | {:.3} | {:.3} | {:.3} | {:.1} | {:.3} |\n",
                subset.len(),
                avg(|s| s.precision),
                avg(|s| s.recall),
                avg(|s| s.f1),
                avg(|s| s.tool_call_count as f64),
                avg(|s| s.composite),
            ));
        }

        md.push('\n');
    }

    // ---- Per-task detail ----
    md.push_str("## Per-Task Results\n\n");

    // Group by task_id.
    let mut by_task: HashMap<&str, Vec<&TaskScore>> = HashMap::new();
    for score in scores {
        by_task.entry(score.task_id.as_str()).or_default().push(score);
    }

    let mut task_ids: Vec<&str> = by_task.keys().cloned().collect();
    task_ids.sort();

    for task_id in &task_ids {
        let task_scores = &by_task[task_id];
        let category = task_scores[0].category.as_str();

        md.push_str(&format!("### Task `{task_id}` ({category})\n\n"));
        md.push_str(
            "| Condition | Precision | Recall | F1 | Efficiency | Tool Calls | \
             Input Tok | Output Tok | Wall ms |\n",
        );
        md.push_str(
            "|-----------|-----------|--------|----|------------|------------|-----------|------------|----------|\n",
        );

        for score in task_scores.iter() {
            md.push_str(&format!(
                "| {} | {:.3} | {:.3} | {:.3} | {:.3} | {} | {} | {} | {} |\n",
                score.condition,
                score.precision,
                score.recall,
                score.f1,
                score.efficiency,
                score.tool_call_count,
                score.input_tokens,
                score.output_tokens,
                score.wall_time_ms,
            ));
        }

        md.push('\n');

        // Found / missed per condition.
        for score in task_scores.iter() {
            if !score.missed_items.is_empty() {
                md.push_str(&format!(
                    "**{} — Missed items:**\n\n",
                    score.condition
                ));
                for item in &score.missed_items {
                    md.push_str(&format!("- `{item}`\n"));
                }
                md.push('\n');
            }
        }
    }

    // ---- Token cost comparison ----
    md.push_str("## Token Cost Comparison\n\n");
    md.push_str("| Condition | Total Input Tokens | Total Output Tokens | Total Tokens |\n");
    md.push_str("|-----------|-------------------|---------------------|---------------|\n");

    for condition in &[Condition::UseBearWisdom, Condition::NoBearWisdom] {
        let cond_str = condition.to_string();
        let subset: Vec<&TaskScore> = scores.iter().filter(|s| s.condition == cond_str).collect();
        if subset.is_empty() {
            continue;
        }
        let total_in: u64 = subset.iter().map(|s| s.input_tokens).sum();
        let total_out: u64 = subset.iter().map(|s| s.output_tokens).sum();
        md.push_str(&format!(
            "| {cond_str} | {total_in} | {total_out} | {} |\n",
            total_in + total_out
        ));
    }

    md.push('\n');
    md
}
