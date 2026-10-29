# Design

## Why the crate exists

Every application that orchestrates a coding agent re-invents the same fragile accounting: what the
provider really charged for prompt tokens, whether the previous session can be resumed, and what a
"cache hit rate" even means. The failure mode is always the same — an absence of data silently
becomes a zero, and a zero becomes a percentage that drives a decision. `warmpath` is the fixed
version of that accounting, published as a library so it can be reviewed once. The research it
rests on — provider mechanics, harness practices, and the strategies considered — is collected in
[`research/README.md`](research/README.md).

## The two rules

**Newtypes.** Every public quantity has a private field and a validating constructor. `Tokens`,
`FreshTokens`, `CachedTokens`, `CacheWriteTokens`, `OutputTokens`, and `ReasoningTokens` are
distinct types even though all wrap `u64`: a function that expects fresh input cannot be handed a
count that includes cache reads. `BasisPoints` is bounded to `0..=10_000`; `Ratio` cannot have a
zero denominator and is reduced to lowest terms; identifiers and digests validate at construction
and on deserialization (`try_from`), so a wire value cannot bypass its checks.

**Synchronous core, async edges.** Parsing, canonicalization, metric arithmetic, calibration, and
warm decisions are pure functions: no clock, no I/O, no randomness beyond the caller's inputs. Only
`artifact::ArtifactStore` performs I/O, with `tokio::fs` and `tokio::io::AsyncWriteExt`. The crate
never constructs a runtime, and no async path calls blocking `std::fs`.

## Reported versus measured

A provider may omit a field, print it as zero, or print a count for a shape in which it means
something else. The crate encodes that explicitly:

- `Reported<T>` — `NotReported` or `Reported(T)`; there is no `Default` and no `Deref`.
- `PromptTokens` — the reported input, cache read, cache write, and the `InputShape` that says what
  the input includes. `fresh()` and `expected()` are the only normalization points.

An inclusive input whose cache-read field is missing has an **unknown** fresh share. A zero
denominator is an undefined share (`None`), not 0 %. A provider that reports a read larger than its
input is inconsistent; fresh saturates at zero and the share clamps at 100 %, because inventing a
negative count would be worse than reporting the inconsistency's arithmetic consequence.

## Cache semantics are data

`cache::semantics(provider, model)` returns a `CacheContract`: mode, TTL(s), read/write cost
multipliers, minimum cacheable length, and a one-line note. Unknown routes are **strict** — no
marker, no TTL, unit multipliers — so a route this build cannot source never produces a savings
claim. The provider is a field on `HarnessProfile`, never inferred from the harness name: `agy`, for
example, is a gateway.

## The TTL is a step

A warm decision compares age against the declared TTL. At the expiry instant the session is refused.
There is no exponential decay in the decision path: providers document a lifetime, not a curve, and
a decay model would turn "maybe warm" into a number nobody measured. Calibration *observes* what
happened inside each age band; it does not change the decision rule.

## Resume counterfactual

The expected saving of a warm resume is:
`cached × overlap × (w − r)`
where `cached` is the previous run's cache-read count, `overlap` is the declared-file Jaccard share,
and `w − r` is the avoided cache-write multiplier minus the read multiplier actually paid. It is an
estimate; it saturates at zero and is never used to claim a *measured* saving (that is the
calibration report's job).

## Identity includes the tool surface

A `CacheIdentity` is the prefix digest, the route that serves it (harness, provider, model, effort,
session), and — since 0.2 — the canonical tool-surface digest. Tool definitions are prompt bytes:
two requests with the same prefix and route but a different `ToolManifest` do not share a provider
cache entry, so an identity that compared equal would be lying. An absent surface is `None`:
unknown, never a claimed match. `first_request_break` names a changed surface before the prefix
rewrite it causes, because the surface is the cause and the one break a caller can fix.

## Topology is declared, not inferred

A request's cache identity describes bytes; it does not describe the agent topology those bytes
belong to. The same prefix can be sent by a root, a subagent, a fork, or a sibling fan-out, and
those differ in a way that changes reuse and attribution: a fork may read its parent's write, a
subagent starts its own, and a parent's reported usage may or may not include its children's.

`topology` makes that structure a contract. `NodeKind` names the instance (root, subagent, fork);
`PrefixInheritance` distinguishes an inherited parent prefix from an own prefix, a shared routing
key, and an undocumented relationship; `ChildUsage` keeps separate, inclusive, and unknown child
accounting apart. `capabilities(harness)` is a small, versioned table of what the harness
documents — the same "unknown is strict" rule the provider table follows — and `CohortKey` names
the identity facts a sibling fan-out groups on, so a cohort groups nodes and is never an identity
itself. A root always owns its prefix; inheritance is claimed only when the table says
`InheritsParent`.

## The append-only invariant is checkable

The cache unit is an exact byte prefix, so a request sequence is only expectably warm while every
step extends its predecessor. `prefix::check_append_only` states that invariant as a value: it
returns `Ok(())` for a growing or identically repeated sequence and names the first violating step
(and its first differing byte) otherwise. It is an assertion over bytes the caller already has, not
a new subsystem: it calls no provider, keeps no state, and is cheap enough to run over a scripted
sequence in a test or a recorded trace in a report.

## Metric contracts

A contract is a value (`MetricContract`) with the numerator, denominator, exclusions, unit, and
rounding written down and exported. Two are built in:

- **AICS** answers "how much of the prompt traffic did the provider serve from cache?", pooled over
  all measured requests, cold included.
- **EEPR** answers "when a warm run existed, how much of the expected prefix was served?", warm only,
  with cold runs excluded from the ratio and reported separately.

Both refuse a zero denominator (`Insufficient`). Percentages are basis points; the arithmetic uses
`u128` intermediates and rounds half up.

## Pre-provider savings

Two reduction stages can remove prompt work before a request reaches its provider, in request
order: `Ingest` and `Request`. `savings` attributes them and nothing else. Provider-side cache
savings are a billing fact, not a reduction, and never enter this total.

The contract is deliberately narrow. A stage either **reports** how much it removed, is
**measured** before and after, or is **partial** (some contributions were measured while others were
unmeasured). The report never derives a before/after from a reported count. A partial stage keeps
its known sum as a lower bound (`measured_lower_bound`), while the saved total (`total_saved`) and
pre-provider total (`total_before`) remain absent (`None`). The saved total exists only when every
observation is recording and usable — an absent, disabled, or partial observation leaves `total_saved`
`None`, not zero — and the pre-provider ("before") total exists only when every stage was measured,
because the stages are sequential and summing their `before` values would count the prompt once per
stage. Duplicate stages, a measured pair with `before < after`, and invalid partial counts are
construction errors with stable codes.

## Artifacts

Artifacts are opaque bytes addressed by `ArtifactName` (`subject-<digest[..8]>[-variant].ext`) and
produced under an `ArtifactIdentity` (crate version, contract and profile versions, harness version,
input digest). Publishing writes a temp file in the same directory, syncs it, and hard-links it into
place, so an existing name is a `Conflict` rather than a silent overwrite. Reuse reads under a
bound, validates the bytes as untrusted input, and compares the recorded identity; anything else is
`Stale` with a reason.
