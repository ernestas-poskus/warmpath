CARGO ?= cargo
MSRV := $(shell sed -n 's/^rust-version = "\(.*\)"/\1/p' Cargo.toml)
RUSTDOCFLAGS ?= -D warnings

.PHONY: fmt check clippy test doc package msrv verify

## Formatting: fail if any file is not rustfmt-clean.
fmt:
	$(CARGO) fmt --all -- --check

## Compile every target, including tests and examples.
check:
	$(CARGO) check --all-targets

## Lint every target and deny warnings.
clippy:
	$(CARGO) clippy --all-targets -- -D warnings

## Run the offline test suite (nextest) and the doc-tests.
test:
	$(CARGO) nextest run --status-level fail
	$(CARGO) test --doc

## Build the API documentation and deny rustdoc warnings.
doc:
	RUSTDOCFLAGS="$(RUSTDOCFLAGS)" $(CARGO) doc --no-deps

## Verify the package manifest and the files that would ship.
package:
	$(CARGO) package --list --allow-dirty >/dev/null

## Check the crate against its declared minimum supported Rust version.
msrv:
	$(CARGO) +$(MSRV) check --all-targets

## Run every gate locally.
verify: fmt check clippy test doc package
