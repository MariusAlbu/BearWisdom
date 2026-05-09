// =============================================================================
// scorer.rs  —  Score RunResults against BenchmarkTask ground truth
// =============================================================================

use serde::{Deserialize, Serialize};

use crate::runner::RunResult;
use crate::task::BenchmarkTask;

// ---------------------------------------------------------------------------
// Score types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskScore {
    pub task_id: String,
    pub category: String,
    pub condition: String,
    /// Project tag forwarded from the result (basename of project root, or
    /// override from --project-name).
    #[serde(default)]
    pub project: String,
    /// Run index within the (task × condition) group; 0-based.
    #[serde(default)]
    pub run_idx: u32,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    /// Efficiency penalises excessive tool calls: 1 / (1 + log2(tool_calls))
    pub efficiency: f64,
    /// Latency score: 30 / (30 + wall_seconds).  Faster = higher.
    pub latency_score: f64,
    /// Composite = 0.25*precision + 0.25*recall + 0.15*f1 + 0.20*efficiency + 0.15*latency
    pub composite: f64,
    pub tool_call_count: usize,
    /// LLM round-trip count (request/response cycles or assistant events).
    #[serde(default)]
    pub iterations: u32,
    /// Run outcome: "completed", "max_iterations", or "api_error".
    #[serde(default = "default_outcome_str")]
    pub outcome: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub wall_time_ms: u64,
    /// Ground truth items that appear in the answer.
    pub found_items: Vec<String>,
    /// Ground truth items that do NOT appear in the answer.
    pub missed_items: Vec<String>,
}

fn default_outcome_str() -> String {
    "completed".to_owned()
}

// ---------------------------------------------------------------------------
// Scoring
// ---------------------------------------------------------------------------

pub fn score_run(task: &BenchmarkTask, result: &RunResult) -> TaskScore {
    let answer_lower = result.answer.to_lowercase();

    let mut found_items: Vec<String> = Vec::new();
    let mut missed_items: Vec<String> = Vec::new();

    for item in &task.ground_truth.expected_items {
        // Check the simple name (last dot-segment) OR the full qualified name.
        let simple = item
            .rsplit('.')
            .next()
            .unwrap_or(item.as_str())
            .to_lowercase();
        let qualified_lower = item.to_lowercase();

        let hit = answer_lower.contains(&simple) || answer_lower.contains(&qualified_lower);

        if hit {
            found_items.push(item.clone());
        } else {
            missed_items.push(item.clone());
        }
    }

    let total_expected = task.ground_truth.expected_items.len();
    let total_found = found_items.len();

    // Count how many distinct ground-truth items appear in the answer (used for recall).
    // Precision denominator: we estimate "total mentions" as total_expected for simplicity
    // (we don't have a ground-truth negative set).  So precision = found / expected.
    let precision = if total_expected == 0 {
        1.0
    } else {
        total_found as f64 / total_expected as f64
    };

    let recall = if total_expected == 0 {
        1.0
    } else {
        total_found as f64 / total_expected as f64
    };

    let f1 = if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    };

    let tool_calls = result.tool_calls.len();
    let efficiency = if tool_calls == 0 {
        1.0
    } else {
        1.0 / (1.0 + (tool_calls as f64).max(1.0).log2())
    };

    let wall_seconds = result.wall_time_ms as f64 / 1000.0;
    let latency_score = 30.0 / (30.0 + wall_seconds);

    let composite = 0.25 * precision + 0.25 * recall + 0.15 * f1 + 0.20 * efficiency + 0.15 * latency_score;

    TaskScore {
        task_id: task.id.clone(),
        category: task.category.as_str().to_owned(),
        condition: result.condition.to_string(),
        project: result.project.clone(),
        run_idx: result.run_idx,
        precision,
        recall,
        f1,
        efficiency,
        latency_score,
        composite,
        tool_call_count: tool_calls,
        iterations: result.iterations,
        outcome: result.outcome.to_string(),
        input_tokens: result.input_tokens,
        output_tokens: result.output_tokens,
        wall_time_ms: result.wall_time_ms,
        found_items,
        missed_items,
    }
}
