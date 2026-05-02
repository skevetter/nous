use std::path::{Path, PathBuf};

use clap::Subcommand;

#[derive(Subcommand)]
pub enum SkillCommands {
    /// List available local skills
    List,
}

pub async fn run(cmd: SkillCommands, _port: Option<u16>) {
    if let Err(e) = execute(cmd).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

pub fn list_skills_in_dir(dir: &Path) -> Result<Vec<(String, PathBuf)>, std::io::Error> {
    let mut skills: Vec<(String, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                skills.push((stem.to_string(), path));
            }
        }
    }
    skills.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(skills)
}

async fn execute(cmd: SkillCommands) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        SkillCommands::List => {
            let skills_dir = dirs::config_dir()
                .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
                .unwrap_or_else(|| PathBuf::from("."))
                .join("nous")
                .join("skills");

            if !skills_dir.exists() {
                println!("No skills directory found at {}", skills_dir.display());
                println!("Create it and add .md files to define skills.");
                return Ok(());
            }

            let skills = list_skills_in_dir(&skills_dir)?;

            if skills.is_empty() {
                println!("No skills found in {}", skills_dir.display());
                println!("Add .md files to define skills.");
            } else {
                println!("LOCAL SKILLS ({})", skills_dir.display());
                for (name, path) in &skills {
                    println!("  {}  ({})", name, path.display());
                }
                println!("\n{} skill(s) found.", skills.len());
            }
        }
    }
    Ok(())
}
