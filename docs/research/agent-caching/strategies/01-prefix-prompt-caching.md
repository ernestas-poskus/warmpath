# Prefix / prompt caching (context re-use)

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research, 2026-10-28
- **Scope:** the reuse mechanism every coding-agent provider offers — an exact match on a request's leading bytes — across provider, harness-assembly, and gateway layers; its economics; the seven harnesses a dispatch plane may run; and what it implies for `warmpath`.
- **Method:** local files (below); the seven [agent docs](../agents/); the [provider-mechanics series](../../prompt-caching/README.md); 4 web sources (2 searches, 2 fetches, both fetches region-gated); access date 2026-10-28.
- **Companion:** [index](../README.md) · [matrix](../matrix.md) · [provider mechanics](../../prompt-caching/01-provider-mechanics.md)

## 1. What the strategy is (and is not)

The provider stores model state for a **byte-identical leading sequence** and reuses it on the next
request that starts with those bytes. The match is exact: a change anywhere in the prefix recomputes
everything after it, and there is "no per-file or per-segment caching"
([claude](../agents/claude-code.md) §1, high: vendor page).

**A hit is a billing fact, not a reduction.** The tokens are still sent and still counted; only the
rate changes. `warmpath` draws the boundary: pre-provider stages are "a reduction", while
provider-side cache savings "are a billing fact, not a reduction"
([DESIGN](../../../../docs/DESIGN.md):76-81).

| Layer | What is stored | What a "hit" is | Who decides |
| --- | --- | --- | --- |
| Provider (prefix/KV) | Model state for a prefix, keyed by serialized bytes; the unit is a breakpoint (Anthropic, OpenAI explicit), a provider-chosen boundary (OpenAI implicit, Gemini, Kimi), or a persisted prefix unit (DeepSeek) | The next request's leading bytes match an entry; usage reports read tokens and the read rate applies | Provider, except where a marker names the position |
| Harness assembly | **Nothing** — only session state (transcript, recorded prompt, session id, TTL bucket) that makes a prefix *stable* | A *claim* that the prefix is unchanged; confirmed solely by usage fields | Harness |
| Gateway | Usually nothing; possibly an undocumented response cache, or rewritten bytes | Unobservable unless the gateway reports provider usage; the bad case is a 200 with markers stripped | Gateway owner |

It is **not** a response cache (a repeat prompt is still billed — [grok](../agents/grok.md) §3), a
semantic cache, a tool-result cache, or a productivity metric (§4).

## 2. How it works

**Assembly order, stable first:** tools/system → project context → append-only history → volatile
tail. Claude Code documents that layering and the rule — "static content first, dynamic content
last"; DSH derives it from an append-only log with prefix stability as "corollary #1, not the
headline".

**Four mechanisms, which are `warmpath`'s `CacheMode`**
([cache.rs](../../../../src/cache.rs):28-40):

- `Marker` — the client sends a directive and the provider caches through the marked block.
  Anthropic's `cache_control` also has a **top-level automatic form**: one `cache_control`
  at the body root makes the API place the breakpoint on the last cacheable block and move it
  forward; it spends one of four slots, is 5m by default, 1h at 2× (medium: vendor snippets; fetch
  region-gated).
- `Automatic` — the provider notices a repeated prefix: OpenAI implicit breakpoint, DeepSeek disk
  prefix units, Gemini implicit, Kimi implicit, Grok exact prefix from `messages[0]`.
- `ExplicitBreakpoint` — the client chooses positions: OpenAI GPT-5.6+ `prompt_cache_breakpoint`,
  which Perplexity forwards. No route here declares it.
- `Strict` — no marker, no TTL, unit multipliers, no savings claim; every unknown provider *and* a
  known provider with an unaddressed model ([cache.rs](../../../../src/cache.rs):203-212).

**The append-only rule is the portable contract.** All providers accept "previous request plus new
bytes at the end". Rewriting an earlier message invalidates from that point, so append-only is the
only shape that can hit by construction. `warmpath` encodes the consequence as a static-parts digest
plus a byte-offset comparison naming the first difference and whether it is tail-confined
([prefix.rs](../../../../src/prefix.rs):70-95, 141-147).

## 3. Cross-agent evidence

Grades are for the cache claim in that row, not for the agent.

| Agent | Mechanism | Who places / sets it | Declared TTL | Usage field(s) | Documented vs inferred | Grade |
| --- | --- | --- | --- | --- | --- | --- |
| [claude](../agents/claude-code.md) | `Marker` (block-level; now also top-level automatic) | Claude Code; provider for the automatic one | 5m; 1h on a subscription main conversation (2× write) | `cache_read_input_tokens`, `cache_creation_input_tokens`, `cache_creation.{ephemeral_5m,1h}` | vendor page + blog (high); breakpoint count/placement undocumented | high / unknown |
| [codex](../agents/codex.md) | `Automatic` | OpenAI; harness only sets `prompt_cache_key` (session-derived) and keeps the front stable | 30m; 12h option unconfirmed | `cached_input_tokens`; `cache_write_tokens` on newer models | source + tests at `rust-v0.164.0` (high); provider guide 404'd | high / medium |
| [agy](../agents/agy.md) | `Strict` in-repo; Gemini implicit upstream | Nobody — no marker surface in the CLI | none published | `usage.cache_read_tokens` (since 1.1.8) | CLI + changelog (medium-high); a hit never measured | medium |
| [grok](../agents/grok.md) | `Automatic` | xAI; harness may send `x-grok-conv-id` / `prompt_cache_key` as an affinity hint | none published | `prompt_tokens_details.cached_tokens` / `input_tokens_details.cached_tokens`; no write field | three fetched primary pages (high); CLI assembly unread | high / unknown |
| [dsh](../agents/dsh.md) | `Automatic` (DeepSeek disk prefix cache) | Nobody — no marker, no `prompt_cache_key`; head kept stable, prompt changes appended in history | none; best-effort, cleared in hours to days | `prompt_cache_hit_tokens` / `prompt_cache_miss_tokens` → `cacheReadTokens` / `cacheWriteTokens` | source-verified harness (high); TTL unknown | high / unknown |
| [kimi](../agents/kimi.md) | `Automatic` (`mode = implicit`); top-level `cache_control.ttl` picks a TTL, not a position | Moonshot; client chooses `5m` or `1h` | 5m default, 1h opt-in; locked at first write, renewed on hit | per API: `prompt_tokens_details.{cached,cache_write}` or Anthropic-compat `cache_read_input_tokens` | fetched primary guide (high); block size/minimum unpublished | high / unknown |
| [perplexity](../agents/perplexity.md) | `Strict` in-repo; gateway forwards OpenAI-dialect controls | Perplexity for the Agent API's shared prefix; caller may set key/options/breakpoints | contradictory: enum `30m` vs prose `5m`/`1h`/`24h` | `prompt_tokens_details.cached_tokens` / `cache_write_tokens` (inclusive) | OpenAPI schema (high); TTL/minimum unresolvable | medium / unknown |

Where this document disagrees with the agent docs:

- **Anthropic is not explicit-only.** `claude-code.md` §3 says "the CLI places the marker" and infers a
  budget of "four minus at least one". With the top-level automatic mode the *provider* can be the
  placer, spending a slot without choosing a position; the mid-conversation-block claim stands, the
  only-placer framing does not.
- **Codex's TTL is weaker than stated.** `codex.md` §6 says "OpenAI documents 30 min", but its own
  method note says the guide 404'd — second-hand. The current reference snippet calls 30m "the only
  supported value", leaving `cache.rs`'s 12h `long_ttl` uncorroborated; verify, and change only with a
  `SEMANTICS_VERSION` bump.
- **Grok's input shape is contested.** `grok.md` §6 argues `Exclusive` is wrong (a full hit has
  `cached_tokens == prompt_tokens`, a subset), while one deployment's accounting
  encodes `fresh = input`. One biases every Grok share; one raw usage object settles it.

## 4. Measured evidence and economics

**Multipliers** (cost against fresh input, read/write): Anthropic 0.1× / 1.25× (5m) or 2× (1h);
OpenAI 0.1× / 1.25×–2.5× on newer models, none on older; DeepSeek 0.1× / 1.0× (a miss *is* the
write); Gemini 0.25× / 1.0×; Kimi 0.1× / 1.0× (5m) or 2.0× (1h); Grok 0.25× (4.7) or 0.2× (build)
with no write line. Encoded at [cache.rs](../../../../src/cache.rs):214-257.

**TTLs and minimums are the weak columns.** Declared TTLs cluster at 300 s (Anthropic, Kimi) and
1,800 s (OpenAI); DeepSeek, Gemini, Grok, and agy publish none. Minimums: 1,024 (OpenAI), 2,050
(Gemini), 1,024–4,096 model-dependent (Anthropic), unpublished (Kimi, Grok, DeepSeek, Perplexity).

**Cold starts and writes are where the money is.** Every first turn pays full price or a write
premium, and every invalidation re-pays it. The resume counterfactual is `cached × overlap × (w − r)`
— avoided write minus read actually paid — not `1 − r`, which overstates a resume that would not
otherwise rewrite ([DESIGN](../../../../docs/DESIGN.md):54-61).

**Steady state is high by construction.** In an append-only conversation the reusable prefix grows
faster than the tail. Measured: Claude 99.998 % on a three-run batch; Codex 96.73 % (20,281,216 cached /
685,887 fresh); DSH 99.24 %; the seven-run Claude batch 93.05 % **with `warm=false` on every run**
([claude](../agents/claude-code.md) §4, [codex](../agents/codex.md) §4, [dsh](../agents/dsh.md) §4) —
most observed reuse is intra-run, not warm resume, the smaller lever.

**Why hit ratio alone is not a success metric.** A ratio can be raised by re-sending more history, by
pooling cold starts into it, or by counting a write as a read. Hence two contracts: **AICS** —
token-weighted `cached / (cached + fresh)` over all measured requests, cold included — answers "how
much prompt traffic did the provider serve from cache"; **EEPR** — `served / expected` over warm runs
only, cold excluded and reported separately — answers "when a warm run existed, how much of the
expected prefix was served" ([gate.rs](../../../../src/gate.rs):6-15). The EEPR gate is
≥99.70 % eligible expected-prefix, cold excluded, not zeroed. 99 % AICS with 60 % EEPR is a
prefix-shape bug.

## 5. Risks and failure modes

Four invalidators matter most for a dispatch plane:

1. **Tool surface.** Definitions are prompt bytes; add/remove/reorder/edit invalidates from the tool
   block onward — Claude's rule is "never add or remove tools mid-session", Codex calls it
   `tools_changed`. Mitigation is deferral: Claude's `defer_loading` stubs, Codex's always-on
   `tool_search`. Deferral is not universal: agy's tool block is host state.
2. **Model / effort / speed.** Caches are per model; `/model` only warns while warm. Effort is
   cache-safe on Claude's 5.5-class models but not on Bedrock/Agent-Platform/gateway surfaces, and
   Perplexity's effort levels can move model, system prompt, and tools *under a stable key*. Including
   effort in the identity is conservative.
3. **Rewriting history.** Compaction, pruning, image offload, in-place tool-result trims, a
   `cch=`-style hash substitution. Codex's local compaction is the shipped example — it sends
   `tools: []` and `parallel_tool_calls: false`, so it cannot hit; remote v2 appends a trigger.
   DSH's summarizer is the counter-example: it replays the prompt, tools, and history, then appends
   the directive.
4. **Volatile content before the static layer.** A timestamp, a fresh worktree path, a git-status
   snapshot, today's date, a per-turn todo block. Claude's prefix is scoped to one machine *and one
   directory*; Perplexity's frozen-config examples prepend `Today is {date}`; Codex puts `cwd` inside
   the cached contextual message — exactly the per-work-item worktree shape a dispatch plane can create.

Two more corrupt the reading: a **gateway** that strips or mistranslates a marker (silent, 200,
visible only in usage), and a **usage-shape error** (inclusive vs exclusive input, an absent field
zeroed). `warmpath` excludes both: `Reported<T>` has no `Default`.

**The rule that follows:** *only append; if bytes must change, declare a new series, batch every
rewrite into one request, and keep the stable front intact.* `warmpath` already refuses to call a
changed prefix warm, and a resume is refused when the prefix, model, effort, provider, workspace,
plan, or TTL moved. What remains is making append-only construction *enforced* rather than
observed.

## 6. Verdict for `warmpath`

**Already implemented — do not rebuild.** `CacheMode`/`CacheContract`/`SEMANTICS_VERSION` with
strict-unknown and no-model-strict ([cache.rs](../../../../src/cache.rs):26, 203-259);
`CacheIdentity` over prefix+harness+provider+model+effort+session (`:317-390`); step-TTL cold
classification; warm resume with refusal on any moved identity fact ([warm.rs](../../../../src/warm.rs)); AICS/EEPR as data with exported exclusions
([gate.rs](../../../../src/gate.rs)); length-delimited digest plus canonical tool surface
([prefix.rs](../../../../src/prefix.rs):70-95, 174-191); eligible expected-prefix gate with
cold excluded, not zeroed; pre-provider savings kept out of provider savings.

**Remaining work.**

- **Semantics:** record Anthropic's top-level automatic mode as a documented surface (it
  changes *who places*, not the multiplier); settle the OpenAI 30m-vs-12h question before anyone
  quotes a 12h saving; treat Kimi/Grok/Perplexity as *unsourced families*, not permanent strict
  defaults — each addition is a `SEMANTICS_VERSION` bump.
- **Assembly and compaction:** compaction as a same-prefix side call. DSH already does it
  (`compaction-basic/src/summarizer.ts`); side calls (reviewer, second opinion, conflict repair)
  should replay the routed request's prefix or be labelled a separate prefix. Tool deferral is the
  second lever: a canonical tool manifest, never mid-run.
- **Prefix invariance:** the cross-provider test is a *serialization* test: assert request N is a
  byte prefix of N+1 under each declared mode, failing on the first differing block. Fixtures must
  include a timestamp, a tool reorder, an effort change, and a compaction call — the four failures the
  agent docs actually observed; it runs offline, the cheapest gate in the set.

**Must not be built.** No mechanism can "guarantee" a hit: routing, eviction, and provider changes
are outside the client, and every vendor that addresses it says so. No response cache, semantic
cache, tool-result cache, or pre-warm of another process's prefix (no documented surface exists for
the last). No breakpoint injection on automatic providers, no retry to "get a hit", and no gating or
ranking on hit ratio — the gate is EEPR, with AICS beside cold-start cost, first-turn fresh input,
and writes per accepted outcome.

## 7. Confidence and gaps

| Claim | Grade | Cheapest measurement that closes it |
| --- | --- | --- |
| Exact-prefix matching, append-only assembly, the four invalidators | high | none; vendor primary + three source reads |
| Multipliers and declared TTLs | high / medium | one `-p` run per provider, raw usage object |
| Anthropic top-level automatic mode | medium | one request through a local proxy (a compressing proxy already terminates the base URL); capture body + `usage.cache_creation` bucket |
| OpenAI 12h long TTL on current models | medium / low | one request with `ttl: 12h`; the error or usage settles it |
| Claude Code breakpoint count and placement | unknown | one captured request body (`cache_control` is not in transcripts) |
| agy ever hitting | unknown | two runs, frozen request, second `--conversation <id>`, require `cache_read_tokens > 0` |
| Grok cache share | unknown | one headless grok run, raw JSON per turn |
| Kimi block size / minimum | unknown | bisect a fixed prefix, watch `cached_tokens` |
| Perplexity TTL | low (the contradiction is high) | four requests at 0, 31m, 61m, 25h with `include_usage` |
| A compaction side call actually hits | medium | compare write tokens on the compaction request vs the next turn |

The largest gap: **no captured request body exists for any harness here**, so every breakpoint claim
is documentation or inference; one metadata-only proxy capture per harness would close more than more
searching.

## 8. Sources

Web (accessed 2026-10-28; snippets only where marked):

- [Prompt caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching) — Anthropic, undated live page, snippet only (both fetches returned a region-gate body), accessed 2026-10-28 — supports: top-level automatic `cache_control`, last-cacheable-block placement, one of four slots, 5m default / 1h at 2×, 400 when four explicit breakpoints exist.
- [Compact a response — OpenAI API reference](https://developers.openai.com/api/reference/go/resources/responses/methods/compact) — OpenAI, snippet only, accessed 2026-10-28 — supports: implicit breakpoint by default, `prompt_cache_breakpoint` explicit mode, ≤4 writes per request, latest 80 breakpoints, `ttl` default 30m described as the only supported value.
- [Amazon Bedrock User Guide — prompt caching](https://docs.aws.amazon.com/pdfs/bedrock/latest/userguide/bedrock-ug.pdf) — AWS, undated, snippet only, accessed 2026-10-28 — supports: GPT-5.6+ implicit and explicit caching; GPT-6.1 Sol at 1.25× write / 0.05× read on that route.
- [strands-agents/harness-sdk#3573](https://github.com/strands-agents/harness-sdk/issues/3573) — GitHub, 2026-07-29, snippet only, UGC, accessed 2026-10-28 — corroborates only that Anthropic's automatic mode exists; not the authority for any number.

Local (2026-10-28; each supports the clause after it):

- Agent docs (2026-10-28): [claude](../agents/claude-code.md), [codex](../agents/codex.md), [agy](../agents/agy.md), [grok](../agents/grok.md), [dsh](../agents/dsh.md), [kimi](../agents/kimi.md), [perplexity](../agents/perplexity.md) — each supports the mechanism, TTL, usage fields, and invalidators in its §3-§6.
- [01-provider-mechanics.md](../../prompt-caching/01-provider-mechanics.md):8-79 and [02-harness-practices.md](../../prompt-caching/02-harness-practices.md):38-57 — comparison table, invalidation list, the three reusable patterns.
- [DESIGN.md](../../../../docs/DESIGN.md):39-61, 76-81 and [cache.rs](../../../../src/cache.rs):26, 203-259, 279-332, 393-469 — semantics as data, TTL step, `(w − r)`, pre-provider boundary, contract table, break reasons, identity, cold classification.
- [prefix.rs](../../../../src/prefix.rs):70-95, 141-147, 174-191 and [gate.rs](../../../../src/gate.rs):6-15, 287-329 — digest, append-only difference, canonical tool surface, AICS/EEPR definitions.
