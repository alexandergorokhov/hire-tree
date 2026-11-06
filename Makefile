# Hire Tree. Every target does real work, or prints "not built yet" and
# exits 1. Nothing here fakes success. Nothing here touches mainnet.

SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c
.DEFAULT_GOAL := help

PROP_CASES ?= 10000
OUT := out
CONTRACTS := job factory pool mock-usdc mock-outlayer
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
	cd agents && cargo test --locked --offline --lib --bins

.PHONY: lint
lint: ## rustfmt check + clippy -D warnings + every test the specs name exists
	cargo fmt --all -- --check
	cd agents && cargo fmt -- --check
	cargo clippy --workspace --all-targets --locked --offline -- -D warnings
	cd agents && cargo clippy --all-targets --locked --offline -- -D warnings
	scripts/check-spec-tests.sh

# Plain cargo needs --cfg near (what cargo-near sets). Without it near-sdk's
# panic_str aborts and every on-chain error reads only "unreachable".
.PHONY: build
build: ## Build the contract wasm into out/ with SHA256SUMS (non-reproducible for now)
	@mkdir -p $(OUT)
	@if command -v cargo-near >/dev/null; then \
	  for c in $(CONTRACTS); do \
	    (cd contracts/$$c && cargo near build non-reproducible-wasm --locked --out-dir ../../$(OUT)); \
	  done; \
	else \
	  echo "cargo-near not installed: plain cargo wasm build (no ABI, no wasm-opt)" >&2; \
	  RUSTFLAGS='--cfg near' cargo build --offline --locked --release --target wasm32-unknown-unknown -p hire-tree-job -p hire-tree-factory -p hire-tree-pool -p mock-usdc -p mock-outlayer; \
	  for w in hire_tree_job hire_tree_factory hire_tree_pool mock_usdc mock_outlayer; do cp target/wasm32-unknown-unknown/release/$$w.wasm $(OUT)/; done; \
	  if command -v wasm-opt >/dev/null; then for w in $(OUT)/*.wasm; do \
	    wasm-opt -Oz --strip-debug --strip-producers --enable-bulk-memory --enable-sign-ext --enable-mutable-globals \
	      --enable-nontrapping-float-to-int $$w -o $$w; done; \
	  else echo "wasm-opt not found: the job code stays ~500 KB (a global deploy burns ~51 NEAR; ~27 after wasm-opt)" >&2; fi; \
	fi
	cd $(OUT) && sha256sum *.wasm | tee SHA256SUMS
	@echo "build: non-reproducible wasm in $(OUT)/ (the Docker reproducible build is not built yet)" >&2

# neard raises its open-file limit to 65535 at start and exits if it cannot.
# Where the hard limit is lower and cannot be raised (some containers), a
# tiny LD_PRELOAD shim clamps that one request to the hard limit.
.PHONY: sandbox
sandbox: build ## near-sandbox tests: G1, G4, tree fold, the agent story with a mock model and from a cassette (about 20 min)
	@cd agents && \
	if ulimit -n 65535 2>/dev/null; then pre=""; else \
	  echo "sandbox: open-file hard limit is $$(ulimit -Hn), neard asks for 65535; loading fdshim" >&2; \
	  mkdir -p target && cc -shared -fPIC -O2 -o target/fdshim.so fdshim/fdshim.c -ldl; \
	  pre="$$PWD/target/fdshim.so"; \
	fi; \
	LD_PRELOAD="$$pre" JOB_WASM=$(CURDIR)/$(OUT)/hire_tree_job.wasm USDC_WASM=$(CURDIR)/$(OUT)/mock_usdc.wasm \
	  FACTORY_WASM=$(CURDIR)/$(OUT)/hire_tree_factory.wasm OUTLAYER_WASM=$(CURDIR)/$(OUT)/mock_outlayer.wasm POOL_WASM=$(CURDIR)/$(OUT)/hire_tree_pool.wasm \
	  cargo test --locked -- --test-threads=1 --nocapture

# The node shim and wasm paths every local-node command needs.
define SBX
	cd agents && \
	if ulimit -n 65535 2>/dev/null; then pre=""; else \
	  mkdir -p target && cc -shared -fPIC -O2 -o target/fdshim.so fdshim/fdshim.c -ldl; \
	  pre="$$PWD/target/fdshim.so"; \
	fi; \
	export LD_PRELOAD="$$pre" JOB_WASM=$(CURDIR)/$(OUT)/hire_tree_job.wasm USDC_WASM=$(CURDIR)/$(OUT)/mock_usdc.wasm \
	  FACTORY_WASM=$(CURDIR)/$(OUT)/hire_tree_factory.wasm OUTLAYER_WASM=$(CURDIR)/$(OUT)/mock_outlayer.wasm POOL_WASM=$(CURDIR)/$(OUT)/hire_tree_pool.wasm;
endef

HT = cargo run --quiet --locked --bin hiretree --
CASSETTE ?= $(CURDIR)/demo/cassette.json

.PHONY: mcp-setup
mcp-setup: build ## Build what the MCP playground needs (.mcp.json runs scripts/mcp-local.sh)
	cd agents && cargo build --locked --bin hiretree

.PHONY: mcp-test
mcp-test: mcp-setup ## Two MCP client sessions: the guide's walk-through, and a guest agent competing in the open market (about 5 min)
	cd agents && cargo test --locked --test judge -- --nocapture --test-threads=1

.PHONY: demo
demo: build ## The agent story on a local NEAR node (mock model, free); writes web/scene.json (about 4 min)
	@$(SBX) $(HT) demo --llm mock --scene $(CURDIR)/web/scene.json

.PHONY: demo-replay
demo-replay: build ## The agent story replayed from the recorded NEAR AI cassette (free, offline)
	@test -s $(CASSETTE) || { echo "$(CASSETTE) missing: make record-cassette (needs NEARAI_API_KEY)" >&2; exit 1; }
	@$(SBX) $(HT) demo --llm replay:$(CASSETTE) --scene $(CURDIR)/web/scene.json

.PHONY: web
web: ## Serve the tree page on http://localhost:8080 (reads web/scene.json)
	@test -s web/scene.json || { echo "web/scene.json missing: run make demo first" >&2; exit 1; }
	cd web && python3 -m http.server 8080

.PHONY: nearai-check
nearai-check: ## [NEAR AI key] three small signed calls: receipt format, judge score, planner JSON; GO or NO-GO
	@cd agents && $(HT) nearai-check

.PHONY: record-cassette
record-cassette: build ## [NEAR AI key] run the story on a local node with real NEAR AI and save every signed answer (~55 calls, ~$$0.02)
	@mkdir -p $(dir $(CASSETTE))
	@rm -f $(CASSETTE).new
	@$(SBX) $(HT) demo --llm record:$(CASSETTE).new --scene $(CURDIR)/web/scene.json
	@cd agents && $(HT) verify-cassette $(CASSETTE).new
	@mv $(CASSETTE).new $(CASSETTE) && echo "cassette saved: $(CASSETTE); check it replays with make demo-replay"

.PHONY: verify-cassette
verify-cassette: ## Re-check every NEAR AI signature in the cassette (offline)
	@cd agents && $(HT) verify-cassette $(CASSETTE)

.PHONY: testnet-dryrun
testnet-dryrun: build ## The testnet code path (derived keys, tight funding, rerun reuse, real 5-min waits) against a local node; prints NEAR spent (~25 min)
	@$(SBX) cargo test --locked --test agents testnet_path_dry_run_on_a_local_node -- --ignored --nocapture 2>&1 | grep -E "global job code|operator spent|test result|panicked|error"

.PHONY: testnet-all
testnet-all: build ## [testnet: HT_OPERATOR, HT_OPERATOR_KEY, NEARAI_API_KEY] deploy (or reuse) and run the agent story with live NEAR AI; rerun resumes
	@cd agents && $(HT) demo --net testnet --llm live --scene $(CURDIR)/web/scene-testnet.json
