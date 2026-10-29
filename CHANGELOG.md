# Changelog

All notable changes to `warmpath` are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses semantic
versioning for the public API.

## Unreleased

### Added

- `fanout` module: `FanoutKey`/`FanoutMark`/`FanoutDecision`/`FanoutPlan`/`FanoutRelease` and
  `plan_fanout_warmup`, the pure fan-out warm-up decision (which cold members of a same-key cohort
  start now, which wait, and why). It reads no clock beyond the caller's instant and performs no I/O;
  the dispatch plane supplies the keys and marks and records the release reason as evidence.
- `shadow` module: `exact_request_key`, `dynamic_tail_key`, `ShadowObservation`, `ShadowSummary`, and
  `summarize` — the measurement-only keys a replay or semantic cache would need, as digests with no
  store, no lookup, and no serving path. A caller maps its persisted record onto the borrowed
  observation.
- `SavingsBasis::Partial`, `SavingsObservation::partial`, `SavingsReport::measured_lower_bound`, and
  `SavingsError::InvalidPartial`: pre-provider savings support for stages where some runs were
  measured and others unmeasured. A partial stage keeps its known measured sum as a lower bound
  ([`SavingsReport::measured_lower_bound`]), while leaving [`SavingsReport::total_saved`] and
  [`SavingsReport::total_before`] absent (`None`).

## 0.4.0 — 2026-10-28

### Added

- `CacheIdentity::speed` with `with_speed`: the route's speed class (for example a fast mode) is
  now a first-class cache-identity dimension. Providers that key their cache by speed treat a
  speed change as a different cache series, and an absent speed is unknown — it never claims a
  match. The identity digest includes the speed when one is declared.

### Changed

- **Breaking:** `CacheIdentity` gained a public `speed` field; a struct literal must set it (or
  call `with_speed` on a builder). The `0.x` version bump is for this compatibility change.


## 0.3.1 — 2026-10-28

### Added

- `topology` module: the agent-topology layer contract. `NodeKind` names the instance a request
  belongs to (root, subagent, fork); `PrefixInheritance` distinguishes an inherited parent prefix
  from an own prefix, a shared routing key, and an undocumented relationship; `ChildUsage`
  distinguishes a parent's separate, inclusive, and unknown child accounting. The types are
  data for records, so a run can name the version it was resolved against (`TOPOLOGY_VERSION`).
- `topology::capabilities(&HarnessId)`: the declared capability table. Claude Code starts
  subagents fresh, forks inherit, documents a same-prefix hold, and reports usage that excludes
  subagent tokens; Codex children share an affinity key but not the prefix while a full-history
  fork inherits; DSH forks from a completed-turn seed and spawns fresh. Every unnamed harness —
  including a gateway whose request body this build cannot inspect — is `unknown` and claims
  nothing.
- `topology::CohortKey`: the identity facts a sibling fan-out groups on (prefix, provider, model),
  so a cohort is never an identity of its own.

## 0.3.0 — 2026-10-28

Open-source preparation plus one provider-data/API tranche.

### Added

- `examples/schedule_batch.rs`: a cache-aware batch order that puts warm keys first.
- `examples/calibration_report.rs`: warm-resume observations summarized by age band.
- Integration tests for the public surface: the crate-root re-exports, the custom-profile builder,
  a warm request built from a record, batch ordering, and an end-to-end parse → gate →
  calibration pipeline.
- `Makefile` with `fmt`, `check`, `clippy`, `test`, `doc`, `package`, `msrv`, and `verify` targets.
- GitHub Actions CI: formatting, compile, Clippy, tests, doc-tests, docs, package check, and the
  declared MSRV.
- `cache::ProviderFamily::{Xai, Moonshot}` with the documented contracts: xAI automatic caching
  (model-dependent read discount, no declared TTL or minimum) and Moonshot implicit caching
  (5 m default at a 1.0x write, 1 h at 2x, no published minimum or block size). Both stay strict for
  an empty model, like every other family.
- `CacheContract::repeated_reuse_saving(prefix, copies)`: the expected tokens saved when several
  requests would share one prefix write instead of each paying one, `(copies - 1) * prefix * (w - r)`.
  It returns `None` for a strict contract and for fewer than two copies, and it is an estimate.
- `Ratio::multiply_tokens`: saturating, half-up scaling used by the helper above.

### Changed

- Documentation is de-branded and written for a standalone crate: the README, crate-level docs,
  `docs/DESIGN.md`, and `AGENTS.md` now describe a coding-agent host or an orchestration layer
  instead of the private integration this code was extracted from.
- `Cargo.toml` carries complete package metadata (`description`, `keywords`, `categories`,
  `documentation`, `repository`) and docs.rs settings.
- **Breaking:** `cache::ProviderFamily` gained `Xai` and `Moonshot`; an exhaustive match on the
  public enum must say what it does with them. The `0.x` version bump is for this change.
- `cache::SEMANTICS_VERSION` is `2`; recorded runs name the version they were interpreted under.
- `HarnessProfile::grok` is `grok-v2`: automatic cache class, inclusive input shape, and the nested
  xAI usage paths (`usage.prompt_tokens_details.cached_tokens`, Responses
  `usage.input_tokens_details.cached_tokens`); the non-existent write field is gone.
- `HarnessProfile::agy` is `agy-v2`: reasoning tokens read `usage.thinking_tokens` first, and the
  resume form is agy's own `--conversation {session}`. The provider literal stays the gateway's
  (`agy`), which resolves to the strict contract.

### Notes

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
