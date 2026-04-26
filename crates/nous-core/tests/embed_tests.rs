use nous_core::embed::{EmbeddingBackend, MockEmbedding, OnnxBackend};

#[test]
fn embed_one_returns_correct_dimensions() {
    let mock = MockEmbedding::new(64);
    let vec = mock.embed_one("hello").unwrap();
    assert_eq!(vec.len(), 64);
}

#[test]
fn embed_batch_returns_correct_count() {
    let mock = MockEmbedding::new(64);
    let vecs = mock.embed(&["a", "b"]).unwrap();
    assert_eq!(vecs.len(), 2);
    assert_eq!(vecs[0].len(), 64);
    assert_eq!(vecs[1].len(), 64);
}

#[test]
fn embed_is_deterministic() {
    let mock = MockEmbedding::new(64);
    let v1 = mock.embed_one("hello world").unwrap();
    let v2 = mock.embed_one("hello world").unwrap();
    assert_eq!(v1, v2);
}

#[test]
fn different_inputs_produce_different_vectors() {
    let mock = MockEmbedding::new(64);
    let v1 = mock.embed_one("alpha").unwrap();
    let v2 = mock.embed_one("beta").unwrap();
    assert_ne!(v1, v2);
}

#[test]
fn vectors_are_unit_normalized() {
    let mock = MockEmbedding::new(64);
    for text in &["hello", "world", "foo bar baz", ""] {
        let vec = mock.embed_one(text).unwrap();
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 0.01,
            "expected unit norm, got {norm} for input '{text}'"
        );
    }
}

#[test]
fn trait_is_object_safe() {
    let mock = MockEmbedding::new(32);
    let boxed: Box<dyn EmbeddingBackend> = Box::new(mock);
    let vec = boxed.embed_one("test").unwrap();
    assert_eq!(vec.len(), 32);
}

#[test]
fn embed_one_delegates_to_embed() {
    let mock = MockEmbedding::new(64);
    let via_one = mock.embed_one("delegate").unwrap();
    let via_batch = mock.embed(&["delegate"]).unwrap();
    assert_eq!(via_one, via_batch[0]);
}

#[test]
fn model_id_and_metadata() {
    let mock = MockEmbedding::new(128);
    assert_eq!(mock.model_id(), "mock");
    assert_eq!(mock.dimensions(), 128);
    assert!(mock.max_tokens() > 0);
}

// --- OnnxBackend tests (require model download) ---

#[test]
#[ignore]
fn onnx_embed_one_returns_correct_dimensions() {
    let backend = OnnxBackend::builder()
        .model("Qwen/Qwen3-Embedding-0.6B")
        .variant("onnx/model.onnx")
        .build()
        .expect("failed to build OnnxBackend");
    let vec = backend.embed_one("test").unwrap();
    assert_eq!(vec.len(), backend.dimensions());
}

#[test]
#[ignore]
fn onnx_embed_batch_returns_correct_count() {
    let backend = OnnxBackend::builder()
        .model("Qwen/Qwen3-Embedding-0.6B")
        .variant("onnx/model.onnx")
        .build()
        .expect("failed to build OnnxBackend");
    let vecs = backend.embed(&["hello", "world", "foo bar"]).unwrap();
    assert_eq!(vecs.len(), 3);
    for v in &vecs {
        assert_eq!(v.len(), backend.dimensions());
    }
}

#[test]
#[ignore]
fn onnx_vectors_are_l2_normalized() {
    let backend = OnnxBackend::builder()
        .model("Qwen/Qwen3-Embedding-0.6B")
        .variant("onnx/model.onnx")
        .build()
        .expect("failed to build OnnxBackend");
    for text in &["hello", "world", "foo bar baz"] {
        let vec = backend.embed_one(text).unwrap();
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 0.01,
            "expected unit norm, got {norm} for input '{text}'"
        );
    }
}
