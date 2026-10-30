.PHONY: help increment_version_number build build-static fmt-check clippy test deny coverage sbom ci run docker clean env tools version upgrade release doctor

#################################################################################
# GLOBALS                                                                       #
#################################################################################

CARGO ?= cargo
DOCKER ?= docker
IMAGE ?= btop-web:dev
RUSTUP ?= rustup
LEVEL ?= patch
# Set EXECUTE=1 to let make release act instead of a dry run
EXECUTE ?= 0
FEATURES ?=
FEATURES_FLAG = $(if $(FEATURES),--features $(FEATURES))
RUST_VERSION := $(shell sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)

# Every ?= variable is also read from the environment: STATIC=1 make build is make build STATIC=1
# Set STATIC=1 to build the static glibc binary without FEATURES (make build-static is make build STATIC=1)
STATIC ?= 0
# Set STRIP=1 to strip the symbols from the release binary
STRIP ?= 0
# Set SKIP_VERSION_INCREMENT=1 to skip the patch version increment in build, build-static, run and docker
SKIP_VERSION_INCREMENT ?= 0

#################################################################################
# COMMANDS                                                                      #
#################################################################################

## Increment build number (skip with SKIP_VERSION_INCREMENT=1)
increment_version_number:
	@if [ "$(SKIP_VERSION_INCREMENT)" = "1" ]; then \
		echo "skipping version increment (SKIP_VERSION_INCREMENT=1)"; \
	elif ! command -v cargo-set-version >/dev/null 2>&1; then \
		echo "cargo-set-version not installed - install it with: make tools"; \
		exit 1; \
	else \
		$(CARGO) set-version --workspace --bump patch; \
	fi

## Build the release binary; STATIC=1 builds the static glibc binary without FEATURES (rocm needs the shared librocm_smi64), STRIP=1 strips its symbols
build: increment_version_number
	$(if $(filter 1,$(STRIP)),CARGO_PROFILE_RELEASE_STRIP=true) $(if $(filter 1,$(STATIC)),RUSTFLAGS="-C target-feature=+crt-static") $(CARGO) build --locked --release $(if $(filter 1,$(STATIC)),--target x86_64-unknown-linux-gnu,$(FEATURES_FLAG))

## Build the static glibc release binary, the same as make build STATIC=1; deploys use it
build-static:
	$(MAKE) build STATIC=1

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

## Regenerate sbom.cdx.json from Cargo.lock and the crate manifests, offline
sbom:
	python3 scripts/sbom.py

# The glibc arena cap takes effect only from the environment that starts the
# process; a value set inside it arrives too late. The deployed service must set
# MALLOC_ARENA_MAX=1 in its unit or container environment as well.
## Build and run the server with one glibc arena
run: increment_version_number
	MALLOC_ARENA_MAX=1 $(CARGO) run --locked --release $(FEATURES_FLAG)

# shortcut: the Dockerfile has no ARG FEATURES, so this build arg is ignored until it declares one
## Build the container image
docker: increment_version_number
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

#################################################################################
# Self Documenting Commands                                                     #
#################################################################################

.DEFAULT_GOAL := help

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

## Print the list of available commands
help:
	@python3 -c "$${PRINT_HELP_PYSCRIPT}" < $(MAKEFILE_LIST)
