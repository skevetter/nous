use clap::Subcommand;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum ModelCommands {
    /// Download the embedding model and tokenizer from HuggingFace
    Download,
}

const MODEL_URL: &str =
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/onnx/model.onnx";
const TOKENIZER_URL: &str =
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/tokenizer.json";

const EXPECTED_MODEL_SIZE_MIN: u64 = 80_000_000;
const EXPECTED_TOKENIZER_SIZE_MIN: u64 = 600_000;

pub async fn run(command: ModelCommands) {
    match command {
        ModelCommands::Download => download().await,
    }
}

async fn download() {
    let models_dir = models_dir();
    if let Err(e) = fs::create_dir_all(&models_dir) {
        eprintln!(
            "ERROR: cannot create models directory {}: {e}",
            models_dir.display()
        );
        std::process::exit(1);
    }

    let model_path = models_dir.join("all-MiniLM-L6-v2.onnx");
    let tokenizer_path = models_dir.join("tokenizer.json");

    download_file(
        MODEL_URL,
        &model_path,
        "all-MiniLM-L6-v2.onnx",
        EXPECTED_MODEL_SIZE_MIN,
    )
    .await;
    download_file(
        TOKENIZER_URL,
        &tokenizer_path,
        "tokenizer.json",
        EXPECTED_TOKENIZER_SIZE_MIN,
    )
    .await;

    println!("\nDone. Models are ready at {}", models_dir.display());
}

async fn download_file(url: &str, dest: &PathBuf, name: &str, expected_min_size: u64) {
    if dest.exists() {
        if let Ok(meta) = fs::metadata(dest) {
            if meta.len() >= expected_min_size {
                println!("[SKIP] {name} already exists ({} bytes)", meta.len());
                return;
            }
            println!(
                "[WARN] {name} exists but is too small ({} bytes), re-downloading...",
                meta.len()
            );
        }
    }

    println!("[DOWNLOADING] {name} from HuggingFace...");

    let response = match reqwest::get(url).await {
        Ok(resp) => resp,
        Err(e) => {
            eprintln!("ERROR: failed to fetch {name}: {e}");
            std::process::exit(1);
        }
    };

    if !response.status().is_success() {
        eprintln!("ERROR: HTTP {} when downloading {name}", response.status());
        std::process::exit(1);
    }

    let content_length = response.content_length();
    if let Some(len) = content_length {
        println!("       file size: {:.1} MB", len as f64 / 1_048_576.0);
    }

    let bytes = match response.bytes().await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("ERROR: failed to read response body for {name}: {e}");
            std::process::exit(1);
        }
    };

    let tmp_path = dest.with_extension("tmp");
    let mut file = match fs::File::create(&tmp_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("ERROR: cannot create file {}: {e}", tmp_path.display());
            std::process::exit(1);
        }
    };

    if let Err(e) = file.write_all(&bytes) {
        eprintln!("ERROR: failed to write {name}: {e}");
        let _ = fs::remove_file(&tmp_path);
        std::process::exit(1);
    }

    if let Err(e) = fs::rename(&tmp_path, dest) {
        eprintln!("ERROR: failed to move {name} into place: {e}");
        let _ = fs::remove_file(&tmp_path);
        std::process::exit(1);
    }

    println!("[OK] {name} downloaded ({} bytes)", bytes.len());
}

pub fn models_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".nous")
        .join("models")
}

pub fn check_model_files() -> (bool, bool) {
    let dir = models_dir();
    let model_ok = dir.join("all-MiniLM-L6-v2.onnx").exists();
    let tokenizer_ok = dir.join("tokenizer.json").exists();
    (model_ok, tokenizer_ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_dir_ends_with_nous_models() {
        let dir = models_dir();
        assert!(dir.ends_with(".nous/models"));
    }

    #[test]
    fn check_model_files_returns_bools() {
        let (model, tokenizer) = check_model_files();
        // Just verify the function runs without panic — actual presence depends on host
        let _ = (model, tokenizer);
    }
}
