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
