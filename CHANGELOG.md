# Changelog

All notable changes to `warmpath` are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses semantic
versioning for the public API.

## Unreleased

Open-source preparation. There is **no public API or behavioural change**, so the crate stays at
0.2.0.

### Added

- `examples/schedule_batch.rs`: a cache-aware batch order that puts warm keys first.
- `examples/calibration_report.rs`: warm-resume observations summarized by age band.
- Integration tests for the public surface: the crate-root re-exports, the custom-profile builder,
  a warm request built from a record, batch ordering, and an end-to-end parse → gate →
  calibration pipeline.
- `Makefile` with `fmt`, `check`, `clippy`, `test`, `doc`, `package`, `msrv`, and `verify` targets.
- GitHub Actions CI: formatting, compile, Clippy, tests, doc-tests, docs, package check, and the
  declared MSRV.

### Changed

- Documentation is de-branded and written for a standalone crate: the README, crate-level docs,
  `docs/DESIGN.md`, and `AGENTS.md` now describe a coding-agent host or an orchestration layer
  instead of the private integration this code was extracted from.
- `Cargo.toml` carries complete package metadata (`description`, `keywords`, `categories`,
  `documentation`, `repository`) and docs.rs settings.
- No new dependencies; `#![forbid(unsafe_code)]` and `#![deny(missing_docs)]` remain.

## 0.2.0 — 2026-10-28

### Added

- `prefix::ToolManifest`: a canonical tool surface (sorted by name, identical duplicates
  collapsed, conflicting duplicates refused) whose digest is insensitive to discovery order and
  sensitive to a rename, a redefinition, an addition, or a removal. The surface is **part of the
  cache identity**: tool definitions are prompt bytes, so two runs with the same prefix and route
  but a different tool manifest do not share a provider cache entry.
- `prefix::PrefixVariance` and `prefix::check_append_only`: the request-sequence invariant behind
  a prefix-cache expectation. Every step must extend its predecessor; an identical repeat counts,
  an earlier rewrite and a shortened step do not, and the first violating step is named with its
  first differing byte.
- `prefix::PrefixDifference::is_extension`: the directional counterpart of `is_append_only`
  (extension versus either-direction prefix).
- `CacheIdentity::tools` with `with_tools` and `with_tool_manifest`: an absent surface (`None`)
  is unknown and never claims a match; the identity digest includes the surface when declared.
- `cache::first_request_break` now attributes a changed tool surface as
  `CacheBreakReason::ToolSurfaceChanged` before the prefix rewrite it causes.
- `savings`: pre-provider savings attribution across the two request-path reduction stages
  (`ReductionStage::{Ingest, Request}`), with `SavingsBasis::{Reported, Measured}`, validated
  `SavingsObservation` values, and a `SavingsReport` whose saved total is claimed only when every
  observation is usable and recording, and whose pre-provider ("before") total is claimed only when
  every stage was measured. Reported savings never invent a before/after, and provider-side cache
  savings are explicitly out of scope.

### Changed

- **Breaking:** `CacheIdentity` gained a public `tools` field and `cache::first_request_break`
  gained a `tool_change: Option<PrefixChange>` argument. Pass `None` when the caller does not
  track a tool surface. The `0.x` version bump is for this compatibility change.

## 0.1.0 — 2026-10-27

First release: the cache-and-reuse primitives packaged as a standalone library.

### Added

- Newtyped public quantities: token counts, basis points, ratios, instants, TTLs, identifiers,
  and digests, all with private fields and validating constructors.
- `Reported<T>` so a measured zero, an absent field, and an inapplicable field stay distinct.
- `PromptTokens` with `InputShape::{Inclusive, Exclusive, Partitioned}` and a single
  fresh/expected normalization point.
- `HarnessProfile` data with built-in Codex, Claude, Antigravity (`agy`), DSH, and Grok profiles;
  the provider is an explicit field, never inferred from the harness.
- Output parsing for text, single-JSON, JSONL, and per-step JSONL harnesses, with warnings instead
  of clamped or invented numbers.
- Canonical prefix hashing (length-delimited SHA-256), tool/JSON/text canonicalization, and prefix
  difference reporting.
- Provider cache contracts with the verified 2026 research corrections: model-dependent Anthropic
  minimums and read multipliers, write costs (1.25×/2×), Gemini's implicit-cache `>=2050` token
  rule and legacy-only explicit caching, and strict-by-default unknown routes.
- Warm-session capture, resume preparation, TTL-as-a-step expiry, identity mismatch reporting, and
  the `(w − r)` resume counterfactual.
- AICS and EEPR metric contracts with exported exclusions, per-request distributions, write
  amplification, and integer dollar savings.
- Warm-resume calibration by age band with a two-warm-sample floor per harness.
- Deterministic cache-aware scheduling (`schedule_order`).
- Async, path-confined artifact storage (`tokio::fs` only) with atomic no-clobber publishing,
  bounded reads, provenance identities, and untrusted artifact validation.

### Notes

- Only artifact filesystem work is asynchronous. The crate never starts a runtime; callers provide
  one (or use the `#[tokio::test]` examples).
- No public API is workflow-specific: there is no product or caller-workflow name in the crate.
