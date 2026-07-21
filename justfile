build:
    cargo build --release

test:
    cargo test --lib

lint:
    cargo clippy -- -D warnings

fix:
    cargo clippy --fix --allow-dirty

ci: lint test build

run:
    cargo run

coverage:
    cargo tarpaulin --out Html
