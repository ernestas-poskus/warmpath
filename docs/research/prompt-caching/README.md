# Prompt and context caching research

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research complete, 2026-10-26
- **Scope:** provider prompt/context caching, harness implementations, practitioner evidence, and the implications for `warmpath`
- **Method:** three parallel research subagents — provider mechanics, harness implementations, practitioner evidence — plus local code and measurement review
- **Companion:** [coding-agent caching strategies research](../agent-caching/README.md) and the [sources and confidence](04-sources-and-confidence.md) record

## Decision this research supports

Adopt one cache-safety contract across the request path:

1. The cache unit is an exact byte prefix. Anything that changes before a breakpoint
   invalidates everything after it.
2. Static content goes first; append-only history next; volatile turn content last.
3. A session keeps one provider/model/effort/speed/tool surface for its lifetime unless
   a deliberate new series is declared.
4. Compaction and pruning are declared cache breaks, never accidental history rewrites.
5. Every stage records fresh input, cache reads, cache writes, and cache breaks separately;
   cache-read volume is not a productivity metric.

## What was done

- Read the vendor primary documentation for Anthropic, OpenAI, DeepSeek, and Gemini.
- Read the public Claude Code caching documentation, Cursor's token-efficiency post, Aider's
  caching guide, and the relevant OpenHands, Goose, Continue, Cline, Codex, and Gemini CLI
  source/issues.
- Read local implementation and measurement evidence:
  - `docs/subsystems/system-prompt.md`, `packages/core/agent-loop/README.md`,
    `packages/compaction/compaction-basic/README.md`,
    `packages/compaction/compaction-tool-result-pruner/README.md`, and
    `docs/subsystems/token-meter.md` in the DSH checkout;
  - one deployment's request assembly, cache eligibility and gates, cold-start and session
    handling, and usage parsing;
  - that deployment's cost model, warm-resume calibration report, and cost ledger.
- Cross-checked measured claims against vendor docs and source. Reddit was not directly
  fetchable (HTTP 403/429); Reddit-derived claims are not treated as primary evidence.

## Findings index

| Document | Covers |
| --- | --- |
| [01 — provider mechanics](01-provider-mechanics.md) | cache units, minimums, TTLs, pricing, usage fields, and invalidation rules |
| [02 — harness practices](02-harness-practices.md) | Claude Code, Codex, Cursor, Aider, Cline, Continue, OpenHands, Goose, Gemini CLI, Amp |
| [03 — practitioner evidence](03-practitioner-evidence.md) | measured hit rates, cost incidents, silent cache misses, and myths |
| [04 — sources and confidence](04-sources-and-confidence.md) | provenance, access dates, confidence, and gaps |

## The five findings that matter most

1. **Exact-prefix, not semantic.** Claude Code's own words: "The match is exact, so a change
   anywhere in the prefix recomputes everything after it. There is no per-file or per-segment
   caching." Every provider has a variant of the same rule.
2. **The steady-state hit rate is high by construction.** In an append-only conversation the
   unchanged prefix grows faster than the new tail; measured sessions routinely reach 95–99.9%.
   Cost problems therefore concentrate in cold starts, cache writes, and invalidations, not in
   the steady-state read share.
3. **The common failure is silent.** A timestamp, random UUID, git-status snapshot, tool reorder,
   model switch, or gateway that drops `cache_control` turns the next request into a full
   uncached re-read. There is no API error for it; only the usage fields show it.
4. **Compaction is a cache decision, not only a context decision.** The best implementations
   repeat the system prompt, tools, and history as a side call and append the instruction; a
   fresh summarizer prompt or a changed tool list forfeits the prefix.
5. **One cache shape is optimal everywhere.** Append-only request assembly is accepted by every
   provider's cache model; provider-specific breakpoint placement is a small declared layer on
   top, not a reason to reimplement request assembly per provider.

## Confidence

- **High:** vendor docs; Claude Code/Codex/Continue/Goose source and issues; measured
  deployment cost/ledger data.
- **Medium:** Cursor and Amp internals (official prose, no source); fast-moving Codex `main`.
- **Low / folklore:** blanket multipliers, Reddit claims that could not be fetched, and claims
  that partial prefix overlap hits.
