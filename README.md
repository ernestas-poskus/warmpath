# warmpath

Provider-agnostic prompt-cache accounting, warm-session decisions, and metric gates for
applications that orchestrate coding agents — as a small library you can audit.

[![crates.io](https://img.shields.io/crates/v/warmpath.svg)](https://crates.io/crates/warmpath)
[![docs.rs](https://docs.rs/warmpath/badge.svg)](https://docs.rs/warmpath)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Why it exists

Every coding-agent host re-invents the same fragile accounting: what the provider really charged for
prompt tokens, whether the previous session can be resumed, and what a "cache hit rate" even means.
The failure mode is always the same — **an absence of data silently becomes a zero**, and a zero
becomes a percentage that drives a decision. `warmpath` is the fixed version of that accounting,
published once so it can be reviewed, tested, and reused. It knows tokens, providers, harnesses,
sessions, metrics, and files; it does not know your product, your workflow, or your task tracker, and
it makes no model calls.

## Install

```sh
cargo add warmpath
```

The minimum supported Rust version is **1.95** (Rust edition 2024). No async runtime is required
unless you use the artifact store — see [Async runtime](#async-runtime).

## Quickstart

```rust
use warmpath::profile::HarnessProfile;
use warmpath::usage::parse;

let profile = HarnessProfile::codex();
let output = br#"{"type":"turn.completed","usage":{"input_tokens":1200,"cached_input_tokens":900,"output_tokens":40}}"#;
let outcome = parse(&profile, output);
let fresh = outcome.usage.prompt.fresh().as_reported().copied();
assert_eq!(fresh.map(|tokens| tokens.get()), Some(300));
```

`parse` reads the provider's own vocabulary; `PromptTokens` is the single place that normalizes it
into fresh, cached, and expected counts. This snippet is the start of the crate-level doc example,
which `cargo test --doc` compiles and runs.

## What it gives you

- **Reported, never invented, token numbers.** `Reported<T>` keeps a measured zero apart from an
  absent field; a provider that does not print a number never becomes a synthetic 0 %.
- **One normalization point.** `PromptTokens` handles inclusive (OpenAI/Codex), exclusive
  (Anthropic and most others), and partitioned prompt shapes, and exposes `fresh()` and `expected()`
  for everything downstream.
- **Harness profiles as data.** Output shape, usage paths, input shape, cache class, session paths,
  resume argv, version range, and TTL, with built-ins for Codex, Claude, Antigravity, DSH, and Grok.
  The provider is an explicit field: a gateway is never assumed to share a harness's cache contract.
- **Provider cache contracts from documented behavior.** Anthropic's model-dependent minimums and
  read multipliers, 5m/1h write costs; OpenAI's automatic cache and model-gated writes; Gemini's
  implicit `>=2050`-token rule with explicit caching legacy-only; strict semantics for an unknown
  route.
- **A warm decision with evidence.** Identity, TTL (a step at expiry), file overlap, and the
  `(w − r)` resume counterfactual, plus stable mismatch/rejection codes.
- **An identity that includes the tool surface, and an append-only gate.** `ToolManifest` is a
  canonical, order-insensitive digest of tool definitions with the names included, so a rename is
  a different surface and a discovery shuffle is not; `CacheIdentity` carries it, and
  `check_append_only` names the first request step that stops extending its predecessor. The
  silent cache break becomes a value a test can assert on.
- **Metric contracts.** AICS (token-weighted cached share, cold included) and EEPR (expected-prefix
  reuse, warm only with cold reported separately), with exported exclusions, per-request
  distributions, write amplification, and integer dollar savings.
- **Calibration and scheduling.** Age-band warm-resume reports with a two-warm-sample floor, and a
  deterministic cache-aware batch order.
- **Pre-provider savings attribution.** `savings` sums what the request path removed before it
  reached the provider — ingest-stage and request-stage reductions, each reported or measured. A
  reported saving never invents a before/after, and the pre-provider total is claimed only when
  every stage was measured. Provider cache savings stay in the cache/metric modules, not here.
- **Async artifact storage.** Path-confined names, bounded reads, atomic no-clobber publishes,
  recorded identities, and validation of untrusted metric artifacts.

## Design rules

1. **Every public quantity is a newtype with a private field.** Invalid values cannot be
   constructed; arithmetic is checked or saturating; identifiers are validated; serde cannot bypass
   a constructor.
2. **Only artifact I/O is async.** Parsing, normalization, metric math, and warm decisions are pure
   synchronous functions. `ArtifactStore` uses `tokio::fs` and `tokio::io::AsyncWriteExt` only.

`#![forbid(unsafe_code)]` and `#![deny(missing_docs)]` are not negotiable.

## Async runtime

The crate **does not start a Tokio runtime**. You need one for `ArtifactStore` and for the
`#[tokio::test]` suite: enable `tokio` with `rt`/`rt-multi-thread` in your application. The library
registers no global state.

## Supported harnesses (built-in profiles)

| harness | provider (explicit) | output | input shape | cache class | resume | warm TTL |
| --- | --- | --- | --- | --- | --- | --- |
| `codex` | `openai` | JSONL | inclusive | automatic | `exec resume <session>` | 600 s |
| `claude` | `claude` | JSON | exclusive | marker | `--resume <session>` (assignable) | 300 s |
| `agy` | `agy` (gateway → strict) | JSON | exclusive | strict | `--resume <session>` | 300 s |
| `dsh` | `dsh` (strict) | JSONL (per-step) | exclusive | strict | `--resume <session>` | 300 s |
| `grok` | `grok` (strict) | JSON | exclusive | strict | none | none |

A custom harness is a `HarnessProfile::custom(...)` builder: declare its paths and shapes rather
than guessing them.

## Metric contracts

| id | measures | numerator | denominator | exclusions |
| --- | --- | --- | --- | --- |
| `aics.v1` | agent input cache share | cache-read input tokens | cache-read + fresh input | output; unreported requests; cold/warm blended |
| `eepr.v1` | expected-prefix reuse (warm) | cache-read tokens of warm runs | expected prompt tokens of warm runs | cold runs (reported separately); unreported requests; output |

A zero denominator is `Insufficient`, never a synthetic 0 %. Target parsing accepts percentages such
as `99.7`; `0.997` is **not** accepted as a fraction — bare numbers are percentages (`99.7` →
9 970 bps).

## Honest limits

- **No network, no credentials, no model calls.** It reads bytes you already have.
- **No cache guarantee.** The contracts say what providers document; a provider can miss a cache
  for reasons the request cannot express.
- **Calibration is observational.** The TTL is a documented step, not a learned curve; the
  two-warm-sample floor exists so a scenario is not reported as a finding.
- **Write cost estimates are ratios**, not invoices; use `dollar_savings_micro_usd` with the price
  you actually pay.
- **The artifact store is local and unprivileged.** It confines names to its root and bounds reads,
  but it is not a security boundary against a hostile filesystem (symlinks are not followed
  specially; the temp-then-link publish assumes a local POSIX-like filesystem).

## Examples

- `cargo run --example parse_and_gate` — parse a Codex turn and gate it on AICS.
- `cargo run --example warm_session` — decide whether a captured Claude session should be resumed.
- `cargo run --example artifact_reuse` — publish and reuse a metric artifact (own Tokio runtime).
- `cargo run --example schedule_batch` — order a batch so warm prompt-cache keys go first.
- `cargo run --example calibration_report` — summarize warm-resume observations by age band.

## Documentation

- API: <https://docs.rs/warmpath> · Design: [`docs/DESIGN.md`](docs/DESIGN.md) · Changelog: [`CHANGELOG.md`](CHANGELOG.md)

## License

MIT. See [`LICENSE`](LICENSE).

## Contributing

- Run `make verify` before opening a pull request; it runs the same gates as CI
  (`fmt`, `check`, `clippy`, `nextest`, doc-tests, docs, and the package check). `make test`
  requires [cargo-nextest](https://nexte.st/), and the MSRV gate is `make msrv`.
- Keep the crate domain-agnostic: no caller workflow concepts in public names, tests, examples, or
  error messages. If a name describes the caller's process rather than the provider, it does not
  belong here.
- Examples and tests stay offline: no network, no real provider calls, no environment-dependent
  expectations. The public API is a compatibility surface; a breaking change needs a minor `0.x`
  bump and a changelog entry.
- New dependencies need a reason in the pull request; prefer the standard library and the existing
  small set. Open a pull request for changes, and start a discussion first for a new public API or a
  changed contract.
