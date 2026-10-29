# Provider prompt and context caching mechanics

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research, 2026-10-26
- **Sources:** vendor primary documentation; see [04](04-sources-and-confidence.md)
- **Rule of thumb:** all providers cache a **prefix**; they differ in who places the
  breakpoint, what the minimum is, how long it lives, and what a write costs.

## Comparison

| Provider | Cache unit and matching | Minimum and TTL | Usage and pricing | Important consequence |
| --- | --- | --- | --- | --- |
| **Anthropic** | Explicit `cache_control`; tools → system → messages; cumulative hash; reads walk backward up to 20 blocks | Model-dependent minimum, roughly 512–4,096 tokens; 5m default or 1h; lifetime measured from request start | `input_tokens` = after last breakpoint; `cache_creation_input_tokens`; `cache_read_input_tokens`; read ~0.1× input, write 1.25× (5m) or 2× (1h) | A marker on a changing block writes every turn and reads nothing. A long tool turn can move the read past the 20-block lookback; add a second breakpoint pre-emptively. |
| **OpenAI** | Automatic prefix caching; GPT-5.6+ supports explicit `prompt_cache_breakpoint`; model, tools, developer messages, history participate | ~1,024 visible tokens on GPT-5.6+; earlier models vary; TTL 30m default / 12h optional on 5.6+, older retention modes | `input_tokens_details.cached_tokens`; cache-write fields; reads ~0.1×; writes 1.25×/2.5× on newer models; no write charge on older models | Routing is machine-local; `prompt_cache_key` affects routing but never guarantees a hit. A stable prefix followed by a volatile suffix can miss without an explicit boundary. |
| **DeepSeek** | Automatic disk cache; only exact persisted **prefix units** hit | No documented minimum or TTL; best-effort, cleared in hours-to-days | `prompt_cache_hit_tokens` / `prompt_cache_miss_tokens`; a miss is the write; cache-hit price is a small fraction of miss | A+B then A+C misses even though A is shared, until A is persisted as its own unit. Append a new message rather than editing the tail. |
| **Gemini** | Implicit prefix caching on 2.5+; explicit `CachedContent` resources on some generations | Implicit minimum ~2,048 (2.5) or ~4,096 (3.x/4); explicit cache TTL defaults to 1h | `usage.total_cached_tokens`; explicit caches charge storage for their TTL plus cached-input pricing | Explicit caches are immutable and model-locked. Implicit caching is opportunistic. |

## Anthropic details

- `cache_control` marks a content block; the cached prefix runs from the start through that
  block. Up to four breakpoints per request.
- Cache writes occur at breakpoints; reads search backward over prior writes with a 20-block
  lookback. A run of consecutive `tool_use` or `tool_result` blocks counts as one position.
- TTL is 5 minutes by default; 1 hour is opt-in and costs 2× for the write. The lifetime is
  measured from the start of the request that writes or reads, not after the response finishes.
- Parallel requests do not share a freshly written entry until the first response starts.
- Isolation differs by surface: Claude API is organization- and workspace-isolated; some cloud
  surfaces are organization-level only.
- The minimum is per model. A marked prefix below the minimum is silently uncached.

## OpenAI details

- Older models cache automatically on an eligible boundary. GPT-5.6+ adds explicit
  `prompt_cache_breakpoint`; the implicit boundary is near the latest eligible message.
- The visible minimum is 1,024 tokens on GPT-5.6+; hidden OpenAI system content does not count.
- `prompt_cache_key` is a routing hint, not a lease. The docs warn that high request rates can
  overflow to another machine; a common practical ceiling cited around 15 requests/minute per
  key. From GPT-5.6, the API routes cache traffic itself and the key is less important.
- A request that extends an existing message rather than appending a new one can destroy the
  prior endpoint.
- Cache writes are not free on newer models; treat a write-heavy path as a cost bug.

## DeepSeek details

- The disk cache persists prefix units at request boundaries (end of user input, end of model
  output), at detected common prefixes, and at fixed token intervals for long inputs/outputs.
- A hit requires a full match of a persisted unit. "Same beginning" is not enough.
- There is no separate write charge: a miss is billed as uncached input.
- The cache is best-effort and has no TTL guarantee; it is usually cleared within hours to days.
- `user_id` isolates KV cache scheduling and privacy; keep it stable per end user.

## Gemini details

- Implicit caching is on by default for 2.5+ and needs no code; it is opportunistic.
- Explicit caching creates a `CachedContent` resource with immutable `model`, `contents`,
  `systemInstruction`, `tools`, and `toolConfig`; the request references it by name.
- Explicit caches have a TTL (default one hour) and a storage fee; they are a poor fit for a
  short burst that never idles.
- Gemini CLI's docs state token caching is available for API-key and Vertex AI users, not OAuth
  users; that is a harness limitation, not a provider impossibility.

## What actually invalidates a prefix

Common across providers:

- changing the model or route;
- changing tool names, descriptions, schemas, or order;
- rewriting an earlier message or tool result instead of appending;
- inserting volatile content before a breakpoint;
- changing the system prompt or project context;
- changing reasoning/effort/speed settings where the provider keys them into the cache;
- a gateway that strips or fails to translate cache markers;
- a cold resume after the TTL.

Provider-specific:

- Anthropic: breakpoint placement, 20-block lookback, 1h-vs-5m ordering constraints;
- OpenAI: implicit vs explicit boundary, machine routing, `prompt_cache_key` churn;
- DeepSeek: non-unit-aligned prefixes, editing the tail;
- Gemini: explicit resource immutability and model lock.

## Verification corrections (2026-10-27)

The first edition compressed several model-dependent facts into one number. The corrections below are
what `warmpath`'s `cache::semantics` encodes; a change to any of them is a semantics-version bump.

- **Anthropic read cost is model-dependent, and so is the minimum cacheable length.** The earlier
  table's flat "read ~0.1×" is a *summary*, not a single contract: current generations cluster around
  0.1×, but the minimum cacheable prompt differs by model family (roughly 1,024–4,096 tokens; Opus 4.x
  is at the high end, Sonnet 4.x lower, Haiku 2,048). `warmpath` keeps a per-model table with the
  defaults it can source instead of one global number.
- **Write cost is not one multiplier.** A 5-minute ephemeral write is 1.25× fresh input and a
  1-hour write is 2×; a warm reuse therefore saves `w − r` per cached token (avoided write minus the
  read actually paid), which is the counterfactual `warmpath` reports. A read-only saving (`1 − r`)
  overstates the benefit of a resume that would not otherwise rewrite the prefix.
- **An `automatic` `cache_control` on some surfaces consumes one of the four cache slots** without
  an explicit marker. Treating the slot budget as "four explicit breakpoints plus anything automatic"
  over-counts by one; a harness that adds an automatic marker has three left.
- **Gemini explicit caching is legacy-only for the current generation, and implicit caching has a
  `>=2050`-token rule.** The earlier "implicit on 2.5+, explicit on some generations" line mixed the
  two mechanisms: explicit `CachedContent` resources are not the supported path on current
  generations, and an implicit cache below 2,050 tokens is not eligible. `warmpath` models the
  implicit cache as automatic, unknown-TTL, with a 2,050-token minimum.
- **A resume is not automatically warm.** A resumed session can rewrite the whole prefix
  (`#40524`: 305,735 cache-write tokens in one turn) when the system/project layers differ. Resume
  changes *which conversation* continues; only a measured cache read shows that a prefix was served
  from cache. The warm decision therefore keeps identity and TTL as gates, not guarantees, and the
  realized/expected calibration is reported separately from the estimate.
