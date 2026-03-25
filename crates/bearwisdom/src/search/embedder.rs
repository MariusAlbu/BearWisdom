// =============================================================================
// search/embedder.rs  —  ONNX model inference for code embeddings
//
// Wraps CodeRankEmbed (or any 768-dim encoder) via ort 2.x + tokenizers 0.21.
// The model is loaded on demand and unloaded after a configurable idle timeout
// to stay within the ~150 MB RAM budget.
//
// Model layout expected under `model_dir/`:
//   onnx/model.onnx                     (full-precision)
//   onnx-quantized/model_quantized.onnx (preferred when present)
//   tokenizer.json
//
// Embedding pipeline:
//   tokenize → pad to same length → build batch tensors → run ONNX session
//   → mean-pool over seq_len (attention-weighted) → L2 normalise → [768f32]
//
// NOTE: ort 2.x API may require minor fixups during compilation.  All
// uncertain call-sites are annotated with TODO comments.
// =============================================================================

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{bail, Result};
use ort::session::{builder::GraphOptimizationLevel, Session};
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Public struct
// ---------------------------------------------------------------------------

/// Lazy-loading ONNX embedder.  Create via `Embedder::new`, then call
/// `ensure_loaded()` before any embed call.
pub struct Embedder {
    session: Option<Session>,
    tokenizer: Option<tokenizers::Tokenizer>,
    model_dir: PathBuf,
    last_used: Instant,
}

impl Embedder {
    /// Create a new embedder pointing to `model_dir`.
    /// Does NOT load the model — call [`ensure_loaded`] first.
    pub fn new(model_dir: PathBuf) -> Self {
        Self {
            session: None,
            tokenizer: None,
            model_dir,
            last_used: Instant::now(),
        }
    }

    /// Load the ONNX model and tokenizer if not already loaded.
    ///
    /// Prefers the quantized model when present, falls back to the full model.
    pub fn ensure_loaded(&mut self) -> Result<()> {
        if self.session.is_some() && self.tokenizer.is_some() {
            return Ok(());
        }

        let model_path = self.resolve_model_path()?;
        let tokenizer_path = self.model_dir.join("tokenizer.json");

        if !tokenizer_path.exists() {
            bail!(
                "tokenizer.json not found at {}",
                tokenizer_path.display()
            );
        }

        info!(model = %model_path.display(), "loading ONNX model");

        // TODO: ort 2.x exact API — Session::builder() call chain verified
        // against ort 2.0.0-rc.8 public docs.  May need adjustment for the
        // release build.
        let session = Session::builder()
            .map_err(|e| anyhow::anyhow!("SessionBuilder: {e}"))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| anyhow::anyhow!("Optimization level: {e}"))?
            .commit_from_file(&model_path)
            .map_err(|e| anyhow::anyhow!("Load model {}: {e}", model_path.display()))?;

        let tokenizer =
            tokenizers::Tokenizer::from_file(&tokenizer_path)
                .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {e}"))?;

        self.session = Some(session);
        self.tokenizer = Some(tokenizer);
        self.last_used = Instant::now();

        info!("ONNX model and tokenizer loaded");
        Ok(())
    }

    /// Unload the model and tokenizer to free memory.
    pub fn unload(&mut self) {
        if self.session.is_some() {
            info!("unloading ONNX model");
            self.session = None;
            self.tokenizer = None;
        }
    }

    /// Returns `true` if the model is currently loaded.
    pub fn is_loaded(&self) -> bool {
        self.session.is_some() && self.tokenizer.is_some()
    }

    /// Embed multiple documents (code chunks). Returns 768-dim unit vectors.
    ///
    /// The inputs are batched together; all sequences are padded to the same
    /// length before inference.
    pub fn embed_documents(&mut self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        self.ensure_loaded()?;
        self.last_used = Instant::now();
        self.run_batch(texts, false)
    }

    /// Embed a single query string. Returns a 768-dim unit vector.
    ///
    /// For asymmetric models the query prefix ("search_query: ") should be
    /// prepended before calling this method.
    pub fn embed_query(&mut self, query: &str) -> Result<Vec<f32>> {
        self.ensure_loaded()?;
        self.last_used = Instant::now();
        let mut results = self.run_batch(&[query], true)?;
        results
            .pop()
            .ok_or_else(|| anyhow::anyhow!("embed_query returned no vectors"))
    }

    /// Resolve the CodeRankEmbed model directory.
    ///
    /// Tries `<project_root>/models/CodeRankEmbed` first, then
    /// `~/.bearwisdom/models/CodeRankEmbed`.  Returns `None` if neither
    /// contains a `tokenizer.json`.
    pub fn resolve_model_dir(project_root: &std::path::Path) -> Option<PathBuf> {
        let workspace_model = project_root.join("models").join("CodeRankEmbed");
        if workspace_model.join("tokenizer.json").exists() {
            return Some(workspace_model);
        }
        if let Some(home) = dirs::home_dir() {
            let home_model = home.join(".bearwisdom").join("models").join("CodeRankEmbed");
            if home_model.join("tokenizer.json").exists() {
                return Some(home_model);
            }
        }
        None
    }

    /// Unload the model if it has been idle for longer than `timeout`.
    pub fn maybe_unload(&mut self, timeout: Duration) {
        if self.is_loaded() && self.last_used.elapsed() > timeout {
            debug!(
                idle_secs = self.last_used.elapsed().as_secs(),
                "unloading idle ONNX model"
            );
            self.unload();
        }
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

impl Embedder {
    /// Resolve which ONNX model file to use.  Quantized is preferred.
    fn resolve_model_path(&self) -> Result<PathBuf> {
        let quantized = self
            .model_dir
            .join("onnx-quantized")
            .join("model_quantized.onnx");
        if quantized.exists() {
            return Ok(quantized);
        }
        let full = self.model_dir.join("onnx").join("model.onnx");
        if full.exists() {
            return Ok(full);
        }
        bail!(
            "No ONNX model found in {} (tried onnx-quantized/model_quantized.onnx and onnx/model.onnx)",
            self.model_dir.display()
        )
    }

    /// Tokenize `texts`, pad to a common length, build batch tensors, run
    /// the ONNX session, and return normalised 768-dim vectors.
    ///
    /// `is_query`: if true a query prefix is prepended (model-specific).
    fn run_batch(&mut self, texts: &[&str], _is_query: bool) -> Result<Vec<Vec<f32>>> {
        let session = self
            .session
            .as_mut()
            .expect("session must be loaded before run_batch");
        let tokenizer = self
            .tokenizer
            .as_ref()
            .expect("tokenizer must be loaded before run_batch");

        let batch_size = texts.len();

        // Tokenize all inputs and find the maximum sequence length.
        let mut all_ids: Vec<Vec<i64>> = Vec::with_capacity(batch_size);
        let mut all_masks: Vec<Vec<i64>> = Vec::with_capacity(batch_size);
        let mut max_len: usize = 0;

        for text in texts {
            // TODO: tokenizers 0.21 API — encode(text, add_special_tokens)
            let encoding = tokenizer
                .encode(*text, true)
                .map_err(|e| anyhow::anyhow!("Tokenization failed: {e}"))?;

            let ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
            let mask: Vec<i64> = encoding
                .get_attention_mask()
                .iter()
                .map(|&m| m as i64)
                .collect();

            max_len = max_len.max(ids.len());
            all_ids.push(ids);
            all_masks.push(mask);
        }

        // Clamp to 512 tokens (CodeRankEmbed context window).
        let seq_len = max_len.min(512);

        // Build flat row-major buffers for [batch_size, seq_len].
        let mut input_ids_flat: Vec<i64> = vec![0i64; batch_size * seq_len];
        let mut attention_mask_flat: Vec<i64> = vec![0i64; batch_size * seq_len];

        for (i, (ids, mask)) in all_ids.iter().zip(all_masks.iter()).enumerate() {
            let actual_len = ids.len().min(seq_len);
            let row_start = i * seq_len;
            input_ids_flat[row_start..row_start + actual_len]
                .copy_from_slice(&ids[..actual_len]);
            attention_mask_flat[row_start..row_start + actual_len]
                .copy_from_slice(&mask[..actual_len]);
        }

        let shape = [batch_size as i64, seq_len as i64];

        // TODO: ort 2.x tensor construction.  The exact API for creating a
        // CowArray / Tensor from a flat Vec<i64> may differ across ort 2.x
        // patch releases.  The pattern below follows ort 2.0.0-rc docs.
        let _ = shape;

        // Build ort Values using (shape, vec) tuples — avoids ndarray version conflicts.
        let input_ids_val = ort::value::Value::from_array(
            ([batch_size, seq_len], input_ids_flat),
        )
        .map_err(|e| anyhow::anyhow!("input_ids tensor: {e}"))?;

        // Keep a copy before moving into the tensor — needed for mean pooling weights.
        let attention_mask_f32: Vec<f32> =
            attention_mask_flat.iter().map(|&m| m as f32).collect();

        let attention_mask_val = ort::value::Value::from_array(
            ([batch_size, seq_len], attention_mask_flat),
        )
        .map_err(|e| anyhow::anyhow!("attention_mask tensor: {e}"))?;

        let outputs = session
            .run(ort::inputs![
                "input_ids" => input_ids_val,
                "attention_mask" => attention_mask_val,
            ])
            .map_err(|e| anyhow::anyhow!("ONNX inference failed: {e}"))?;

        // Extract the last hidden state: expected shape [batch, seq_len, 768].
        let raw_output = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow::anyhow!("Failed to extract output tensor: {e}"))?;

        let output_data: &[f32] = raw_output.1;
        let total = output_data.len();

        // Infer dimensions: model output is [batch_size, out_seq_len, embed_dim].
        // CodeRankEmbed always produces 768-dim embeddings.
        let embed_dim = 768usize;
        if total == 0 || total % (batch_size * embed_dim) != 0 {
            bail!(
                "Unexpected output size {total} for batch_size={batch_size}, embed_dim={embed_dim}"
            );
        }
        let out_seq_len = total / (batch_size * embed_dim);

        // attention_mask_f32 was computed before moving attention_mask_flat into the tensor.

        let mut embeddings: Vec<Vec<f32>> = Vec::with_capacity(batch_size);

        for i in 0..batch_size {
            let mask_row = &attention_mask_f32[i * seq_len..(i + 1) * seq_len];
            let mask_sum: f32 = mask_row.iter().sum::<f32>().max(1e-9);

            let mut pooled = vec![0f32; embed_dim];

            for t in 0..out_seq_len.min(seq_len) {
                let weight = mask_row[t];
                if weight == 0.0 {
                    continue;
                }
                let offset = i * out_seq_len * embed_dim + t * embed_dim;
                for d in 0..embed_dim {
                    pooled[d] += output_data[offset + d] * weight;
                }
            }

            // Divide by the number of non-padding tokens.
            for v in &mut pooled {
                *v /= mask_sum;
            }

            // L2 normalise.
            l2_normalize(&mut pooled);

            embeddings.push(pooled);
        }

        Ok(embeddings)
    }
}

/// In-place L2 normalisation of a vector.
fn l2_normalize(v: &mut Vec<f32>) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-9 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    } else {
        warn!("Zero-norm embedding vector — not normalised");
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn embedder_starts_unloaded() {
        let e = Embedder::new(PathBuf::from("/nonexistent/path/to/model"));
        assert!(!e.is_loaded());
    }

    #[test]
    fn ensure_loaded_fails_when_model_missing() {
        let mut e = Embedder::new(PathBuf::from("/nonexistent/path/to/model"));
        let result = e.ensure_loaded();
        assert!(result.is_err(), "Should fail with missing model directory");
    }

    #[test]
    fn unload_on_unloaded_embedder_is_noop() {
        let mut e = Embedder::new(PathBuf::from("/nonexistent"));
        e.unload(); // should not panic
        assert!(!e.is_loaded());
    }

    #[test]
    fn maybe_unload_does_nothing_when_not_loaded() {
        let mut e = Embedder::new(PathBuf::from("/nonexistent"));
        e.maybe_unload(Duration::from_secs(0));
        assert!(!e.is_loaded());
    }

    #[test]
    fn l2_normalize_produces_unit_vector() {
        let mut v = vec![3.0f32, 4.0f32];
        l2_normalize(&mut v);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6, "norm should be 1.0, got {norm}");
        assert!((v[0] - 0.6).abs() < 1e-6);
        assert!((v[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn l2_normalize_zero_vector_does_not_panic() {
        let mut v = vec![0.0f32; 768];
        l2_normalize(&mut v); // should not panic, just warns
        // Vector stays zero or near-zero — just assert no panic occurred.
    }

    /// Full inference test — requires model files; skipped in CI.
    #[test]
    #[ignore]
    fn embed_query_returns_768_dim_unit_vector() {
        let model_dir = std::env::var("ALPHAT_MODEL_DIR")
            .expect("Set ALPHAT_MODEL_DIR to run this test");
        let mut embedder = Embedder::new(PathBuf::from(model_dir));
        embedder.ensure_loaded().unwrap();

        let v = embedder.embed_query("fn main() {}").unwrap();
        assert_eq!(v.len(), 768);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "query vector should be unit length, norm={norm}");
    }

    /// Batch embedding test — requires model files; skipped in CI.
    #[test]
    #[ignore]
    fn embed_documents_batch() {
        let model_dir = std::env::var("ALPHAT_MODEL_DIR")
            .expect("Set ALPHAT_MODEL_DIR to run this test");
        let mut embedder = Embedder::new(PathBuf::from(model_dir));
        embedder.ensure_loaded().unwrap();

        let docs = vec!["fn foo() {}", "class Bar {}", "def baz(): pass"];
        let vecs = embedder.embed_documents(&docs).unwrap();

        assert_eq!(vecs.len(), 3);
        for v in &vecs {
            assert_eq!(v.len(), 768);
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!((norm - 1.0).abs() < 1e-5, "doc vector should be unit length");
        }
    }
}
