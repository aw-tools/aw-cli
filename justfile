default: ci

fmt:
    dprint check

fmt-fix:
    dprint fmt

clippy:
    cargo clippy --all-targets -- -D warnings

test:
    cargo test

e2e: build
    tests/e2e.sh

build:
    cargo build

deny:
    cargo deny check

doc:
    cargo doc --no-deps

install:
    cargo install --path .

ci: fmt clippy test doc deny e2e

setup:
    git config core.hooksPath .githooks
