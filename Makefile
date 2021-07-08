# This supports environments where $HOME/.cargo/env has not been sourced (CI, CLion Makefile runner)
CARGO  = $(or $(shell which cargo),  $(HOME)/.cargo/bin/cargo)
RUSTUP = $(or $(shell which rustup), $(HOME)/.cargo/bin/rustup)
NPM    = $(or $(shell which npm),    /usr/bin/npm)

RUST_TOOLCHAIN := $(shell cat rust-toolchain)

CARGO_OPTS := --locked
CARGO := $(CARGO) $(CARGO_TOOLCHAIN) $(CARGO_OPTS)

DISABLE_LOGGING = RUST_LOG=MatchesNothing

# Rust Contracts
# Directory names should match crate names
BENCH       = $(shell find ./smart_contracts/contracts/bench     -mindepth 1 -maxdepth 1 -type d -exec basename {} \;)
CLIENT      = $(shell find ./smart_contracts/contracts/client    -mindepth 1 -maxdepth 1 -type d -exec basename {} \;)
EXPLORER    = $(shell find ./smart_contracts/contracts/explorer  -mindepth 1 -maxdepth 1 -type d -exec basename {} \;)
PROFILING   = $(shell find ./smart_contracts/contracts/profiling -mindepth 1 -maxdepth 1 -type d -exec basename {} \;)
SRE         = $(shell find ./smart_contracts/contracts/SRE       -mindepth 1 -maxdepth 1 -type d -exec basename {} \;)
TEST        = $(shell find ./smart_contracts/contracts/test        -mindepth 1 -maxdepth 1 -type d -exec basename {} \;)

BENCH_CONTRACTS     := $(patsubst %, build-contract-rs/%, $(BENCH))
CLIENT_CONTRACTS    := $(patsubst %, build-contract-rs/%, $(CLIENT))
EXPLORER_CONTRACTS  := $(patsubst %, build-contract-rs/%, $(EXPLORER))
PROFILING_CONTRACTS := $(patsubst %, build-contract-rs/%, $(PROFILING))
SRE_CONTRACTS       := $(patsubst %, build-contract-rs/%, $(SRE))
TEST_CONTRACTS      := $(patsubst %, build-contract-rs/%, $(TEST))

# AssemblyScript Contracts
CLIENT_CONTRACTS_AS  = $(shell find ./smart_contracts/contracts_as/client -mindepth 1 -maxdepth 1 -type d)
TEST_CONTRACTS_AS    = $(shell find ./smart_contracts/contracts_as/test   -mindepth 1 -maxdepth 1 -type d)

CLIENT_CONTRACTS_AS  := $(patsubst %, build-contract-as/%, $(CLIENT_CONTRACTS_AS))
TEST_CONTRACTS_AS    := $(patsubst %, build-contract-as/%, $(TEST_CONTRACTS_AS))

CONTRACT_TARGET_DIR       = target/wasm32-unknown-unknown/release
CONTRACT_TARGET_DIR_AS    = target_as

CRATES_WITH_DOCS_RS_MANIFEST_TABLE = \
	execution_engine_testing/test_support \
	node \
	smart_contracts/contract \
	types

CRATES_WITH_DOCS_RS_MANIFEST_TABLE := $(patsubst %, doc-stable/%, $(CRATES_WITH_DOCS_RS_MANIFEST_TABLE))

.PHONY: all
all: build build-contracts

.PHONY: build
build:
	$(CARGO) build $(CARGO_FLAGS)

build-contract-rs/%:
	$(CARGO) build \
	        --release $(filter-out --release, $(CARGO_FLAGS)) \
	        --package $* \
	        --target wasm32-unknown-unknown
	wasm-strip target/wasm32-unknown-unknown/release/$(subst -,_,$*).wasm 2>/dev/null | true

build-contracts-rs: \
	$(BENCH_CONTRACTS) \
	$(CLIENT_CONTRACTS) \
	$(EXPLORER_CONTRACTS) \
	$(PROFILING_CONTRACTS) \
	$(SRE_CONTRACTS) \
	$(TEST_CONTRACTS)

.PHONY: build-client-contracts
build-client-contracts: $(CLIENT_CONTRACTS)

build-contract-as/%:
	cd $* && $(NPM) run asbuild

.PHONY: build-contracts-as
build-contracts-as: \
	$(CLIENT_CONTRACTS_AS) \
	$(TEST_CONTRACTS_AS) \
	$(EXAMPLE_CONTRACTS_AS)

.PHONY: build-contracts
build-contracts: build-contracts-rs build-contracts-as

resources/local/chainspec.toml: generate-chainspec.sh resources/local/chainspec.toml.in
	@./$<

.PHONY: test-rs
test-rs: resources/local/chainspec.toml
	$(DISABLE_LOGGING) $(CARGO) test $(CARGO_FLAGS) --workspace
	$(DISABLE_LOGGING) $(CARGO) test $(CARGO_FLAGS) --features=std --manifest-path=types/Cargo.toml
	$(DISABLE_LOGGING) $(CARGO) test $(CARGO_FLAGS) --features=std --manifest-path=smart_contracts/contract/Cargo.toml

.PHONY: test-as
test-as: setup-as
	cd smart_contracts/contract_as && npm run asbuild && npm run test

.PHONY: test
test: test-rs test-as

.PHONY: test-contracts-rs
test-contracts-rs: build-contracts-rs
	$(DISABLE_LOGGING) $(CARGO) test $(CARGO_FLAGS) -p casper-engine-tests -- --ignored

.PHONY: test-contracts-as
test-contracts-as: build-contracts-rs build-contracts-as
	@# see https://github.com/rust-lang/cargo/issues/5015#issuecomment-515544290
	$(DISABLE_LOGGING) $(CARGO) test $(CARGO_FLAGS) --manifest-path "execution_engine_testing/tests/Cargo.toml" --features "use-as-wasm" -- --ignored

.PHONY: test-contracts
test-contracts: test-contracts-rs test-contracts-as

.PHONY: check-format
check-format:
	$(CARGO) fmt --all -- --check

.PHONY: format
format:
	$(CARGO) fmt --all

.PHONY: lint
lint:
	$(CARGO) clippy --all-targets --all-features --workspace -- -D warnings -A renamed_and_removed_lints

.PHONY: audit
audit:
	$(CARGO) audit --ignore RUSTSEC-2021-0072

.PHONY: build-docs-stable-rs
build-docs-stable-rs: $(CRATES_WITH_DOCS_RS_MANIFEST_TABLE)

doc-stable/%: CARGO_TOOLCHAIN += +stable
doc-stable/%:
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc $(CARGO_FLAGS) --manifest-path "$*/Cargo.toml" --no-deps

.PHONY: check-rs
check-rs: \
	build-docs-stable-rs \
	build \
	check-format \
	lint \
	audit \
	test-rs \
	test-contracts-rs

.PHONY: check
check: \
	build-docs-stable-rs \
	build \
	check-format \
	lint \
	audit \
	test \
	test-contracts

.PHONY: clean
clean:
	rm -rf resources/local/chainspec.toml
	rm -rf $(CONTRACT_TARGET_DIR_AS)
	$(CARGO) clean

.PHONY: build-for-packaging
build-for-packaging: build-client-contracts
	$(CARGO) build --release

.PHONY: deb
deb: setup build-for-packaging
	cd client && $(CARGO) deb -p casper-client --no-build

.PHONY: package
package:
	cd contract && $(CARGO) package

.PHONY: publish
publish:
	./publish.sh

.PHONY: bench
bench: build-contracts-rs
	$(CARGO) bench

.PHONY: setup-cargo-packagers
setup-cargo-packagers:
	$(CARGO) install cargo-rpm || exit 0
	$(CARGO) install cargo-deb || exit 0

.PHONY: setup-audit
setup-audit:
	$(CARGO) install cargo-audit

.PHONY: setup-rs
setup-rs: rust-toolchain
	$(RUSTUP) update --no-self-update
	$(RUSTUP) toolchain install --no-self-update $(RUST_TOOLCHAIN)
	$(RUSTUP) target add --toolchain $(RUST_TOOLCHAIN) wasm32-unknown-unknown

.PHONY: setup-stable-rs
setup-stable-rs: RUST_TOOLCHAIN := stable
setup-stable-rs: setup-rs

.PHONY: setup-nightly-rs
setup-nightly-rs: RUST_TOOLCHAIN := nightly
setup-nightly-rs: setup-rs

.PHONY: setup-as
setup-as: smart_contracts/contract_as/package.json
	cd smart_contracts/contract_as && $(NPM) ci

.PHONY: setup
setup: setup-rs setup-as
