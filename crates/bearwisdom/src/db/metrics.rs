// =============================================================================
// db/metrics.rs  —  query-level performance metrics
//
// Opt-in metrics collection for Database operations.  When enabled, every
// query routed through the Database delegation methods records timing and
// count data.  Zero overhead when disabled (Option::None check).
//
// Designed for MCP audit trails and CLI diagnostics.
// =============================================================================

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

/// Aggregated per-label query statistics.
#[derive(Debug, Clone)]
pub struct QueryStats {
    pub count: u64,
    pub total_us: u64,
    pub max_us: u64,
}

/// Thread-safe query metrics collector.
///
/// Labels are short identifiers like `"symbol_info"`, `"search"`, etc.
/// Not SQL strings — those would be too verbose.  The query functions
/// assign labels at the call site.
pub struct QueryMetrics {
    /// Total queries across all labels.
    pub total_count: AtomicU64,
    /// Total query time in microseconds across all labels.
    pub total_duration_us: AtomicU64,
    /// Per-label breakdown.
    per_label: Mutex<HashMap<&'static str, LabelStats>>,
}

struct LabelStats {
    count: u64,
    total_us: u64,
    max_us: u64,
}

impl QueryMetrics {
    pub fn new() -> Self {
        Self {
            total_count: AtomicU64::new(0),
            total_duration_us: AtomicU64::new(0),
            per_label: Mutex::new(HashMap::new()),
        }
    }

    /// Record a query execution with the given label and duration.
    pub fn record(&self, label: &'static str, duration_us: u64) {
        self.total_count.fetch_add(1, Ordering::Relaxed);
        self.total_duration_us.fetch_add(duration_us, Ordering::Relaxed);

        if let Ok(mut map) = self.per_label.lock() {
            let entry = map.entry(label).or_insert(LabelStats {
                count: 0,
                total_us: 0,
                max_us: 0,
            });
            entry.count += 1;
            entry.total_us += duration_us;
            entry.max_us = entry.max_us.max(duration_us);
        }
    }

    /// Get a snapshot of all per-label statistics.
    pub fn snapshot(&self) -> Vec<(&'static str, QueryStats)> {
        let map = match self.per_label.lock() {
            Ok(m) => m,
            Err(_) => return vec![],
        };
        let mut result: Vec<_> = map
            .iter()
            .map(|(&label, stats)| {
                (
                    label,
                    QueryStats {
                        count: stats.count,
                        total_us: stats.total_us,
                        max_us: stats.max_us,
                    },
                )
            })
            .collect();
        result.sort_by(|a, b| b.1.total_us.cmp(&a.1.total_us));
        result
    }

    /// Reset all counters.
    pub fn reset(&self) {
        self.total_count.store(0, Ordering::Relaxed);
        self.total_duration_us.store(0, Ordering::Relaxed);
        if let Ok(mut map) = self.per_label.lock() {
            map.clear();
        }
    }
}

/// RAII timer that records elapsed time on drop.
///
/// Created by `Database::timed_query()`.  If metrics are disabled,
/// this is a no-op (the `metrics` field is None).
pub struct QueryTimer {
    label: &'static str,
    start: Instant,
    metrics: Option<std::sync::Arc<QueryMetrics>>,
}

impl QueryTimer {
    pub(crate) fn new(label: &'static str, metrics: Option<std::sync::Arc<QueryMetrics>>) -> Self {
        Self {
            label,
            start: Instant::now(),
            metrics,
        }
    }

    /// Manually stop the timer and record.  Also called on drop.
    pub fn finish(self) {
        // Drop impl does the actual recording.
    }
}

impl Drop for QueryTimer {
    fn drop(&mut self) {
        if let Some(ref metrics) = self.metrics {
            let elapsed_us = self.start.elapsed().as_micros() as u64;
            metrics.record(self.label, elapsed_us);
        }
    }
}
