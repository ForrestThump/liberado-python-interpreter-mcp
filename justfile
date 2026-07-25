build:
    cargo build --release

test:
    cargo test --lib

lint:
    cargo clippy --all-targets -- -D warnings

fix:
    cargo clippy --fix --allow-dirty --allow-staged

fmt:
    cargo fmt --all -- --check

fmt-fix:
    cargo fmt --all

ci: lint fmt test build

run:
    cargo run

run-unsafe:
    LIBERADO_SANDBOX_ENABLED=0 cargo run

coverage:
    cargo llvm-cov --html
    @echo "Coverage report: target/llvm-cov/html/index.html"

coverage-lcov:
    cargo llvm-cov --lcov --output-path lcov.info
