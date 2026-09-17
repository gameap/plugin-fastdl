.PHONY: all build frontend wasm test lint lint-scripts
all: build
build: frontend wasm
frontend:
	cd frontend && npm ci && npm run build
wasm:
	cargo build --target wasm32-wasip1 --release
	cp target/wasm32-wasip1/release/fastdl.wasm fastdl.wasm
test:
	cargo test
	cd frontend && npm test
lint: lint-scripts
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo clippy --target wasm32-wasip1 -- -D warnings
	cd frontend && npm run typecheck
lint-scripts:
	@command -v shellcheck >/dev/null 2>&1 \
		&& shellcheck scripts/install-linux.sh \
		|| echo "shellcheck not installed, skipping scripts/install-linux.sh"
	bash -n scripts/install-linux.sh
