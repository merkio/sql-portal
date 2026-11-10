.PHONY: fmt lint test test-docker test-e2e test-docker-clean image-test coverage run check-config docker

# Image used by the end-to-end tests (tests/portal_image.rs).
SCRIPT_PORTAL_IMAGE ?= script-portal:test
export SCRIPT_PORTAL_IMAGE

fmt:
	cargo fmt --all

lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

# Unit and in-process integration tests. No Docker needed.
test:
	cargo test --workspace --locked

# Everything, including tests that start containers (PostgreSQL, ClickHouse,
# the portal image). Requires a Docker-compatible engine.
test-docker: image-test
	cargo test --workspace --locked --features sql-portal-integration-tests/docker

# Only the end-to-end checks of the portal image.
test-e2e: image-test
	cargo test -p sql-portal-integration-tests --locked --features docker --test portal_image

image-test:
	docker build -t $(SCRIPT_PORTAL_IMAGE) .

# Removes containers left behind by a test run that was killed before cleanup.
test-docker-clean:
	docker ps -aq --filter label=org.sql-portal.integration-test=true | xargs -r docker rm -f

# Requires: cargo install cargo-llvm-cov
coverage:
	cargo llvm-cov --workspace --locked --ignore-filename-regex '(main\.rs|testing/|integration-tests/)' --summary-only

run:
	cargo run --bin script-portal -- --config config/config.demo.yaml

check-config:
	cargo run --bin script-portal -- --config config/config.demo.yaml check-config

docker:
	docker build -t script-portal:local .
