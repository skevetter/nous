use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use nous_core::embed::{EmbeddingBackend, OnnxBackend};
use serde::Serialize;

const MODEL: &str = "sentence-transformers/all-MiniLM-L6-v2";
const VARIANT: &str = "onnx/model.onnx";

const TEST_STRINGS: &[&str] = &[
    // Identical match baseline
    "Rust programming language",
    // Semantic similarity cluster
    "Systems programming with Rust",
    "Writing code in Rust",
    // Dissimilar content
    "Chocolate cake recipe",
    "The weather in Tokyo",
    // Edge cases
    "",
    "hi",
    "Rust is a multi-paradigm, general-purpose programming language that emphasizes performance, type safety, and concurrency. It enforces memory safety without a garbage collector.",
];

#[derive(Serialize)]
struct Fixture {
    model: String,
    variant: String,
    dimension: usize,
    vectors: BTreeMap<String, Vec<f32>>,
}

fn main() {
    eprintln!("Building OnnxBackend with {MODEL} ({VARIANT})...");
    let backend = OnnxBackend::builder()
        .model(MODEL)
        .variant(VARIANT)
        .build()
        .expect("failed to build OnnxBackend");

    eprintln!(
        "Model loaded: {} dimensions, {} max tokens",
        backend.dimensions(),
        backend.max_tokens()
    );

    let texts: Vec<&str> = TEST_STRINGS.to_vec();
    eprintln!("Embedding {} strings...", texts.len());
    let embeddings = backend.embed(&texts).expect("failed to embed texts");

    let mut vectors = BTreeMap::new();
    for (text, vec) in texts.iter().zip(embeddings) {
        vectors.insert(text.to_string(), vec);
    }

    let fixture = Fixture {
        model: MODEL.to_string(),
        variant: VARIANT.to_string(),
        dimension: backend.dimensions(),
        vectors,
    };

    let json = serde_json::to_string_pretty(&fixture).expect("failed to serialize fixture");

    let out_dir: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures"]
        .iter()
        .collect();
    fs::create_dir_all(&out_dir).expect("failed to create fixtures directory");

    let out_path = out_dir.join("embedding_vectors.json");
    fs::write(&out_path, &json).expect("failed to write fixture file");

    eprintln!(
        "Wrote {} vectors to {}",
        fixture.vectors.len(),
        out_path.display()
    );
}
