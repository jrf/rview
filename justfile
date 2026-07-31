default:
    @just --list

build:
    cargo build

release:
    cargo build --release

install:
    cargo install --path .

run *args:
    cargo run -- {{args}}

clean:
    cargo clean

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

lint:
    cargo clippy --all-targets -- -D warnings

lint-all:
    cargo clippy --all-features --all-targets -- -D warnings

test:
    cargo test --all-targets

test-all:
    cargo test --all-features --all-targets

check: fmt-check lint test

check-all: fmt-check lint-all test-all
