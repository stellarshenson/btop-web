CARGO ?= cargo
DOCKER ?= docker
IMAGE ?= btop-web:dev
RUSTUP ?= rustup
LEVEL ?= patch
EXECUTE ?= 0
FEATURES ?=
FEATURES_FLAG = $(if $(FEATURES),--features $(FEATURES))
RUST_VERSION := $(shell sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)

.DEFAULT_GOAL := help

.PHONY: help build fmt-check clippy test deny coverage ci run docker clean env tools version upgrade release doctor

## Print the list of available commands
help:
	@python3 -c "$${PRINT_HELP_PYSCRIPT}" < $(MAKEFILE_LIST)

## Build the release binary
build:
	$(CARGO) build --locked --release $(FEATURES_FLAG)

## Check formatting with rustfmt
fmt-check:
	$(CARGO) fmt --all -- --check

## Lint with clippy, warnings are errors
clippy:
	$(CARGO) clippy --workspace --all-targets $(FEATURES_FLAG) -- -D warnings

## Run the test suite
test:
	$(CARGO) test --locked --workspace --all-targets $(FEATURES_FLAG)

## Check advisories, licences, bans and sources with cargo-deny
deny:
	@if command -v cargo-deny >/dev/null 2>&1; then \
		cargo deny check; \
	else \
		echo "cargo-deny not installed - skipping"; \
	fi

## Print the coverage summary with cargo-llvm-cov
coverage:
	@if command -v cargo-llvm-cov >/dev/null 2>&1; then \
		cargo llvm-cov --workspace $(FEATURES_FLAG) --summary-only; \
	else \
		echo "cargo-llvm-cov not installed - skipping"; \
	fi

## Run every gate: format check, clippy and tests
ci: fmt-check clippy test

## Build and run the server
run:
	$(CARGO) run --locked --release $(FEATURES_FLAG)

# shortcut: the Dockerfile has no ARG FEATURES, so this build arg is ignored until it declares one
## Build the container image
docker:
	$(DOCKER) build -t $(IMAGE) $(if $(FEATURES),--build-arg FEATURES=$(FEATURES)) .

## Remove the build output
clean:
	$(CARGO) clean

## Install the pinned Rust toolchain and components (Rust has no virtualenv: the pinned toolchain plus Cargo's target directory is the project environment)
env:
	@if ! command -v $(RUSTUP) >/dev/null 2>&1; then \
		echo "rustup not found - install it with:"; \
		echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"; \
		exit 1; \
	fi
	$(RUSTUP) toolchain install $(RUST_VERSION) --profile minimal
	$(RUSTUP) component add rustfmt clippy --toolchain $(RUST_VERSION)

## Install the dev tools this Makefile uses, skipping any already on PATH
tools:
	@for tool in cargo-deny cargo-llvm-cov cargo-release cargo-edit; do \
		cmd=$$tool; \
		if [ "$$tool" = cargo-edit ]; then cmd=cargo-upgrade; fi; \
		if command -v $$cmd >/dev/null 2>&1; then \
			echo "$$tool: already installed"; \
		else \
			echo "$$tool: installing"; \
			$(CARGO) install --locked $$tool || exit 1; \
		fi; \
	done

## Print the workspace version from Cargo.toml
version:
	@grep -m1 '^version = ' Cargo.toml | cut -d'"' -f2

## Update Cargo.lock and upgrade Cargo.toml requirements when cargo-edit is installed
upgrade:
	$(CARGO) update
	@if command -v cargo-upgrade >/dev/null 2>&1; then \
		cargo upgrade --incompatible; \
	else \
		echo "cargo-upgrade not installed - install it with: cargo install --locked cargo-edit"; \
	fi

## Release through cargo-release with LEVEL=patch|minor|major, dry run unless EXECUTE=1
release:
	@if [ "$(LEVEL)" = none ]; then \
		echo "LEVEL=none - refusing to release"; \
		exit 1; \
	fi
	@if [ "$(EXECUTE)" = 1 ]; then \
		$(CARGO) release $(LEVEL) --execute; \
	else \
		$(CARGO) release $(LEVEL); \
	fi

## Report the pinned and active toolchain and which dev tools are present
doctor:
	@echo "pinned toolchain: $(RUST_VERSION)"
	@if command -v $(RUSTUP) >/dev/null 2>&1; then \
		echo "active toolchain: $$($(RUSTUP) show active-toolchain)"; \
	else \
		echo "active toolchain: rustup not found"; \
	fi
	@for tool in cargo rustup cargo-deny cargo-llvm-cov cargo-release cargo-edit; do \
		cmd=$$tool; \
		if [ "$$tool" = cargo-edit ]; then cmd=cargo-upgrade; fi; \
		if command -v $$cmd >/dev/null 2>&1; then \
			echo "$$tool: present"; \
		else \
			echo "$$tool: missing"; \
		fi; \
	done

define PRINT_HELP_PYSCRIPT
import re, sys; \
lines = sys.stdin.read(); \
matches = re.findall(r'\n## ([^\n]+)\n(?!\.PHONY)([a-zA-Z_.][a-zA-Z0-9_.-]*):', lines); \
matches = sorted(matches, key=lambda x: x[1].lower()); \
print('\nAvailable rules:\n'); \
print('\n'.join(['\033[36m{:25}\033[0m{}'.format(*reversed(match)) for match in matches])); \
print()
endef
export PRINT_HELP_PYSCRIPT
