// =============================================================================
// runner.rs  —  Execute benchmark tasks via the Anthropic API
//
// Two conditions:
//   • NoBearWisdom: Claude has Read / Grep / Glob / ListDir only
//   • UseBearWisdom:    Claude additionally has bw_* tools backed by the
//                        bearwisdom library called directly in-process.
// =============================================================================

use anyhow::{bail, Context, Result};
use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tracing::{debug, info, warn};

use bearwisdom::{
    query::{
        architecture, blast_radius as blast_radius_mod, call_hierarchy,
        references, search as search_mod, symbol_info,
    },
    resolve_db_path,
};

use crate::task::{BenchmarkTask, TaskSet};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    UseBearWisdom,
    UseBearWisdomCli,
    NoBearWisdom,
}

impl Condition {
    /// All known conditions in display order.
    pub fn all() -> &'static [Condition] {
        &[Condition::UseBearWisdom, Condition::UseBearWisdomCli, Condition::NoBearWisdom]
    }
}

impl std::fmt::Display for Condition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UseBearWisdom => write!(f, "use_bearwisdom"),
            Self::UseBearWisdomCli => write!(f, "use_bearwisdom_cli"),
            Self::NoBearWisdom => write!(f, "no_bearwisdom"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool_name: String,
    pub input: Value,
    pub output_len: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub task_id: String,
    pub condition: Condition,
    pub model: String,
    pub answer: String,
    pub tool_calls: Vec<ToolCall>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub wall_time_ms: u64,
    pub completed_at: chrono::DateTime<chrono::Utc>,
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

pub struct Runner {
    api_key: String,
    model: String,
    client: reqwest::Client,
    project_root: PathBuf,
    pool: bearwisdom::DbPool,
}

impl Runner {
    pub fn new(api_key: String, model: String, project_root: PathBuf) -> Result<Self> {
        let db_path = resolve_db_path(&project_root)?;
        let pool = bearwisdom::DbPool::new(&db_path, 4)?;

        Ok(Self {
            api_key,
            model,
            client: reqwest::Client::new(),
            project_root,
            pool,
        })
    }

    /// Run all tasks in `task_set` for the requested conditions.
    pub async fn run_all(
        &self,
        task_set: &TaskSet,
        conditions: &[Condition],
        output_dir: &Path,
    ) -> Result<Vec<RunResult>> {
        std::fs::create_dir_all(output_dir)?;
        let mut results = Vec::new();

        for condition in conditions {
            info!("Running condition: {condition}");
            for task in &task_set.tasks {
                info!("  Task {} [{}]", task.id, task.category.as_str());
                match self.run_task(task, condition).await {
                    Ok(result) => {
                        // Persist individual result immediately.
                        let filename = format!("{}-{}.json", task.id, condition);
                        let path = output_dir.join(&filename);
                        let json = serde_json::to_string_pretty(&result)?;
                        std::fs::write(&path, json)?;
                        results.push(result);
                    }
                    Err(e) => {
                        warn!("Task {} failed: {e:#}", task.id);
                    }
                }
            }
        }

        Ok(results)
    }

    async fn run_task(&self, task: &BenchmarkTask, condition: &Condition) -> Result<RunResult> {
        let tools = self.build_tools(condition);
        let system = self.system_prompt(condition, task);

        let start = Instant::now();
        let mut messages: Vec<Value> = vec![
            json!({ "role": "user", "content": task.question }),
        ];

        let mut all_tool_calls: Vec<ToolCall> = Vec::new();
        let mut input_tokens: u64 = 0;
        let mut output_tokens: u64 = 0;
        let mut final_answer = String::new();
        let mut iterations = 0;
        const MAX_ITERATIONS: usize = 30;

        loop {
            if iterations >= MAX_ITERATIONS {
                bail!("Exceeded {MAX_ITERATIONS} tool call iterations for task {}", task.id);
            }
            iterations += 1;

            let body = json!({
                "model": self.model,
                "max_tokens": 4096,
                "system": system,
                "messages": messages,
                "tools": tools,
            });

            let response = self
                .client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .context("HTTP request to Anthropic API failed")?;

            let status = response.status();
            let resp_json: Value = response
                .json()
                .await
                .context("Failed to parse Anthropic response")?;

            if !status.is_success() {
                bail!(
                    "Anthropic API error {status}: {}",
                    resp_json.get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown error")
                );
            }

            // Accumulate token usage.
            if let Some(usage) = resp_json.get("usage") {
                input_tokens += usage
                    .get("input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                output_tokens += usage
                    .get("output_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
            }

            let content = resp_json
                .get("content")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();

            let stop_reason = resp_json
                .get("stop_reason")
                .and_then(|s| s.as_str())
                .unwrap_or("end_turn");

            // Append the assistant message.
            messages.push(json!({ "role": "assistant", "content": content }));

            // Collect any tool_use blocks.
            let tool_use_blocks: Vec<&Value> = content
                .iter()
                .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
                .collect();

            if tool_use_blocks.is_empty() || stop_reason == "end_turn" {
                // Extract text answer.
                for block in &content {
                    if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                            final_answer.push_str(text);
                        }
                    }
                }
                break;
            }

            // Execute each tool call and collect results.
            let mut tool_results: Vec<Value> = Vec::new();

            for block in tool_use_blocks {
                let tool_id = block
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                let tool_name = block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                let input = block
                    .get("input")
                    .cloned()
                    .unwrap_or(json!({}));

                debug!("Executing tool: {tool_name}");
                let output = self.execute_tool(&tool_name, &input, condition);
                let output_len = output.len();

                all_tool_calls.push(ToolCall {
                    tool_name: tool_name.clone(),
                    input: input.clone(),
                    output_len,
                });

                tool_results.push(json!({
                    "type": "tool_result",
                    "tool_use_id": tool_id,
                    "content": output,
                }));
            }

            messages.push(json!({ "role": "user", "content": tool_results }));
        }

        let wall_time_ms = start.elapsed().as_millis() as u64;

        Ok(RunResult {
            task_id: task.id.clone(),
            condition: condition.clone(),
            model: self.model.clone(),
            answer: final_answer,
            tool_calls: all_tool_calls,
            input_tokens,
            output_tokens,
            wall_time_ms,
            completed_at: Utc::now(),
        })
    }

    // -----------------------------------------------------------------------
    // Tool execution — native tools + BW tools
    // -----------------------------------------------------------------------

    fn execute_tool(&self, name: &str, input: &Value, _condition: &Condition) -> String {
        match name {
            "Read" => self.tool_read(input),
            "Grep" => self.tool_grep(input),
            "Glob" => self.tool_glob(input),
            "ListDir" => self.tool_list_dir(input),
            "bw_search" => self.tool_bw_search(input),
            "bw_symbol_info" => self.tool_bw_symbol_info(input),
            "bw_find_references" => self.tool_bw_find_references(input),
            "bw_blast_radius" => self.tool_bw_blast_radius(input),
            "bw_architecture_overview" => self.tool_bw_architecture(input),
            "bw_calls_in" => self.tool_bw_calls_in(input),
            "bw_calls_out" => self.tool_bw_calls_out(input),
            other => format!("{{\"error\": \"unknown tool: {other}\"}}"),
        }
    }

    // --- Native: Read ---
    fn tool_read(&self, input: &Value) -> String {
        let path_str = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return r#"{"error": "path parameter required"}"#.to_owned(),
        };

        // Accept absolute paths or resolve relative to project root.
        let path = if Path::new(path_str).is_absolute() {
            PathBuf::from(path_str)
        } else {
            self.project_root.join(path_str)
        };

        let start_line = input
            .get("start_line")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .saturating_sub(1) as usize;
        let end_line = input
            .get("end_line")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();
                let end = end_line.unwrap_or(lines.len()).min(lines.len());
                let slice = if start_line < end {
                    lines[start_line..end].join("\n")
                } else {
                    lines.join("\n")
                };
                json!({
                    "path": path_str,
                    "content": slice,
                    "total_lines": lines.len(),
                })
                .to_string()
            }
            Err(e) => json!({"error": format!("Failed to read {path_str}: {e}")}).to_string(),
        }
    }

    // --- Native: Grep ---
    fn tool_grep(&self, input: &Value) -> String {
        let pattern_str = match input.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return r#"{"error": "pattern parameter required"}"#.to_owned(),
        };
        let is_regex = input
            .get("regex")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let case_insensitive = input
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let glob_filter = input
            .get("glob")
            .and_then(|v| v.as_str())
            .unwrap_or("**/*");
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(30) as usize;

        let regex_pattern = if is_regex {
            if case_insensitive {
                format!("(?i){pattern_str}")
            } else {
                pattern_str.to_owned()
            }
        } else {
            let escaped = regex::escape(pattern_str);
            if case_insensitive {
                format!("(?i){escaped}")
            } else {
                escaped
            }
        };

        let re = match Regex::new(&regex_pattern) {
            Ok(r) => r,
            Err(e) => return json!({"error": format!("Invalid pattern: {e}")}).to_string(),
        };

        // Build a glob matcher from the filter string.
        let glob_re = glob_to_regex(glob_filter);

        let mut matches: Vec<Value> = Vec::new();
        let walker = walkdir::WalkDir::new(&self.project_root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                // Skip .git, .bearwisdom, target directories.
                let name = e.file_name().to_string_lossy();
                !matches!(name.as_ref(), ".git" | ".bearwisdom" | "target" | "node_modules")
            });

        for entry in walker.flatten() {
            if !entry.file_type().is_file() {
                continue;
            }
            let rel_path = entry
                .path()
                .strip_prefix(&self.project_root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");

            if !glob_re.is_match(&rel_path) {
                continue;
            }

            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                for (line_idx, line) in content.lines().enumerate() {
                    if re.is_match(line) {
                        matches.push(json!({
                            "file": rel_path,
                            "line": line_idx + 1,
                            "text": line.trim(),
                        }));
                        if matches.len() >= limit {
                            break;
                        }
                    }
                }
            }

            if matches.len() >= limit {
                break;
            }
        }

        json!({ "matches": matches, "count": matches.len() }).to_string()
    }

    // --- Native: Glob ---
    fn tool_glob(&self, input: &Value) -> String {
        let pattern = match input.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return r#"{"error": "pattern parameter required"}"#.to_owned(),
        };
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(100) as usize;

        let glob_re = glob_to_regex(pattern);
        let mut files: Vec<String> = Vec::new();

        let walker = walkdir::WalkDir::new(&self.project_root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                !matches!(name.as_ref(), ".git" | ".bearwisdom" | "target" | "node_modules")
            });

        for entry in walker.flatten() {
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&self.project_root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");

            if glob_re.is_match(&rel) {
                files.push(rel);
                if files.len() >= limit {
                    break;
                }
            }
        }

        json!({ "files": files, "count": files.len() }).to_string()
    }

    // --- Native: ListDir ---
    fn tool_list_dir(&self, input: &Value) -> String {
        let path_str = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return r#"{"error": "path parameter required"}"#.to_owned(),
        };

        let dir = if Path::new(path_str).is_absolute() {
            PathBuf::from(path_str)
        } else {
            self.project_root.join(path_str)
        };

        match std::fs::read_dir(&dir) {
            Ok(entries) => {
                let items: Vec<Value> = entries
                    .flatten()
                    .map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                        json!({ "name": name, "is_dir": is_dir })
                    })
                    .collect();
                json!({ "path": path_str, "entries": items }).to_string()
            }
            Err(e) => json!({"error": format!("Failed to list {path_str}: {e}")}).to_string(),
        }
    }

    // --- BW: bw_search ---
    fn tool_bw_search(&self, input: &Value) -> String {
        let query = match input.get("query").and_then(|v| v.as_str()) {
            Some(q) => q.to_owned(),
            None => return r#"{"error": "query parameter required"}"#.to_owned(),
        };
        let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(15) as usize;

        let db = match self.pool.get() {
            Ok(d) => d,
            Err(_) => return r#"{"error": "pool connection failed"}"#.to_owned(),
        };
        match search_mod::search_symbols(&db, &query, limit, &bearwisdom::query::QueryOptions::full()) {
            Ok(results) => serde_json::to_string(&results)
                .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}")),
            Err(e) => format!("{{\"error\": \"{e}\"}}"),
        }
    }

    // --- BW: bw_symbol_info ---
    fn tool_bw_symbol_info(&self, input: &Value) -> String {
        let name = match input.get("name").and_then(|v| v.as_str()) {
            Some(n) => n.to_owned(),
            None => return r#"{"error": "name parameter required"}"#.to_owned(),
        };
        let db = match self.pool.get() {
            Ok(d) => d,
            Err(_) => return r#"{"error": "pool connection failed"}"#.to_owned(),
        };
        match symbol_info::symbol_info(&db, &name, &bearwisdom::query::QueryOptions::full()) {
            Ok(results) => serde_json::to_string(&results)
                .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}")),
            Err(e) => format!("{{\"error\": \"{e}\"}}"),
        }
    }

    // --- BW: bw_find_references ---
    fn tool_bw_find_references(&self, input: &Value) -> String {
        let name = match input.get("name").and_then(|v| v.as_str()) {
            Some(n) => n.to_owned(),
            None => return r#"{"error": "name parameter required"}"#.to_owned(),
        };
        let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
        let db = match self.pool.get() {
            Ok(d) => d,
            Err(_) => return r#"{"error": "pool connection failed"}"#.to_owned(),
        };
        match references::find_references(&db, &name, limit) {
            Ok(results) => serde_json::to_string(&results)
                .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}")),
            Err(e) => format!("{{\"error\": \"{e}\"}}"),
        }
    }

    // --- BW: bw_blast_radius ---
    fn tool_bw_blast_radius(&self, input: &Value) -> String {
        let symbol = match input.get("symbol").and_then(|v| v.as_str()) {
            Some(s) => s.to_owned(),
            None => return r#"{"error": "symbol parameter required"}"#.to_owned(),
        };
        let depth = input
            .get("depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(3)
            .min(10)
            .max(1) as u32;
        let db = match self.pool.get() {
            Ok(d) => d,
            Err(_) => return r#"{"error": "pool connection failed"}"#.to_owned(),
        };
        match blast_radius_mod::blast_radius(&db, &symbol, depth, 500) {
            Ok(result) => serde_json::to_string(&result)
                .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}")),
            Err(e) => format!("{{\"error\": \"{e}\"}}"),
        }
    }

    // --- BW: bw_architecture_overview ---
    fn tool_bw_architecture(&self, _input: &Value) -> String {
        let db = match self.pool.get() {
            Ok(d) => d,
            Err(_) => return r#"{"error": "pool connection failed"}"#.to_owned(),
        };
        match architecture::get_overview(&db) {
            Ok(result) => serde_json::to_string(&result)
                .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}")),
            Err(e) => format!("{{\"error\": \"{e}\"}}"),
        }
    }

    // --- BW: bw_calls_in ---
    fn tool_bw_calls_in(&self, input: &Value) -> String {
        let name = match input.get("name").and_then(|v| v.as_str()) {
            Some(n) => n.to_owned(),
            None => return r#"{"error": "name parameter required"}"#.to_owned(),
        };
        let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
        let db = match self.pool.get() {
            Ok(d) => d,
            Err(_) => return r#"{"error": "pool connection failed"}"#.to_owned(),
        };
        match call_hierarchy::incoming_calls(&db, &name, limit) {
            Ok(results) => serde_json::to_string(&results)
                .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}")),
            Err(e) => format!("{{\"error\": \"{e}\"}}"),
        }
    }

    // --- BW: bw_calls_out ---
    fn tool_bw_calls_out(&self, input: &Value) -> String {
        let name = match input.get("name").and_then(|v| v.as_str()) {
            Some(n) => n.to_owned(),
            None => return r#"{"error": "name parameter required"}"#.to_owned(),
        };
        let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
        let db = match self.pool.get() {
            Ok(d) => d,
            Err(_) => return r#"{"error": "pool connection failed"}"#.to_owned(),
        };
        match call_hierarchy::outgoing_calls(&db, &name, limit) {
            Ok(results) => serde_json::to_string(&results)
                .unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}")),
            Err(e) => format!("{{\"error\": \"{e}\"}}"),
        }
    }

    // -----------------------------------------------------------------------
    // Tool definitions (Anthropic tool_use format)
    // -----------------------------------------------------------------------

    fn build_tools(&self, condition: &Condition) -> Value {
        let mut tools = vec![
            json!({
                "name": "Read",
                "description": "Read the contents of a file. Returns the file content as text.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Absolute or project-relative file path to read"
                        },
                        "start_line": {
                            "type": "integer",
                            "description": "1-based start line (optional, defaults to beginning)"
                        },
                        "end_line": {
                            "type": "integer",
                            "description": "1-based end line inclusive (optional, defaults to end of file)"
                        }
                    },
                    "required": ["path"]
                }
            }),
            json!({
                "name": "Grep",
                "description": "Search for a pattern across source files. Returns matching lines with file path and line number.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Search pattern (literal substring by default, or regex if regex=true)"
                        },
                        "regex": {
                            "type": "boolean",
                            "description": "Treat pattern as a regular expression (default: false)"
                        },
                        "case_insensitive": {
                            "type": "boolean",
                            "description": "Case-insensitive search (default: false)"
                        },
                        "glob": {
                            "type": "string",
                            "description": "Glob pattern to filter which files are searched (e.g. **/*.rs)"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of matches to return (default: 30)"
                        }
                    },
                    "required": ["pattern"]
                }
            }),
            json!({
                "name": "Glob",
                "description": "List files matching a glob pattern relative to the project root.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Glob pattern, e.g. **/*.ts or src/**/*.rs"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of results (default: 100)"
                        }
                    },
                    "required": ["pattern"]
                }
            }),
            json!({
                "name": "ListDir",
                "description": "List the contents of a directory.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Absolute or project-relative directory path"
                        }
                    },
                    "required": ["path"]
                }
            }),
        ];

        if *condition == Condition::UseBearWisdom {
            tools.extend([
                json!({
                    "name": "bw_search",
                    "description": "Search code symbols (functions, classes, methods, types) by keyword. Returns ranked results with file location, kind, and signature. Use for finding symbols by name or partial name.",
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "query": {
                                "type": "string",
                                "description": "Search keywords (symbol names, words from signatures or doc comments)"
                            },
                            "limit": {
                                "type": "integer",
                                "description": "Maximum results to return (default: 15)"
                            }
                        },
                        "required": ["query"]
                    }
                }),
                json!({
                    "name": "bw_symbol_info",
                    "description": "Get detailed information about a specific code symbol: signature, doc comment, location, visibility, and edge counts. Use after bw_search to drill into a result.",
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Symbol name or qualified name (e.g. 'DoWork', 'App.Svc.DoWork')"
                            }
                        },
                        "required": ["name"]
                    }
                }),
                json!({
                    "name": "bw_find_references",
                    "description": "Find all locations where a symbol is referenced across the codebase. Returns each reference with file path, line number, referencing symbol, and edge kind.",
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Symbol name to find references for"
                            },
                            "limit": {
                                "type": "integer",
                                "description": "Maximum results to return (default: 100)"
                            }
                        },
                        "required": ["name"]
                    }
                }),
                json!({
                    "name": "bw_blast_radius",
                    "description": "Analyze the blast radius of changing a specific symbol. Shows which symbols are transitively affected (callers, implementors, type users). Use before modifying a symbol.",
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "symbol": {
                                "type": "string",
                                "description": "Symbol name or qualified name to analyze impact for"
                            },
                            "depth": {
                                "type": "integer",
                                "description": "Max traversal depth (default: 3)"
                            }
                        },
                        "required": ["symbol"]
                    }
                }),
                json!({
                    "name": "bw_architecture_overview",
                    "description": "Get a high-level summary of the indexed project: languages, file/symbol counts, hotspots (most-referenced symbols), and entry points. Use at the start of a session to understand the codebase. No parameters needed.",
                    "input_schema": {
                        "type": "object",
                        "properties": {},
                        "required": []
                    }
                }),
                json!({
                    "name": "bw_calls_in",
                    "description": "Show all callers of a function or method (incoming call hierarchy). Use to trace who calls this function.",
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Function or method name (simple or qualified)"
                            },
                            "limit": {
                                "type": "integer",
                                "description": "Maximum results to return (default: 50)"
                            }
                        },
                        "required": ["name"]
                    }
                }),
                json!({
                    "name": "bw_calls_out",
                    "description": "Show all callees of a function or method (outgoing call hierarchy). Use to trace what this function calls.",
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "Function or method name (simple or qualified)"
                            },
                            "limit": {
                                "type": "integer",
                                "description": "Maximum results to return (default: 50)"
                            }
                        },
                        "required": ["name"]
                    }
                }),
            ]);
        }

        json!(tools)
    }

    fn system_prompt(&self, condition: &Condition, task: &BenchmarkTask) -> String {
        let tool_desc = match condition {
            Condition::UseBearWisdom => {
                "You have access to BearWisdom code intelligence tools (bw_*) in addition to \
                 standard file navigation tools. Use the BearWisdom tools first — they provide \
                 pre-indexed, structured answers faster than grepping files manually."
            }
            Condition::NoBearWisdom => {
                "You have access to file navigation tools (Read, Grep, Glob, ListDir). \
                 Use these to answer the question by exploring the codebase directly."
            }
            Condition::UseBearWisdomCli => {
                panic!("UseBearWisdomCli is not supported by the API runner — use the CLI backend instead")
            }
        };

        format!(
            "You are a code analysis assistant analyzing a codebase at `{}`. {tool_desc}\n\n\
             CRITICAL RULES:\n\
             1. You MUST use the provided tools to answer. Do NOT answer from memory or prior knowledge.\n\
             2. Always call tools first to gather information, then synthesize an answer from tool results.\n\
             3. List every relevant symbol you find by its exact name (e.g. CatalogItem, OrderService.PlaceOrder).\n\
             4. Include file paths and line numbers for each symbol.\n\
             5. Be exhaustive — find ALL relevant symbols, not just the first few.",
            task.project_path
        )
    }
}

// ===========================================================================
// CliRunner — uses the `claude` CLI (Claude Code subscription)
// ===========================================================================

pub struct CliRunner {
    model: String,
    project_root: PathBuf,
    mcp_config_path: PathBuf,
    bw_cli_binary: String,
    bw_agent_prompt: String,
}

impl CliRunner {
    pub fn new(model: String, project_root: PathBuf) -> Result<Self> {
        // Write a temporary MCP config for the BearWisdom server.
        let mcp_config_path = project_root.join(".bearwisdom").join("bench-mcp.json");
        let bw_mcp_binary = find_bw_mcp_binary()?;

        let mcp_config = json!({
            "mcpServers": {
                "bearwisdom": {
                    "type": "stdio",
                    "command": bw_mcp_binary,
                    "args": ["--project", project_root.to_string_lossy()]
                }
            }
        });

        if let Some(parent) = mcp_config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&mcp_config_path, serde_json::to_string_pretty(&mcp_config)?)?;

        let bw_cli_binary = find_bw_cli_binary()?;

        // Load the BearWisdom agent prompt for the CLI condition.
        // Look relative to the bw-bench binary, then common locations.
        let bw_agent_prompt = load_bw_agent_prompt()?;

        Ok(Self {
            model,
            project_root,
            mcp_config_path,
            bw_cli_binary,
            bw_agent_prompt,
        })
    }

    pub async fn run_all(
        &self,
        task_set: &TaskSet,
        conditions: &[Condition],
        output_dir: &Path,
    ) -> Result<Vec<RunResult>> {
        std::fs::create_dir_all(output_dir)?;
        let mut results = Vec::new();

        for condition in conditions {
            info!("Running condition: {condition}");
            for task in &task_set.tasks {
                info!("  Task {} [{}]", task.id, task.category.as_str());
                match self.run_task(task, condition).await {
                    Ok(result) => {
                        let filename = format!("{}-{}.json", task.id, condition);
                        let path = output_dir.join(&filename);
                        let json_str = serde_json::to_string_pretty(&result)?;
                        std::fs::write(&path, json_str)?;
                        results.push(result);
                    }
                    Err(e) => {
                        warn!("Task {} failed: {e:#}", task.id);
                    }
                }
            }
        }

        Ok(results)
    }

    async fn run_task(&self, task: &BenchmarkTask, condition: &Condition) -> Result<RunResult> {
        let start = Instant::now();

        let base_rules = format!(
            "You are a code analysis assistant analyzing a codebase at `{}`.\n\n\
             CRITICAL RULES:\n\
             1. You MUST use the provided tools to answer. Do NOT answer from memory or prior knowledge.\n\
             2. Always call tools first to gather information, then synthesize an answer from tool results.\n\
             3. List every relevant symbol you find by its exact name (e.g. CatalogItem, OrderService.PlaceOrder).\n\
             4. Include file paths and line numbers for each symbol.\n\
             5. Be exhaustive — find ALL relevant symbols, not just the first few.",
            task.project_path
        );

        let system_prompt = match condition {
            Condition::UseBearWisdomCli => format!(
                "{base_rules}\n\n\
                 The project is already indexed. Use the BearWisdom CLI (`{bw}`) via Bash \
                 to answer — it returns structured, pre-indexed answers faster than grepping \
                 files manually.\n\n{agent_prompt}",
                bw = self.bw_cli_binary,
                agent_prompt = self.bw_agent_prompt,
            ),
            _ => base_rules,
        };

        let mut cmd = std::process::Command::new("claude");
        cmd.arg("-p")
            .arg("--output-format").arg("json")
            .arg("--verbose")
            .arg("--model").arg(&self.model)
            .arg("--append-system-prompt").arg(&system_prompt)
            .arg("--dangerously-skip-permissions");

        // Set working directory to the project root so native tools work correctly.
        cmd.current_dir(&self.project_root);

        match condition {
            Condition::NoBearWisdom => {
                // --strict-mcp-config with no --mcp-config disables all project MCP servers.
                cmd.arg("--strict-mcp-config")
                    .arg("--allowedTools")
                    .arg("Read,Grep,Glob,Bash(find:*),Bash(ls:*)");
            }
            Condition::UseBearWisdom => {
                cmd.arg("--strict-mcp-config")
                    .arg("--mcp-config").arg(&self.mcp_config_path)
                    .arg("--allowedTools")
                    .arg("Read,Grep,Glob,Bash(find:*),Bash(ls:*),mcp__bearwisdom__*");
            }
            Condition::UseBearWisdomCli => {
                // No MCP — model uses bw CLI via Bash.
                cmd.arg("--strict-mcp-config")
                    .arg("--allowedTools")
                    .arg(format!("Read,Grep,Glob,Bash(find:*),Bash(ls:*),Bash({}:*)", self.bw_cli_binary));
            }
        }

        // Pipe the question via stdin.
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        info!("    Spawning claude CLI...");
        let question = task.question.clone();
        let output = tokio::task::spawn_blocking(move || {
            let mut child = cmd.spawn()?;
            if let Some(mut stdin) = child.stdin.take() {
                use std::io::Write;
                let _ = stdin.write_all(question.as_bytes());
                // stdin drops here, closing the pipe
            }
            child.wait_with_output()
        })
        .await?
        .context("Failed to execute claude CLI")?;

        let wall_time_ms = start.elapsed().as_millis() as u64;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("claude CLI failed (exit {}): {}", output.status, stderr.chars().take(500).collect::<String>());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        // With --verbose, output is a JSON array of conversation events.
        // Parse each event to extract assistant text, tool calls, and usage.
        let events: Vec<Value> = serde_json::from_str(&stdout)
            .with_context(|| format!("Failed to parse claude CLI output ({}b)", stdout.len()))?;

        let mut answer = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut input_tokens: u64 = 0;
        let mut output_tokens: u64 = 0;

        for event in &events {
            let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");

            match event_type {
                "assistant" => {
                    if let Some(content) = event.get("message")
                        .and_then(|m| m.get("content"))
                        .and_then(|c| c.as_array())
                    {
                        for block in content {
                            match block.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                                "text" => {
                                    if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                        answer.push_str(text);
                                    }
                                }
                                "tool_use" => {
                                    let name = block.get("name")
                                        .and_then(|n| n.as_str())
                                        .unwrap_or("unknown")
                                        .to_owned();
                                    let input_val = block.get("input").cloned().unwrap_or(json!({}));
                                    tool_calls.push(ToolCall {
                                        tool_name: name,
                                        input: input_val,
                                        output_len: 0,
                                    });
                                }
                                _ => {}
                            }
                        }
                    }
                }
                "tool" => {
                    // Update output_len for the most recent tool call.
                    if let Some(content) = event.get("content").and_then(|c| c.as_str()) {
                        if let Some(last_tc) = tool_calls.last_mut() {
                            last_tc.output_len = content.len();
                        }
                    }
                }
                "result" => {
                    if let Some(usage) = event.get("usage") {
                        input_tokens = usage.get("input_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        output_tokens = usage.get("output_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                    }
                    // Also check result text as fallback.
                    if answer.is_empty() {
                        if let Some(text) = event.get("result").and_then(|r| r.as_str()) {
                            answer.push_str(text);
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(RunResult {
            task_id: task.id.clone(),
            condition: condition.clone(),
            model: self.model.clone(),
            answer,
            tool_calls,
            input_tokens,
            output_tokens,
            wall_time_ms,
            completed_at: Utc::now(),
        })
    }
}

/// Find the bw-mcp binary — check target/release, target/debug, then PATH.
fn find_bw_mcp_binary() -> Result<String> {
    find_binary("bw-mcp")
}

/// Find the bw CLI binary — check target/release, target/debug, then PATH.
fn find_bw_cli_binary() -> Result<String> {
    find_binary("bw")
}

/// Load the BearWisdom agent markdown (stripping frontmatter) for use as a system prompt.
fn load_bw_agent_prompt() -> Result<String> {
    // Check relative to the current exe first (same repo checkout).
    let candidates = [
        // Relative to exe: target/release/../../agents/bearwisdom.md
        std::env::current_exe()
            .ok()
            .and_then(|e| e.parent()?.parent()?.parent().map(|p| p.join("agents/bearwisdom.md"))),
        // Current working directory
        Some(PathBuf::from("agents/bearwisdom.md")),
    ];

    for candidate in candidates.iter().flatten() {
        if candidate.exists() {
            let content = std::fs::read_to_string(candidate)
                .with_context(|| format!("Failed to read {}", candidate.display()))?;
            // Strip YAML frontmatter (--- ... ---).
            let body = if content.starts_with("---") {
                if let Some(end) = content[3..].find("---") {
                    content[3 + end + 3..].trim_start().to_owned()
                } else {
                    content
                }
            } else {
                content
            };
            return Ok(body);
        }
    }

    bail!("Could not find agents/bearwisdom.md — run from the BearWisdom repo root or build directory")
}

fn find_binary(name: &str) -> Result<String> {
    // Check relative to the current exe (benchmarks binary lives in same target dir).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(name).with_extension(std::env::consts::EXE_EXTENSION);
            if candidate.exists() {
                return Ok(candidate.to_string_lossy().into_owned());
            }
        }
    }

    // Check common build locations.
    for subdir in ["target/release", "target/debug"] {
        let candidate = PathBuf::from(subdir).join(name).with_extension(std::env::consts::EXE_EXTENSION);
        if candidate.exists() {
            return Ok(std::fs::canonicalize(candidate)?.to_string_lossy().into_owned());
        }
    }

    // Fall back to PATH.
    Ok(name.to_owned())
}

// ---------------------------------------------------------------------------
// Glob pattern → Regex conversion (minimal: * and ** and ?)
// ---------------------------------------------------------------------------

fn glob_to_regex(pattern: &str) -> Regex {
    let mut re = String::from("^");
    let mut chars = pattern.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    // "**" matches any path segment including slashes.
                    re.push_str(".*");
                    // Skip optional following slash.
                    if chars.peek() == Some(&'/') {
                        chars.next();
                        re.push_str("(?:.+/)?");
                    }
                } else {
                    // Single * matches within one path component.
                    re.push_str("[^/]*");
                }
            }
            '?' => re.push_str("[^/]"),
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                re.push('\\');
                re.push(c);
            }
            _ => re.push(c),
        }
    }

    re.push('$');
    Regex::new(&re).unwrap_or_else(|_| Regex::new("^$").unwrap())
}

// ---------------------------------------------------------------------------
// Load persisted RunResults from a directory
// ---------------------------------------------------------------------------

pub fn load_results(dir: &Path) -> Result<Vec<RunResult>> {
    let mut results = Vec::new();

    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("Failed to read results directory {}", dir.display()))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        // Skip the aggregate report files.
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if name == "report" || name == "results" {
            continue;
        }

        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        // Try to parse as RunResult; skip files that are not RunResults.
        if let Ok(result) = serde_json::from_str::<RunResult>(&data) {
            results.push(result);
        }
    }

    Ok(results)
}
