# Makefile for ls_web (Rust)
#
# Usage:
#   make          # build (debug)
#   make run      # build + run (debug)
#   make test     # run unit tests
#   make fmt      # format code (rustfmt)
#   make clippy   # run clippy lints
#   make check    # run cargo check
#   make doc      # build documentation
#   make clean    # clean build artifacts

CARGO ?= cargo

.PHONY: all help build run test fmt clippy check doc clean

all: build

help:
	@echo "Usage: make [target]"
	@echo "Targets:"
	@echo "  build    - compile the project (debug)"
	@echo "  release  - compile the project (optimized release)"
	@echo "  run      - compile and run the project (debug)"
	@echo "  run-release - compile and run the project (release)"
	@echo "  test     - run tests"
	@echo "  fmt      - format source with rustfmt"
	@echo "  clippy   - run clippy lints (requires clippy component)"
	@echo "  check    - run cargo check"
	@echo "  doc      - build documentation"
	@echo "  clean    - remove build artifacts"

build:
	$(CARGO) build

run:
	$(CARGO) run

run-release:
	$(CARGO) run --release

release:
	$(CARGO) build --release

test:
	$(CARGO) test

fmt:
	$(CARGO) fmt

clippy:
	$(CARGO) clippy --all-targets --all-features -- -D warnings

check:
	$(CARGO) check

doc:
	$(CARGO) doc --no-deps

clean:
	$(CARGO) clean
