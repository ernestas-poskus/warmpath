# Caching research

The evidence base behind `warmpath`: how provider prompt caches actually behave, how coding-agent
harnesses keep (or break) a reusable prefix, and which reuse strategies are worth building for an
application that orchestrates coding agents. The corpus is written to be read beside the crate, not
instead of it.

## What this corpus is — and is not

**It is** documentation- and source-level research: vendor pages, provider API references, harness
source read at pinned tags/versions, public issues and practitioner reports, each claim graded and
dated. Every document carries a provenance note and its original status/date line.

**It is not** a measurement programme or a guarantee. No model call was made and no request body was
captured while writing these documents; where numbers appear they are either vendor-published, taken
from public reports, or read from recorded runs in one deployment (and labelled as such). Nothing
here promises that a provider will serve a cache hit — the crate models documented behavior, and the
provider may still miss for reasons a request cannot express.

## The two corpora

| Corpus | Dated | What it covers | Start with |
| --- | --- | --- | --- |
| [Prompt and context caching](prompt-caching/README.md) | 2026-10-26 | Provider mechanics (Anthropic, OpenAI, DeepSeek, Gemini), public harness practices, practitioner evidence, sources and confidence | [Provider mechanics](prompt-caching/01-provider-mechanics.md) |
| [Coding-agent caching strategies](agent-caching/README.md) | 2026-10-28 | Seven harnesses (claude, codex, agy, grok, dsh, kimi, perplexity) against seven strategy classes, with a per-strategy verdict | [Matrix](agent-caching/matrix.md) |

Prompt-caching documents:

- [Provider mechanics](prompt-caching/01-provider-mechanics.md) — cache units, minimums, TTLs,
  pricing multipliers, usage fields, invalidation rules.
- [Harness practices](prompt-caching/02-harness-practices.md) — public mechanisms and failure modes
  in Claude Code, Codex, Cursor, Aider, Cline, Continue, OpenHands, Goose, Gemini CLI, Amp.
- [Practitioner evidence](prompt-caching/03-practitioner-evidence.md) — measured hit rates, cost
  incidents, silent misses, myths, and the status of each finding.
- [Sources and confidence](prompt-caching/04-sources-and-confidence.md) — provenance, access dates,
  confidence grades, gaps.

Agent-caching documents:

- [Index and findings](agent-caching/README.md) — the decisions the corpus supports and the
  corrections it found.
- [Matrix](agent-caching/matrix.md) — seven agents x seven strategies at a glance, plus the provider
  contract status and a build / measure / don't-build triage.
- `agent-caching/agents/` — one document per harness.
- `agent-caching/strategies/` — one document per strategy class: prefix caching, deterministic
  ordering, tool and scratchpad output caching, semantic caching, speculative/subagent caching,
  tool/function cache, exact-request cache.

## From research to crate

| Research finding | Where it lives in `warmpath` |
| --- | --- |
| Provider cache contracts: mode, TTLs, read/write multipliers, model-dependent minimums, strict unknown routes | `cache.rs` |
| Reported versus absent token fields; inclusive, exclusive, and partitioned prompt shapes | `prompt.rs` |
| Harness output shapes, usage paths, input shapes, session/resume commands, per-harness TTLs | `profile.rs` |
| Exact-prefix matching, canonical serialization, byte-offset comparison, canonical tool surface | `prefix.rs` |
| Cache identity over prefix, harness, provider, model, effort, session, tools; named break reasons | `cache.rs` |
| AICS and EEPR gate semantics, with cold runs excluded rather than zeroed | `gate.rs` |
| The TTL is a step, not a curve; age-band warm-resume reports; two-warm-sample floor | `calibration.rs` |
| Pre-provider savings kept apart from provider cache savings | `savings.rs` |
| Artifact names, identities, no-clobber publish, bounded reads | `artifact.rs` |

The strategy verdicts are mostly negative, and that is the useful part: only prefix caching (and the
deterministic ordering that enables it) is a cache in the money sense. The rest are stores with an
invalidation problem, anti-caches that trade context for prefix, or scheduling tricks with side
effects. `warmpath` therefore exposes prefix/identity/gate/calibration primitives and no response
cache, semantic cache, or tool-result store.

## Provenance and sync

These documents were copied from the parent project's caching research, de-branded so the crate can
publish them without exposing the private product, and are maintained here as the consumer version.
The parent product's research record remains the archive of record; this copy is the one that must
track the crate's contracts.

When refreshing a document, re-copy it from the archive and re-apply the same de-branding pass:
remove product names, private paths, tracker ids, and links into the private docs; rebase links to
this crate root; keep provider/harness names, public URLs, versions, dates, and confidence grades.
Keep the provenance blockquote at the top of every copied file. A change to a provider's documented
numbers is a `SEMANTICS_VERSION` bump, not only a doc edit.

Line-number citations (`file:line`) inside the corpus were accurate in the archive at the date shown;
the provenance header and de-branding edits shift them in this copy, so treat them as historical
pointers rather than exact offsets.

## Confidence and limits

Grades are per claim, not per document. Read [sources and confidence](prompt-caching/04-sources-and-confidence.md)
for the provider layer and [the agent-caching index](agent-caching/README.md#confidence-and-gaps)
for the harness layer. The irreducible gap is shared: no live model call and no wire capture, so
breakpoint placement, TTL behavior, and harness request assembly are documentation or source reads
until a deployment measures them.
