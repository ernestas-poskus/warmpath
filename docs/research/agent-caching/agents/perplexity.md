# Perplexity — caching behaviour

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research, 2026-10-28
- **Scope:** Perplexity's coding-relevant surfaces — the Agent API (`/v1/agent`, Responses format,
  the GA replacement for Sonar Chat Completions), the Orchestrator API
  (`/orchestrator/v1/chat/completions`, OpenAI wire, **private preview**), and the consumer
  app/Comet. Layers: (a) upstream provider cache, (b) Perplexity API-level plumbing,
  (c) Perplexity harness-side reuse, (d) folklore.
- **Method:** local files read ([01](../../prompt-caching/01-provider-mechanics.md), [02](../../prompt-caching/02-harness-practices.md),
  [04](../../prompt-caching/04-sources-and-confidence.md), `src/cache.rs`, [DESIGN](../../../../docs/DESIGN.md), and a
  deployment's cache and routing-eligibility configuration and cost model); 5 web searches,
  3 fetched pages; access date 2026-10-28.
- **Companion:** [index](../README.md) · [matrix](../matrix.md)

## 1. What it is and where caching lives

**There is no Perplexity coding-agent harness.** A grep for `perplexity` over the research corpus
and the crate returns nothing (no `sonar` either), and no Perplexity
product documents a repo-editing agent loop. Three surfaces exist:

1. **Agent API — a hosted agent harness, not a coding agent.** `POST /v1/agent` with `input`,
   `instructions`, `tools`, `max_steps` (effort `xhigh` uses 100), built-in tools (`web_search`,
   `fetch_url`, `finance_search`, `sandbox`), Skills, connectors/MCP, custom functions. Perplexity
   runs the loop and owns the **system prompt and tool definitions** for an `effort`. Code
   execution is a sandbox tool; no filesystem, repo, diff, or patch primitive is documented.
   Grade: **high**.
2. **Orchestrator API — an OpenAI-compatible gateway to third-party models**, private preview.
   Catalog ids in primary docs: `openai/gpt-6-luna`, `openai/gpt-6-sol`,
   `anthropic/claude-opus-5-5`, `anthropic/claude-sonnet-5-5`, `perplexity/kimi-k3`,
   `perplexity/glm-5.3-flash`; routes across deployments with automatic failover. Grade: **high**.
3. **Consumer app / Comet.** Q&A-shaped; no published request-assembly or cache contract, only
   low-grade secondary claims that it is "not a native multi-file editing agent". Grade: **low**.

**Summary:** the gateway documents cache controls and cache-billed usage fields; the agent harness
documents one stable cache key; neither documents unit, minimum, or a usable TTL.

- **(a) Upstream.** Effort levels run on `openai/gpt-6-*` and `anthropic/claude-*-5-5`, so the cache
  is the upstream prefix cache (OpenAI automatic, Anthropic explicit) — see
  [01-provider-mechanics](../../prompt-caching/01-provider-mechanics.md). Perplexity documents
  routing affinity to preserve it: conversations stay on one deployment "where possible", so you
  "benefit from cache-read pricing without doing anything special".
- **(b) API plumbing.** `prompt_cache_key`, `prompt_cache_options.{ttl,mode}`, deprecated
  `prompt_cache_retention`, per-part `prompt_cache_breakpoint: {mode: explicit}`, usage fields
  `prompt_tokens_details.cached_tokens` / `.cache_write_tokens`: OpenAI-dialect forwarding and
  billing, not a Perplexity-specific mechanism.
- **(c) Harness reuse.** One mechanism: dynamic effort levels "automatically use a stable
  `prompt_cache_key` for their shared prompt prefix (system prompt and tool definitions)",
  overridable per request. Profiles are the caller-owned versioned bundle.
- **(d) Folklore.** Client-side response caches, a stale "no caching" claim (Narev, undated), the
  5% estimate.

## 2. Strategy table

| Strategy | Mechanism observed | Evidence | Grade |
| --- | --- | --- | --- |
| Prefix / prompt caching (context re-use) | Upstream prefix cache; `prompt_cache_key`, `prompt_cache_options.{ttl,mode}`, `prompt_cache_breakpoint`, sticky routing; cached input billed at a per-model "Cache read" rate | gateway schema; routing-and-reliability; router/models (snippet) | high (fields), medium (semantics) |
| Deterministic ordering | Perplexity fixes system prompt + tool definitions inside an effort/profile; `instructions`/`input`/`tools` assembled server-side; exact byte order undocumented | presets (Effort); profiles | medium (stable), unknown (order) |
| Tool & scratchpad output caching | Tool results enter the loop and are re-sent; re-use is only the prefix cache above, no separate store | tools/overview; gateway schema | medium (inferred) |
| Semantic caching | None documented; third-party posts build one in front of the API | apistatuscheck; wisechecker | absent, **medium-high** |
| Speculative / subagent execution caching | No subagent primitive; `max_steps` loop; `pplx_sdk` fan-out is *inside* the sandbox | presets; tools/sandbox | absent / unknown |
| Tool / function cache | No per-tool-call cache; `web_search`/`fetch_url` use Perplexity's own index/fetch layer — internal, not billed as cache reads | tools/overview | unknown (search-side), absent (customer-visible) |
| Exact request cache | None documented for API surfaces; consumer-app identical-query reuse is folklore | — (no doc found) | absent, medium |

## 3. Findings by strategy

### Prefix / prompt caching (context re-use)

- **Orchestrator Chat Completions** (OpenAPI `version: 2.3.0`): `prompt_cache_key` — "Stable key
  grouping related requests to improve prompt-cache hit rates" — and `prompt_cache_options` with
  `ttl` and `mode` (`implicit` | `explicit`). Text, image, audio, and file parts accept
  `prompt_cache_breakpoint: {mode: explicit}`: caller-placed breakpoints in the **OpenAI GPT-5.6+
  dialect**, not Anthropic's `cache_control`. `prompt_cache_retention` is deprecated. Grade:
  **high**.
- **Usage fields** (same schema): `usage.prompt_tokens` = "Input tokens for the request,
  **including cache reads and writes**"; `prompt_tokens_details.cached_tokens` and
  `.cache_write_tokens` (both default 0) — the OpenAI-shaped pair `warmpath` already models, with an
  inclusive denominator. Grade: **high**.
- **Pricing** (Perplexity's `docs/router/models`, snippet only): cache reads bill "at the discounted per-model
  rate shown in the 'Cache read' column", writes "at the rate shown (or at the input rate where no
  dedicated write rate is listed)"; `perplexity/glm-5.3-flash` "supports cache reads but not cache
  writes". Write rates are per model and sometimes absent — not one multiplier. Grade: **medium**.
- **Agent API**: no dedicated caching page found in five searches over `docs.perplexity.ai`; only
  the effort note above. Grade: **high** for that sentence, **unknown** for TTL/minimum/usage shape.
- **TTL — self-contradictory.** The gateway prose says "Set `ttl` to `5m`, `1h`, or `24h`", while
  `PromptCacheTTLEnum` on the same page enumerates **only `30m`**: no TTL is adoptable, grade
  **low** (the contradiction itself is **high**). Minimum cacheable length is undocumented.

### Deterministic ordering

An effort level "bundles a model, search config, reasoning steps, system prompt, and available
tools"; `tools` "merge per tool instead of replacing the whole set". The prompt head is thus
Perplexity's bytes in Perplexity's order, and a tool's position in the serialized array is
undocumented. Caller `instructions` then `input` follow — stable in examples but **inferred**.
Grade: **medium** that the head is stable per version, **unknown** for the order.

### Tool & scratchpad output caching

The loop appends tool results and re-sends the conversation, so tool output is re-read through the
prefix cache; no separate scratchpad or tool-result cache exists. Each step pays cache-read on the
prefix and fresh input on the new observation. Grade: **medium** (inference from documented shape).

### Semantic caching

No Perplexity-documented semantic cache on any surface. Third-party material is about building
your own in front of the API, warning that a false hit serves "a real, fluent, on-topic answer —
to the neighbouring question": client-side advice, not a feature. Grade: **absent**, medium-high
confidence (no field or parameter implies one).

### Speculative / subagent execution caching

No subagent, branch, or speculative-execution primitive in the Agent API. What resembles one is
inside the sandbox: the `xhigh` prompt tells the model to load the `pplx_sdk` skill and use its
"parallel fan-out" patterns — Python in one sandbox, not harness subagents. Grade: **absent**.

### Tool / function cache

Two things get conflated and must stay apart. **Search/fetch result reuse**: `web_search` and
`fetch_url` run inside Perplexity's loop against Perplexity's index; any reuse there is internal,
undocumented, and not visible as cache-read tokens — grade **unknown**, and not customer-billable
as caching. **Prompt-token caching of tool definitions and results**: documented above, and the
only part that is a `CacheContract`.

### Exact request cache

No documented exact-request or exact-response cache on the API; consumer-app reuse is unverified
folklore. Grade: **absent**.

## 4. Measured / quantified evidence

No vendor-published hit rate, TTL, or multiplier in anything read here. What exists:

- One secondary aggregator reports that automatic preset caching (2026-08-22) "can reduce costs by
  approximately 5 percent … depending on cache utilisation" — a vendor claim restated, grade **low**.
- The per-model "Cache read" column is the only quantitative surface, and it is per model. Grade:
  **medium** (snippet, unfetched).

## 5. Failure modes and invalidators

- **Effort level moving under you.** A dynamic effort level is updated when evals improve, so model,
  system prompt, and tools change while your request does not: the `prompt_cache_key` stays stable
  but the **prefix bytes change** and the hit is lost. Pin `effort="fast@2026-10-16.0"` (the
  response's `effort` echoes the version) or freeze the config. The largest hazard here.
- **Per-request `instructions`, `tools`, or `model` overrides.** Caller `instructions` replaces the
  effort's system prompt and tools merge per tool — both change the prefix, and both are the
  documented way to customize. A `model` override beside `effort` moves to a different upstream
  cache; on a read-only model (`perplexity/glm-5.3-flash`) a write-heavy strategy buys nothing.
- **Frozen configs that inject the date.** The documented frozen examples compute the UTC date
  locally and prepend `Today is {current_date}.` to `instructions`, so a frozen config copied from
  them invalidates its own prefix **once per day** — same class as a git-status header.
- **Failover.** Conversations stay on the same deployment "where possible"; on provider error,
  rate limit, or timeout the request is retried elsewhere and almost certainly starts cold.
  Traffic is split even without failure, so identical prefixes do not guarantee one machine.
- **Reasoning/effort and service tier.** `reasoning.effort` and `service_tier`
  (`auto`/`default`/`flex`/`scale`/`priority`) are honored; whether the upstream keys its cache on
  them is undocumented. Grade: **unknown**.
- **Dialect mismatch.** The field is `prompt_cache_breakpoint`, not `cache_control`; for an
  Anthropic model the translation is Perplexity's and undocumented.
- **Sonar-era material is obsolete.** Sonar Chat Completions ended 2026-09-27; posts about Sonar
  caching describe a surface that no longer exists.

## 6. Consequences for `warmpath`

- **Strict is already correct and should stay.** `cache::semantics` classifies by
  `ProviderFamily` (`anthropic`, `openai`, `deepseek`, `gemini`); every Perplexity spelling falls to
  `Unknown` → `strict()`: unit multipliers, `CacheTtl::Unknown`, no minimum, no claimed savings.
  Its own rationale — "a gateway that may rewrite the request is therefore strict by construction"
  — describes Perplexity's gateway exactly. **Do not add a family here.**
- **Nothing to declare yet, for two reasons.** (i) TTL contradicts itself inside one primary page and
  the minimum is undocumented, so no contract can be sourced. (ii) The only endpoint with per-model
  cache pricing is **private preview**, which a public crate may not encode.
- **Record the upstream model, not the agent name.** `docs/DESIGN.md` requires the provider
  field on `HarnessProfile` and forbids inferring it from a harness name, so a Perplexity dispatch
  records `provider="perplexity"`, `model="openai/gpt-6-sol"`: the cache belongs to the upstream.
- **Classification already behaves.** The eligibility classifier maps only codex→`Automatic`,
  claude→`MarkerBased`, gemini→`BestEffort`; `perplexity` falls to `Unreported` and is excluded from
  the eligible-prefix count. A Perplexity run contributes no cache credit and cannot inflate the
  99.7% target. No change needed.
- **If ever adopted, the levers are version pinning, not markers.** The caller does not assemble the
  prompt here; a caller can only pin the effort version, keep one stable `prompt_cache_key`, avoid
  per-request `instructions`, freeze the tool set, and never inject a date — weaker than Codex.
- **Cheapest measurement to resolve the gap** (~4 requests, pennies): two identical
  `POST /orchestrator/v1/chat/completions` calls with a >1,024-token fixed system message and
  `stream_options.include_usage: true`, reading `prompt_tokens_details.cached_tokens` on the second;
  a third with one tool `description` changed (expect collapse → tool invalidator); a fourth with
  `prompt_cache_options.mode: "explicit"` plus one breakpoint; then repeat at 31 minutes, 61 minutes,
  and 25 hours to find which TTL the service honours. Until then, every Perplexity saving in a cost
  report is unbacked.

## 7. Confidence and gaps

| Claim | Grade | Why |
| --- | --- | --- |
| Not a coding-agent harness | high | no repo reference; no vendor doc for a repo-editing loop; only low-grade SEO claims |
| Minimum cacheable length, cache-read multiplier | unknown | undocumented; pricing page unfetched |
| `/v1/agent` usage shape for cache reads | unknown | agent-post reference not fetched; only chat-completions schema read |
| Semantic / exact-response cache | absent | no doc, field, or parameter across five searches |
| "~5% from preset caching" | low | secondary aggregator restating a vendor claim |

Gaps: (1) Perplexity's `docs/router/models` was not fetched, so no cache-read multiplier is quoted; (2)
`/api-reference/agent-post` was not fetched, so the Agent API usage object is unverified; (3) no
dedicated prompt-caching page was found in five searches, and `llms.txt` was not fetched, so one
could exist unseen; (4) translating `prompt_cache_breakpoint` into `cache_control` for
Claude-served models is undocumented; (5) the consumer app rests on one low-grade source.

## 8. Sources

- [Routing & Reliability](https://docs.perplexity.ai/docs/router/routing-and-reliability) — Perplexity, 2026-10-08, fetched, accessed 2026-10-28 — supports: sticky routing for prompt caching, cache-read pricing by reference, automatic failover, private-preview status.
- [Effort (served from the Presets path)](https://docs.perplexity.ai/docs/agent-api/presets) — Perplexity, 2026-10-18, fetched, accessed 2026-10-28 — supports: effort levels bundle model/system prompt/tools; automatic stable `prompt_cache_key`; upstream model ids; `max_steps` 1–100.
- [Create Chat Completion (Orchestrator API, OpenAPI 2.3.0)](https://docs.perplexity.ai/api-reference/gateway-chat-completions-post) — Perplexity, fetched, accessed 2026-10-28 — supports: `prompt_cache_key`, `prompt_cache_options`, deprecated `prompt_cache_retention`, per-part `prompt_cache_breakpoint`, `cached_tokens`/`cache_write_tokens`, `PromptCacheTTLEnum = [30m]` vs prose "5m, 1h, 24h".
- [Orchestrator Models & Pricing](https://docs.perplexity.ai/docs/router/models) — Perplexity, 2026-10-22, snippet only, accessed 2026-10-28 — supports: per-model "Cache read" rate, writes at input rate when unlisted, read-only model.
- [Profiles](https://docs.perplexity.ai/docs/agent-api/profiles) — Perplexity, 2026-10-21, snippet only, accessed 2026-10-28 — supports: versioned caller-owned config bundling instructions/tools/Skills/step budget.
- [Tools overview](https://docs.perplexity.ai/docs/agent-api/tools/overview) — Perplexity, 2026-10-07, snippet only, accessed 2026-10-28 — supports: the agentic loop and four tool kinds.
- [Sandbox](https://docs.perplexity.ai/docs/agent-api/tools/sandbox) — Perplexity, 2026-10-24, snippet only, accessed 2026-10-28 — supports: code execution as a tool, not a repo agent.
- [Migrate from Sonar](https://docs.perplexity.ai/docs/agent-api/migrate-from-sonar/overview) — Perplexity, 2026-10-21, snippet only, accessed 2026-10-28 — supports: Sonar Chat Completions ended 2026-09-27.
- [Perplexity turns on automatic prompt caching for Agent API presets](https://aichangewatch.com/pricing/history/perplexity-perplexity-turns-on-automatic-prompt-caching-for-agent-api-01M0KEMZGGHPNXC08ZFX809V1Z) — AI Change Watch, 2026-08-22, secondary, accessed 2026-10-28 — supports: the ~5% estimate (low grade).
- [Perplexity API Response Caching](https://apistatuscheck.com/blog/perplexity-api-response-caching-guide) — apistatuscheck.com, 2026-08-09, secondary, accessed 2026-10-28 — supports: client-side response/semantic caching is the caller's construction, not the vendor's.
- [How to Use Perplexity Pro as a Coding CLI](https://scavio.dev/tutorials/how-to-use-perplexity-cli-for-coding-2026) — scavio.dev, 2026, low, accessed 2026-10-28 — supports: the "coding CLI" pattern is glue around Comet + MCP, Q&A-shaped.
- [Perplexity provider notes](https://www.narev.ai/docs/sdk/ai-billing/reference/perplexity) — Narev, undated, stale, accessed 2026-10-28 — supports: the now-contradicted "no prompt-caching feature" claim.
- Local: [01](../../prompt-caching/01-provider-mechanics.md), [02](../../prompt-caching/02-harness-practices.md), [04](../../prompt-caching/04-sources-and-confidence.md); `src/cache.rs` (`semantics`, `strict`, `ProviderFamily`); [DESIGN](../../../../docs/DESIGN.md).
