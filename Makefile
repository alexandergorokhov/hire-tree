# Hire Tree. Every target does real work, or prints "not built yet" and
# exits 1. Nothing here fakes success. Nothing here touches mainnet.

SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c
.DEFAULT_GOAL := help

PROP_CASES ?= 10000
OUT := out
CONTRACTS := job mock-usdc
export CARGO_INCREMENTAL := 0

define not_built
	@echo "not built yet: $(1) (planned: $(2))" >&2; exit 1
endef

.PHONY: help
help: ## List targets
	@grep -E '^[a-zA-Z0-9_-]+:.*## ' $(MAKEFILE_LIST) | sed 's/:.*## /\t/' | sort

.PHONY: setup
setup: ## Print toolchain versions and fetch crates (the only networked step)
	rustc --version
	cargo --version
	cargo fetch --locked
	@command -v cargo-near >/dev/null && cargo near --version || echo "cargo-near not installed: make build falls back to plain cargo" >&2

.PHONY: test
test: ## Unit + property tests, offline (PROP_CASES=10000 by default)
	PROP_CASES=$(PROP_CASES) cargo test --workspace --locked --offline

.PHONY: lint
lint: ## rustfmt check + clippy -D warnings
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --locked --offline -- -D warnings

.PHONY: build
build: ## Build the contract wasm into out/ with SHA256SUMS (non-reproducible for now)
	@mkdir -p $(OUT)
	@if command -v cargo-near >/dev/null; then \
	  for c in $(CONTRACTS); do \
	    (cd contracts/$$c && cargo near build non-reproducible-wasm --locked --out-dir ../../$(OUT)); \
	  done; \
	else \
	  echo "cargo-near not installed: plain cargo wasm build (no ABI, no wasm-opt)" >&2; \
	  cargo build --offline --locked --release --target wasm32-unknown-unknown -p hire-tree-job -p mock-usdc; \
	  cp target/wasm32-unknown-unknown/release/hire_tree_job.wasm target/wasm32-unknown-unknown/release/mock_usdc.wasm $(OUT)/; \
	fi
	cd $(OUT) && sha256sum *.wasm | tee SHA256SUMS
	@echo "build: non-reproducible wasm in $(OUT)/ (the Docker reproducible build is not built yet)" >&2

# ---------------------------------------------------------------- not built yet

.PHONY: sandbox
sandbox: ## (not built yet) near-workspaces integration + attack tests
	$(call not_built,sandbox,day S+1/S+4: tests/sandbox with near-workspaces)

.PHONY: demo
demo: ## (not built yet) offline hermetic demo, diffs fixtures/expected-output.txt
	$(call not_built,demo,day S+5)

.PHONY: web
web: ## (not built yet) static tree page
	$(call not_built,web,day S+5)

.PHONY: testnet-deploy
testnet-deploy: ## (not built yet) [testnet] deploy factory, global job code, mock-usdc
	$(call not_built,testnet-deploy,day S+6)

.PHONY: testnet-demo
testnet-demo: ## (not built yet) [testnet] full run with real agents
	$(call not_built,testnet-demo,day S+6)

.PHONY: record-fixtures
record-fixtures: ## (not built yet) [testnet, NEAR AI key] record 3 NEAR AI Cloud receipts
	$(call not_built,record-fixtures,day S+2 on the owner's PC)
