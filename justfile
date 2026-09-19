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

# Lint the human-facing documents
vale:
    vale README.md CHANGELOG.md docs

# Check every document against its word budget
budget:
    bin/lint-docs-budget

# Prove every prose rule fires on its fixture
test-lint:
    bin/test-lint-docs

# Lint the shell scripts
shellcheck:
    shellcheck bin/* tests/*.sh

# Lint the GitHub Actions workflows
actionlint:
    actionlint

# Run the full CI pipeline
ci: fmt clippy test doc deny e2e vale budget test-lint shellcheck actionlint

# Configure git hooks (run once after clone)
setup:
    git config core.hooksPath .githooks
