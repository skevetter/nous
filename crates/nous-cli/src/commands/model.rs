use clap::Subcommand;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum ModelCommands {
    /// Download the embedding model and tokenizer from `HuggingFace`
    Download,
}

const MODEL_URL: &str =
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/onnx/model.onnx";
const TOKENIZER_URL: &str =
    "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/tokenizer.json";

const EXPECTED_MODEL_SIZE_MIN: u64 = 80_000_000;
const EXPECTED_TOKENIZER_SIZE_MIN: u64 = 600_000;

pub async fn run(command: ModelCommands, _port: Option<u16>) {
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

fn check_existing_file(dest: &PathBuf, name: &str, expected_min_size: u64) -> bool {
    if !dest.exists() {
        return false;
    }
    match fs::metadata(dest) {
        Ok(meta) if meta.len() >= expected_min_size => {
            println!("[SKIP] {name} already exists ({} bytes)", meta.len());
            true
        }
        Ok(meta) => {
            println!(
                "[WARN] {name} exists but is too small ({} bytes), re-downloading...",
                meta.len()
            );
            false
        }
        Err(_) => false,
    }
}

async fn fetch_url(url: &str, name: &str) -> Vec<u8> {
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
        // Divide first to bring value into u32 range; model files are well under 4 TB
        let mb = f64::from(u32::try_from(len / 1_048_576).unwrap_or(u32::MAX));
        let frac = f64::from(u32::try_from(len % 1_048_576).unwrap_or(0)) / 1_048_576.0;
        println!("       file size: {:.1} MB", mb + frac);
    }

    match response.bytes().await {
        Ok(b) => b.to_vec(),
        Err(e) => {
            eprintln!("ERROR: failed to read response body for {name}: {e}");
            std::process::exit(1);
        }
    }
}

fn write_file_atomic(dest: &PathBuf, name: &str, bytes: &[u8]) {
    let tmp_path = dest.with_extension("tmp");
    let mut file = match fs::File::create(&tmp_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("ERROR: cannot create file {}: {e}", tmp_path.display());
            std::process::exit(1);
        }
    };

    if let Err(e) = file.write_all(bytes) {
        eprintln!("ERROR: failed to write {name}: {e}");
        let _ = fs::remove_file(&tmp_path);
        std::process::exit(1);
    }

    if let Err(e) = fs::rename(&tmp_path, dest) {
        eprintln!("ERROR: failed to move {name} into place: {e}");
        let _ = fs::remove_file(&tmp_path);
        std::process::exit(1);
    }
}

async fn download_file(url: &str, dest: &PathBuf, name: &str, expected_min_size: u64) {
    if check_existing_file(dest, name, expected_min_size) {
        return;
    }

    println!("[DOWNLOADING] {name} from HuggingFace...");
    let bytes = fetch_url(url, name).await;
    write_file_atomic(dest, name, &bytes);
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
