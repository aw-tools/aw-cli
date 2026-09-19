# List available recipes
default:
    @just --list

# Check formatting
fmt:
    dprint check

# Fix formatting
fmt-fix:
    dprint fmt

# Run clippy lints
clippy:
    cargo clippy --all-targets -- -D warnings

# Run tests
test:
    cargo test

# Run the end-to-end shell harness
e2e: build
    tests/e2e.sh

# Build the binary
build:
    cargo build

# Run dependency audits
deny:
    cargo deny check

# Build documentation
doc:
    cargo doc --no-deps

# Install aw to ~/.cargo/bin
install:
    cargo install --path .

# Lint the shell scripts
shellcheck:
    shellcheck tests/*.sh

# Lint the GitHub Actions workflows
actionlint:
    actionlint

# Run the full CI pipeline
ci: fmt clippy test doc deny e2e shellcheck actionlint

# Configure git hooks (run once after clone)
setup:
    git config core.hooksPath .githooks
