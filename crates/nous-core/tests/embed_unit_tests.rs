use std::io::Write;

use tokenizers::Encoding;

fn make_encoding(attention_mask: Vec<u32>) -> Encoding {
    let len = attention_mask.len();
    Encoding::new(
        vec![0u32; len],          // ids
        vec![0u32; len],          // type_ids
        vec![String::new(); len], // tokens
        vec![Some(0u32); len],    // words
        vec![(0, 0); len],        // offsets
        vec![0u32; len],          // special_tokens_mask
        attention_mask,           // attention_mask
        vec![],                   // overflowing
        Default::default(),       // sequence_ranges
    )
}

// --- mean_pool tests ---

#[test]
fn mean_pool_known_tensor() {
    // 2 samples × 3 tokens × 4 hidden dims
    // Sample 0: mask [1, 1, 0] → average of tokens 0 and 1
    // Sample 1: mask [1, 1, 1] → average of all 3 tokens
    let encodings = vec![make_encoding(vec![1, 1, 0]), make_encoding(vec![1, 1, 1])];

    #[rustfmt::skip]
    let data: Vec<f32> = vec![
        // sample 0
        1.0, 2.0, 3.0, 4.0,   // token 0
        5.0, 6.0, 7.0, 8.0,   // token 1
        9.0, 9.0, 9.0, 9.0,   // token 2 (masked out)
        // sample 1
        3.0, 6.0, 9.0, 12.0,  // token 0
        0.0, 0.0, 0.0, 0.0,   // token 1
        6.0, 3.0, 0.0, 3.0,   // token 2
    ];

    let dims = vec![2, 3, 4];
    let result = nous_core::embed::mean_pool(&encodings, &data, &dims);

    assert_eq!(result.len(), 2);
    assert_eq!(result[0].len(), 4);
    assert_eq!(result[1].len(), 4);

    // Sample 0: mean of token 0 and 1 → (1+5)/2=3, (2+6)/2=4, (3+7)/2=5, (4+8)/2=6
    let expected_0 = [3.0, 4.0, 5.0, 6.0];
    for (a, b) in result[0].iter().zip(expected_0.iter()) {
        assert!((a - b).abs() < 1e-6, "sample 0: expected {b}, got {a}");
    }

    // Sample 1: mean of all 3 → (3+0+6)/3=3, (6+0+3)/3=3, (9+0+0)/3=3, (12+0+3)/3=5
    let expected_1 = [3.0, 3.0, 3.0, 5.0];
    for (a, b) in result[1].iter().zip(expected_1.iter()) {
        assert!((a - b).abs() < 1e-6, "sample 1: expected {b}, got {a}");
    }
}

#[test]
fn mean_pool_all_pad_returns_zeros() {
    let encodings = vec![make_encoding(vec![0, 0, 0])];
    let data = vec![1.0f32; 9]; // 1×3×3
    let dims = vec![1, 3, 3];

    let result = nous_core::embed::mean_pool(&encodings, &data, &dims);
    assert_eq!(result.len(), 1);
    assert!(
        result[0].iter().all(|&v| v == 0.0),
        "all-pad should produce zero vector, got {:?}",
        result[0]
    );
}

// --- detect_max_tokens tests ---

#[test]
fn detect_max_tokens_reads_model_max_length() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokenizer.json");
    let mut f = std::fs::File::create(&path).unwrap();
    write!(f, r#"{{"model_max_length": 512}}"#).unwrap();
    drop(f);

    let result = nous_core::embed::detect_max_tokens(&path).unwrap();
    assert_eq!(result, 512);
}

#[test]
fn detect_max_tokens_fallback_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokenizer.json");
    let mut f = std::fs::File::create(&path).unwrap();
    write!(f, r#"{{}}"#).unwrap();
    drop(f);

    let result = nous_core::embed::detect_max_tokens(&path).unwrap();
    assert_eq!(result, 8192);
}

#[test]
fn detect_max_tokens_fallback_on_float_sentinel() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokenizer.json");
    let mut f = std::fs::File::create(&path).unwrap();
    // 1e30 is a common sentinel value — as_u64() returns None for floats
    write!(f, r#"{{"model_max_length": 1e30}}"#).unwrap();
    drop(f);

    let result = nous_core::embed::detect_max_tokens(&path).unwrap();
    assert_eq!(result, 8192);
}
