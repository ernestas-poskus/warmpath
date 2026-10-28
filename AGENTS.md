# warmpath agent instructions

`warmpath` is a **standalone Cargo workspace**. It owns domain-agnostic cache/reuse primitives and
must never learn about a consuming product.

## Boundaries

- **Domain-agnostic.** No caller workflow concept — a tracker item, a work request, a product
  name — in the public API, tests, examples, docs, or error messages. If a name describes *the
  caller's* workflow rather than *the provider*, it does not belong here.
- **Two hard rules.** Every public quantity is a newtype with a private field and a validating
  constructor; only artifact filesystem work is asynchronous (`tokio::fs` +
  `tokio::io::AsyncWriteExt`, never starting a runtime, never blocking `std::fs` in an async path).
- **The public API is a compatibility surface.** A breaking change needs a `0.x` version bump and a
  CHANGELOG entry.

## Gates

Run inside `warmpath/`:

```bash
cargo fmt --all -- --check
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
cargo nextest run --status-level fail
cargo test --doc
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
cargo package --list --allow-dirty
cargo +<rust-version> check --all-targets
```

`make verify` runs the same set locally and `make msrv` checks the declared minimum supported Rust
version; `.github/workflows/ci.yml` runs them in CI.

## Dependencies

- Keep the dependency set small and conventional: `serde`, `serde_json`, `sha2`, `thiserror`, and
  `tokio` with only `fs`/`io-util`.
- Tests are offline: no network, no real provider calls, no environment-dependent expectations.
- The untyped-JSON opt-out (`clippy::disallowed_types`) is allowed only in the modules that parse
  untrusted output, and only with the local comment that says why.
