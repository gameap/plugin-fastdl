POWERSHELL ?= pwsh

.PHONY: all build frontend wasm test test-scripts lint lint-scripts
all: build
build: frontend wasm
frontend:
	cd frontend && npm ci && npm run build
wasm:
	cargo build --target wasm32-wasip1 --release
	cp target/wasm32-wasip1/release/fastdl.wasm fastdl.wasm
test: test-scripts
	cargo test
	cd frontend && npm test
test-scripts:
	python3 scripts/tests/test_release_resolution.py
	@if command -v "$(POWERSHELL)" >/dev/null 2>&1; then \
		"$(POWERSHELL)" -NoProfile -NonInteractive -File scripts/tests/release-resolution.ps1; \
	else \
		echo "$(POWERSHELL) not installed, skipping Windows installer tests"; \
	fi
lint: lint-scripts
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo clippy --target wasm32-wasip1 -- -D warnings
	cd frontend && npm run typecheck
lint-scripts:
	@if command -v shellcheck >/dev/null 2>&1; then \
		shellcheck scripts/install-linux.sh; \
	else \
		echo "shellcheck not installed, skipping scripts/install-linux.sh"; \
	fi
	bash -n scripts/install-linux.sh
