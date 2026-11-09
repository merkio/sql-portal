.PHONY: fmt lint test coverage run check-config docker

fmt:
	cargo fmt --all

lint:
	cargo fmt --all -- --check
	cargo clippy --all-targets --all-features --locked -- -D warnings

test:
	cargo test --all-features --locked

# Requires: cargo install cargo-llvm-cov
coverage:
	cargo llvm-cov --all-features --locked --ignore-filename-regex '(src/main\.rs|src/testing\.rs)' --summary-only

run:
	cargo run -- --config config/config.demo.yaml

check-config:
	cargo run -- --config config/config.demo.yaml check-config

docker:
	docker build -t script-portal:local .
