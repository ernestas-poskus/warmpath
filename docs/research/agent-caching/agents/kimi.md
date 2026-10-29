# Kimi (Moonshot AI) — caching behaviour

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research, 2026-10-28
- **Scope:** the Kimi coding-agent surface — Kimi Code CLI (`kimi` 0.36.0, installed) and the
  Kimi API models it calls (K3 series; K2 series legacy), plus the Anthropic-compatible endpoint
  third-party harnesses point at Kimi. Provider billing stays separate from harness behaviour: no
  source documents the CLI's request assembly, so that layer is graded lower.
- **Method:** local files read ([01](../../prompt-caching/01-provider-mechanics.md), [02](../../prompt-caching/02-harness-practices.md),
  [04](../../prompt-caching/04-sources-and-confidence.md); [DESIGN](../../../../docs/DESIGN.md); `src/cache.rs`; one
  deployment's cost model, dispatcher notes, and ledger); a grep for `kimi|moonshot` returned **zero**
  hits across the research corpus and the crate; `kimi --version` = `0.36.0` and `kimi --help` read; 1 page
  fetched + 9 further web sources via search results; access 2026-10-28.
- **Companion:** [index](../README.md) · [matrix](../matrix.md)

## 1. What it is and where caching lives

**`kimi-cli`** was the open-source Python CLI (Apache-2.0, `pip`, last release 1.52.0); its repo is
archived and read-only, pointing at **Kimi Code CLI** (`MoonshotAI/kimi-code`). The installed binary is
the successor: `kimi 0.36.0`, home `~/.kimi-code/`, with a `migrate` subcommand for legacy data.

Models moved past the earlier assumption. **Kimi K2.5** shipped 2026-01-27 but has no row in the
current price list, so a 2026-10 integration never calls it. That list carries **K3** (`kimi-k3`,
`kimi-k3.1-pro`, `kimi-k3.1-pro-highspeed`, 1,048,576-token window) and **K2 legacy** (`kimi-k2.6`,
`kimi-k2.7-code`, 262,144 ctx, retiring end of 2026) — treat "K2.5" as historical.

Caching has exactly **one** layer in the sources: Moonshot's server-side **context cache** on
repeated request prefixes; no client-side, gateway-side, semantic, or exact-response cache is
documented. Since the 2026-09 upgrade **cache write is a separate line item** with two TTLs (`5m`,
`1h`), supported only by `kimi-k3.1-pro` and `kimi-k3` (the FAQ excludes `kimi-k2.7`, `kimi-k2.6`).

## 2. Strategy table

| Strategy | Mechanism observed | Evidence | Grade |
| --- | --- | --- | --- |
| Prefix / prompt caching (context re-use) | Automatic implicit prefix cache on `/chat/completions`, `/responses`, `/anthropic/v1/messages`; `prompt_cache_options.mode` supports only `implicit`; TTL via `prompt_cache_options.ttl` or top-level `cache_control.ttl`; separate cache-write billing on K3 | Caching guide (fetched); pricing page; Messages reference | high |
| Deterministic ordering | Provider *advice* to front-load system prompt, tool definitions and reference material, tail-load turns and tool results; no published statement that the CLI enforces append-only assembly | Guide §"How to improve cache hit rates" (high); third-party analysis (low) | medium (advice) / low (harness) |
| Tool & scratchpad output caching | Tool results ride in the message tail and are cached only as part of the prefix once sent; no separate tool-result store or dedupe documented | Caching guide scenario table; nothing in CLI docs | medium (prefix) / none (dedupe) |
| Semantic caching | None documented — no embedding or normalized-task reuse in API or CLI docs | Absence across guide, pricing, Messages reference, CLI docs | high that it is undocumented |
| Speculative / subagent execution caching | None documented. K2.5 marketing describes parallel "agent clusters"; the CLI has background tasks and `/fork` (an independent copy); no source claims shared or speculative cache for child work | Release coverage (low); CLI sessions doc | unknown |
| Tool / function cache | None documented — no tool-definition hash, compiled-tool cache, or function-result cache | Absence | high that it is undocumented |
| Exact request cache | None documented — identical-request replay is not a feature; the prefix cache is the only reuse path | Absence | high that it is undocumented |

## 3. Findings by strategy

### Prefix / prompt caching (context re-use)

- **Automatic and implicit, no markers.** "The Kimi API automatically enables context caching billing
  for repeated request prefixes", default TTL `5m`. `prompt_cache_options` takes
  `{"mode": "implicit", "ttl": "5m" | "1h"}` and `mode` "currently supports only `implicit`". Not
  Gemini-style `CachedContent` resources, not Anthropic-style breakpoints.
- **Unit is a prefix block; size unpublished.** "Cache is stored in blocks. A portion smaller than one
  full block cannot be written to cache; it counts as a cache miss." No block size and **no minimum
  token count** appears on the caching guide, the pricing page, or the Messages reference — nothing
  primary lets a caller compute a minimum cacheable prefix.
- **Position-sensitive and org-scoped.** "When any part of a prefix changes, the content after that
  position cannot be reused." Isolation is "within an organization, not across organizations"; manual
  clearing is unsupported.
- **Two independent caches, TTL locked at first write.** `5m` and `1h` "do not share entries"; the TTL
  is fixed by the first write, hits renew it free under the same TTL, and a new TTL requires full
  expiry first. A `5m` entry that keeps being hit never becomes `1h`. Expiry is an **inactivity**
  window: "a `5m` entry expires after at least 5 minutes of inactivity".
- **Pricing (kimi-k3, per 1M tokens, fetched table):** input miss **$3.00**; Cache Write `5m`
  **$3.00**; Cache Write `1h` **$6.00**; Cached Input **$0.30** — read **0.1×**, 5m write **1.0×**,
  1h write **2.0×**. The split did not change total cost ("The write cost was already included in the
  input price"): a written token bills as a write *instead of* fresh input.
- **Three usage shapes, not interchangeable.** Chat Completions reports total in
  `usage.prompt_tokens` with `prompt_tokens_details.{cached_tokens,cache_write_tokens}`; Responses
  mirrors it on `usage.input_tokens`; read, write and uncached are "mutually exclusive and sum to
  total input tokens" (inclusive). The Anthropic-compat Messages shape is **exclusive**:
  `usage.input_tokens` excludes `usage.cache_read_input_tokens` and `usage.cache_creation_input_tokens`,
  with `cache_creation.ephemeral_5m_input_tokens` / `ephemeral_1h_input_tokens` splitting writes by
  TTL. Streaming needs `stream_options.include_usage=true`; headers
  `Msh-Usage-Cache-Write-Tokens-5m` / `-1h` expose writes per request.

### Deterministic ordering

For the CLI the evidence is thin: a third-party deep-dive describes ordering as system prompt →
injections (config rules, goal, plan, skill prompts) → history, with changes appended at the end of
their segment rather than inserted mid-block and in-turn changes batched. Plausible and cache-aware,
but secondary (**grade low**): no vendor statement of an append-only invariant, single assembly point,
or "old bytes are never rewritten" guarantee exists in the docs I read. The provider's own ordering
advice (stable content front, volatile tail, no timestamps in the prefix) is summarised in §2.

### Tool & scratchpad output caching

Tool results are ordinary tail messages; they join the prefix only after being sent once, so they are
cached on the *next* turn as history. Nothing documents a separate tool-output cache, dedupe, or
scratchpad reuse. The guide lists "tool definitions" as front-of-prefix material and "tool results" as
tail material: the only vendor statement about the tool surface.

### Semantic caching

None documented: no embedding store, normalized-task matching, or response memoization in the API
reference, caching guide, pricing page, CLI sessions guide, or configuration reference. Tooling
*around* the CLI exists (a "cache-keeper" skill pinging to hold a session warm; a cross-session
file-blob cache advertising Kimi Code support) — external tooling, not Kimi behaviour.

### Speculative / subagent execution caching

None documented. K2.5 launch coverage describes "agent clusters" (up to 100 sub-agents) — a
model-capability claim, not a statement that child contexts share or speculatively reuse a cache. The
CLI has background tasks (`TaskList`/`TaskOutput`/`TaskStop`) and `/fork`, an independent copy of the
session (a saved `/goal` is deliberately not copied). Whether a fork inherits a warm prefix is
undocumented; because it copies history bytes, a warm fork is plausible within the TTL and is a
measurable question.

### Tool / function cache

None documented in any layer: no tool-definition hash, compiled-tool cache, or schema cache.

### Exact request cache

None documented. Repeating an identical request is not a reuse path; the prefix cache serves an
identical prefix at the cached rate — a billing effect, not a response cache.

## 4. Measured / quantified evidence

- **Provider ratios (kimi-k3)**: read **0.1×**, write 5m **1.0×**, write 1h **2.0×** — from the fetched
  table, not inferred. `warmpath`'s resume counterfactual `w − r` is therefore **0.9** per token for a
  5m write avoided and **1.9** for a 1h write avoided.
- **1h-versus-5m breakeven, as published**: on a 1M-token prefix the extra write costs $3.00 and each
  hit saves $2.70, so two hits net $2.40 for `1h` and one hit does not. Vendor arithmetic,
  reproducible from the table, the only quantified TTL decision offered.
- **Third-party probe (one repository, own tests)**: on the Anthropic-compat `/coding/v1/messages`
  endpoint a baseline reports **24,050 `cache_read` tokens = full HIT**, and swapping `system` while
  keeping messages identical loses it — so `system` and `tools` participate in the prefix. The same
  work places the cache *below* the protocol layer and reads at a "chunk-aligned offset". **Medium/low**
  — one suite, no vendor confirmation, OpenAI-compat path unprobed.
- **No harness-level hit rate is public**, and this deployment has no Kimi data (no run rows, adapter,
  ledger entry, or measured cost), so every number here is provider-side. The instrument exists: the
  CLI's `wire.jsonl` records "the request trajectory sent to the model (tool schemas, request
  parameters, MCP tool list)", with no published reading.

## 5. Failure modes and invalidators

- **Editing earlier content** invalidates everything after that position (vendor-stated), and
  **volatile bytes in the front** — timestamps, random IDs, git state, per-turn task state before
  stable content — are explicitly warned against.
- **Tool-surface churn**: tool definitions sit in the cached prefix (vendor advice + probe), so
  toggling an MCP server or reordering tools invalidates from the tools block onward — same failure
  class as Cline/Cursor, with no documented mitigation.
- **TTL expiry at 5 minutes** is aggressive for human-in-the-loop work: an entry idle past 5 minutes is
  gone and the next turn pays a **1.0×** write on the whole prefix — cheaper than Anthropic's 1.25×,
  not free. The TTL is also **locked**: a live `5m` entry cannot be promoted to `1h` without going idle
  past 5 minutes and being rewritten, so a harness that "just adds `ttl: 1h`" gets nothing until then.
- **The Anthropic-compat trap**: `cache_control` "is effective only at the top level; the same marker
  inside the messages body is ignored", so a harness placing mid-conversation breakpoints is only
  selecting a TTL, not managing cache positions. Missing usage fields are not zeros either: without
  `stream_options.include_usage=true` a streaming response reports no breakdown, not a zero.
- **Compaction is an unmeasured write event.** Auto-compaction triggers as the context nears the
  window limit, and `/compact` accepts guidance on what to keep. If it is a new prompt rather than a
  same-prefix side call it pays a full write; nothing documents which. Largest unquantified harness
  risk, alongside cross-org resume: `--session`/`--continue` restore local history, but the cache is
  org-scoped server-side, so a resumed session in another org is a cold prefix regardless of recency.

## 6. Consequences for `warmpath`

- **Today a Kimi route is `strict()`.** `ProviderFamily::of` maps `claude|anthropic`,
  `codex|openai`, `deepseek`, `gemini|google`, everything else → `Unknown` → `CacheMode::Strict`,
  `CacheTtl::Unknown`, unit multipliers, no minimum, no savings claim. Correct default, and what any
  `kimi`/`moonshot` id currently gets. The cost: at `r = 0.1` a real Kimi prefix read is a 90%
  discount the plane will silently refuse to estimate.
- **If a `Moonshot`/`Kimi` family is added, encode only what the fetched source says**: mode
  `Automatic` (not `Marker` — top-level `cache_control` selects a TTL, it does not place a breakpoint),
  default TTL 300 s, `read_multiplier` 0.1, `write_multiplier` **1.0** (not 1.25), `long_ttl` 3,600 s
  with `long_write_multiplier` 2.0, `minimum_tokens: None` — block size and any minimum are
  unpublished, so defaulting to 1,024 would be fabrication. Keeping `strict()` for an unaddressed Kimi
  generation matches how the crate treats a known provider with an empty model.
- **`cache_write` is first-class for Kimi and the crate has no path for it.** Writes bill at 1.0×/2.0×,
  yet the provider cache facts table exposes only `usage_paths.cached`, so a Kimi run's write spend —
  plausibly dominant in a compaction-heavy session — is invisible to the cache-share metric and the cache gate. Note
  too that native Chat Completions/Responses are inclusive (read + write + uncached = total input) while
  Anthropic-compat Messages is exclusive, so one hard-coded shape mis-normalizes one of the two.
- **Warm window: 300 s is the defensible local default.** It coincides with the provider default rather
  than deriving from it, and because the option is client-selected, a plane that does not set
  `prompt_cache_options` must assume 5m. Safe both ways: never claims warmth a 5m entry cannot have,
  under-claims if the client opted into 1h; do not raise it without observing
  `Msh-Usage-Cache-Write-Tokens-1h` on the wire. Renewal-on-hit fits the TTL-as-step model — `warmpath`
  anchors at `prior_ended_at` and Kimi renews per hit, so the last request's end is the right anchor,
  and no keepalive is worth building: renewal on hit is free.
- **What to measure:** (1) hit ratio from `prompt_tokens_details.cached_tokens / prompt_tokens`; (2)
  `cache_write_tokens` per run and per request, split 5m/1h via the `Msh-*` headers, against the two-hit
  breakeven; (3) whether the CLI sets `prompt_cache_options` at all — `wire.jsonl` records request
  parameters, so this is a grep of recorded trajectories; (4) whether `/compact` rewrites the prefix —
  compare write tokens on the compaction request with the next turn; (5) the hit-rate delta of an
  MCP/tool-set change; (6) whether `/fork` and `--session` resumes warm, by `cache_read_input_tokens`.

## 7. Confidence and gaps

| Claim class | Grade | Basis |
| --- | --- | --- |
| Implicit prefix caching, 5m/1h TTLs, TTL lock, renewal-on-hit, org isolation | high | fetched primary guide |
| kimi-k3 prices (miss $3.00, write $3.00/$6.00, hit $0.30) | high | same guide, explicit table |
| Usage field names; inclusive-vs-exclusive shapes per API | high | same guide, migration table |
| `cache_control` top-level-only; in-body markers ignored; Cache Write K3-only; block storage | high | guide + Messages reference + FAQ |
| **Block size / minimum cacheable prefix** | **unknown** | unpublished on guide, pricing, Messages reference; nothing in CLI docs |
| K2.5 release date; current K2/K3 name list | medium | launch coverage for the date; the price list for the names |
| CLI sessions, resume flags, `/fork`, `/compact`, session layout | high (existence), medium (detail) | official CLI docs, read as search excerpts not a full fetch |
| CLI append-only assembly / single assembly point | low | third-party deep-dive only |
| `system` + `tools` in the cached prefix | medium/low | one third-party probe, 24,050-token baseline |
| CLI sets `prompt_cache_options`; compaction behaviour; subagent/fork sharing | unknown | undocumented; the first is measurable from `wire.jsonl` |
| Semantic / exact-response / tool caches | none documented | absence across all read sources |

Open questions worth one measurement each: the block size (bisect a fixed prefix, watch `cached_tokens`),
whether the 5m default is what the CLI gets, the write cost of a `/compact`, whether `/fork` is warm. The
CLI sessions page is quoted from search excerpts (two direct fetches 404'd) and should be re-read at
source. I did not read `~/.kimi-code/config.toml` (may hold credentials) or `~/.kimi-code/cache/`.

## 8. Sources

- [Best Practices for context caching](https://platform.kimi.ai/docs/guide/context-caching) — Moonshot AI / Kimi API Platform, updated 2026-09-28, accessed 2026-10-28 — supports: implicit prefix caching, 5m/1h TTLs, TTL lock and renewal, org isolation, kimi-k3 price table, per-API usage fields, block storage, top-level-only `cache_control`.
- [Model Inference Pricing Explanation](https://platform.kimi.ai/docs/pricing/chat) — Kimi API Platform, 2026-10-22, accessed 2026-10-28 — supports: current model list (K3 series 1,048,576 ctx; `kimi-k2.6`/`kimi-k2.7-code` 262,144 ctx), K2 retirement notice, default 5m TTL.
- [Messages API](https://platform.kimi.ai/docs/api/messages) — Kimi API Platform, accessed 2026-10-28 — supports: Anthropic-compatible endpoint, top-level `cache_control` with `type: ephemeral` and `ttl` 5m/1h, in-body markers ignored.
- [Sessions and context](https://www.kimi.com/code/docs/en/kimi-code-cli/guides/sessions.html) — Kimi Code CLI docs, accessed 2026-10-28 — supports: sessions under `~/.kimi-code/sessions/`, `wire.jsonl` request-trajectory recording, `--continue`/`--session` resume, `/new`, `/sessions`, `/fork`, auto-compaction and `/compact`.
- [Configuration files](https://www.kimi.com/code/docs/en/kimi-code-cli/configuration/config-files.html) — Kimi Code CLI docs, accessed 2026-10-28 — supports: model/provider config surface, `max_context_size` and `max_input_size` (declared per-request input limit used by compaction and overflow checks); no cache-tuning key.
- [Kimi Code CLI quick reference](https://www.kimi.ai/academy/kimi-code-cheat-sheet) — Kimi, 2026-09-14, accessed 2026-10-28 — supports: slash-command surface, `/compact`, `/fork`, background `Task*` tools, `--output-format stream-json`.
- [MoonshotAI/kimi-cli](https://github.com/MoonshotAI/kimi-cli) — GitHub, archived 2026-09-22, accessed 2026-10-28 — supports: legacy Python CLI archived and read-only, replaced by Kimi Code CLI; last release 1.52.0.
- [pi-provider-kimi-code: caching notes](https://github.com/Leechael/pi-provider-kimi-code/blob/main/docs/caching.md) — third-party, 2026-02, accessed 2026-10-28 — supports: measured probe that `system` and `tools` participate in the cached prefix (24,050-token full HIT), cache layer below the protocol layer, chunk-aligned reads; not vendor-confirmed.
- [kimi-code deep dive: context management, compaction and memory](https://cloud.tencent.com/developer/article/2723006) — Tencent Cloud developer article, 2026-08-08, accessed 2026-10-28 — supports: third-party description of kimi-code ordering (system → injections → messages) and append-at-segment-end updates; low grade.
- [Moonshot AI releases Kimi K2.5](https://en.tmtpost.com/news/7857176) — TMTPOST, 2026-01-27, accessed 2026-10-28 — supports: K2.5 release date, multimodal architecture, "agent cluster" claim.
