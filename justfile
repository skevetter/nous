set shell := ["bash", "-euo", "pipefail", "-c"]

export CARGO_BUILD_JOBS := "1"

# list all recipes
default:
    @just --list

# build all workspace crates
build:
    cargo build

# run all workspace tests
test:
    cargo test

# run clippy with warnings as errors
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# format all code
fmt:
    cargo fmt --all

# check formatting without modifying files
fmt-check:
    cargo fmt --all -- --check

# run end-to-end tests (builds first)
e2e: build
    cargo test -p e2e-tests

# start the MCP server
serve-mcp:
    cargo run -p nous-mcp -- serve

# start the OTLP receiver
serve-otlp:
    cargo run -p nous-otlp -- serve

# remove build artifacts
clean:
    cargo clean

# run CI checks: format, lint, test
check: fmt-check clippy test
