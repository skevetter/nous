mod config;

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, ValueEnum)]
enum Transport {
    Stdio,
    Http,
}

#[derive(Debug, Parser)]
#[command(name = "nous-mcp", about = "Nous MCP server and management CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve {
        #[arg(long, default_value = "stdio")]
        transport: Transport,
        #[arg(long, default_value_t = 8377)]
        port: u16,
    },
    ReEmbed {
        #[arg(long)]
        model: String,
        #[arg(long)]
        variant: Option<String>,
    },
    ReClassify {
        #[arg(long)]
        since: Option<String>,
    },
    Category(CategoryCmd),
    Export {
        #[arg(long, default_value = "json")]
        format: String,
    },
    Import {
        file: PathBuf,
    },
    RotateKey {
        #[arg(long)]
        new_key_file: Option<PathBuf>,
    },
    Status,
}

#[derive(Debug, Parser)]
struct CategoryCmd {
    #[command(subcommand)]
    command: CategorySubcommand,
}

#[derive(Debug, Subcommand)]
enum CategorySubcommand {
    List {
        #[arg(long)]
        source: Option<String>,
    },
    Add {
        name: String,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
}

fn main() {
    let _cli = Cli::parse();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_defaults() {
        let cli = Cli::try_parse_from(["nous-mcp", "serve"]).unwrap();
        match cli.command {
            Command::Serve { transport, port } => {
                assert!(matches!(transport, Transport::Stdio));
                assert_eq!(port, 8377);
            }
            _ => panic!("expected Serve"),
        }
    }

    #[test]
    fn serve_explicit_http_and_port() {
        let cli =
            Cli::try_parse_from(["nous-mcp", "serve", "--transport", "http", "--port", "9000"])
                .unwrap();
        match cli.command {
            Command::Serve { transport, port } => {
                assert!(matches!(transport, Transport::Http));
                assert_eq!(port, 9000);
            }
            _ => panic!("expected Serve"),
        }
    }

    #[test]
    fn re_embed_with_model() {
        let cli = Cli::try_parse_from(["nous-mcp", "re-embed", "--model", "org/repo"]).unwrap();
        match cli.command {
            Command::ReEmbed { model, variant } => {
                assert_eq!(model, "org/repo");
                assert!(variant.is_none());
            }
            _ => panic!("expected ReEmbed"),
        }
    }

    #[test]
    fn re_embed_with_variant() {
        let cli = Cli::try_parse_from([
            "nous-mcp",
            "re-embed",
            "--model",
            "org/repo",
            "--variant",
            "q4",
        ])
        .unwrap();
        match cli.command {
            Command::ReEmbed { model, variant } => {
                assert_eq!(model, "org/repo");
                assert_eq!(variant.as_deref(), Some("q4"));
            }
            _ => panic!("expected ReEmbed"),
        }
    }

    #[test]
    fn re_classify_no_args() {
        let cli = Cli::try_parse_from(["nous-mcp", "re-classify"]).unwrap();
        match cli.command {
            Command::ReClassify { since } => assert!(since.is_none()),
            _ => panic!("expected ReClassify"),
        }
    }

    #[test]
    fn re_classify_with_since() {
        let cli =
            Cli::try_parse_from(["nous-mcp", "re-classify", "--since", "2024-01-01"]).unwrap();
        match cli.command {
            Command::ReClassify { since } => {
                assert_eq!(since.as_deref(), Some("2024-01-01"));
            }
            _ => panic!("expected ReClassify"),
        }
    }

    #[test]
    fn category_add() {
        let cli = Cli::try_parse_from(["nous-mcp", "category", "add", "testing"]).unwrap();
        match cli.command {
            Command::Category(CategoryCmd {
                command:
                    CategorySubcommand::Add {
                        name,
                        parent,
                        description,
                    },
            }) => {
                assert_eq!(name, "testing");
                assert!(parent.is_none());
                assert!(description.is_none());
            }
            _ => panic!("expected Category Add"),
        }
    }

    #[test]
    fn category_add_with_parent_and_description() {
        let cli = Cli::try_parse_from([
            "nous-mcp",
            "category",
            "add",
            "unit-tests",
            "--parent",
            "testing",
            "--description",
            "Unit test category",
        ])
        .unwrap();
        match cli.command {
            Command::Category(CategoryCmd {
                command:
                    CategorySubcommand::Add {
                        name,
                        parent,
                        description,
                    },
            }) => {
                assert_eq!(name, "unit-tests");
                assert_eq!(parent.as_deref(), Some("testing"));
                assert_eq!(description.as_deref(), Some("Unit test category"));
            }
            _ => panic!("expected Category Add"),
        }
    }

    #[test]
    fn category_list_no_filter() {
        let cli = Cli::try_parse_from(["nous-mcp", "category", "list"]).unwrap();
        match cli.command {
            Command::Category(CategoryCmd {
                command: CategorySubcommand::List { source },
            }) => {
                assert!(source.is_none());
            }
            _ => panic!("expected Category List"),
        }
    }

    #[test]
    fn category_list_with_source() {
        let cli =
            Cli::try_parse_from(["nous-mcp", "category", "list", "--source", "manual"]).unwrap();
        match cli.command {
            Command::Category(CategoryCmd {
                command: CategorySubcommand::List { source },
            }) => {
                assert_eq!(source.as_deref(), Some("manual"));
            }
            _ => panic!("expected Category List"),
        }
    }

    #[test]
    fn export_default_format() {
        let cli = Cli::try_parse_from(["nous-mcp", "export"]).unwrap();
        match cli.command {
            Command::Export { format } => assert_eq!(format, "json"),
            _ => panic!("expected Export"),
        }
    }

    #[test]
    fn import_file() {
        let cli = Cli::try_parse_from(["nous-mcp", "import", "/tmp/data.json"]).unwrap();
        match cli.command {
            Command::Import { file } => {
                assert_eq!(file, PathBuf::from("/tmp/data.json"));
            }
            _ => panic!("expected Import"),
        }
    }

    #[test]
    fn rotate_key_no_file() {
        let cli = Cli::try_parse_from(["nous-mcp", "rotate-key"]).unwrap();
        match cli.command {
            Command::RotateKey { new_key_file } => assert!(new_key_file.is_none()),
            _ => panic!("expected RotateKey"),
        }
    }

    #[test]
    fn rotate_key_with_file() {
        let cli = Cli::try_parse_from(["nous-mcp", "rotate-key", "--new-key-file", "/tmp/key.bin"])
            .unwrap();
        match cli.command {
            Command::RotateKey { new_key_file } => {
                assert_eq!(new_key_file, Some(PathBuf::from("/tmp/key.bin")));
            }
            _ => panic!("expected RotateKey"),
        }
    }

    #[test]
    fn status_command() {
        let cli = Cli::try_parse_from(["nous-mcp", "status"]).unwrap();
        assert!(matches!(cli.command, Command::Status));
    }

    #[test]
    fn invalid_subcommand_errors() {
        let result = Cli::try_parse_from(["nous-mcp", "nonexistent"]);
        assert!(result.is_err());
    }

    #[test]
    fn re_embed_missing_model_errors() {
        let result = Cli::try_parse_from(["nous-mcp", "re-embed"]);
        assert!(result.is_err());
    }

    #[test]
    fn import_missing_file_errors() {
        let result = Cli::try_parse_from(["nous-mcp", "import"]);
        assert!(result.is_err());
    }
}
