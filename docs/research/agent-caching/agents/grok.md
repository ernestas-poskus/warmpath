# Grok (xAI) — caching behaviour

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research, 2026-10-28
- **Scope:** the xAI provider cache as documented for the Grok API (`grok-4.7`, `grok-build-0.1`),
  plus what is knowable about the Grok Build harness (`grok` CLI) that a deployment configured with a `grok`
  harness would run. Client/gateway caches are named as a separate layer.
- **Method:** 12 local files read ([prompt-caching 01](../../prompt-caching/01-provider-mechanics.md)–[02](../../prompt-caching/02-harness-practices.md),
  [04](../../prompt-caching/04-sources-and-confidence.md), [warmpath DESIGN](../../../../docs/DESIGN.md),
  `src/{cache,profile}.rs`, a deployment's profile and cache configuration, and its cost model §9 and
  usage ledger); 6 web sources searched, 3 fetched; access date
  2026-10-28. No Grok binary or credential on this host.
- **Companion:** [index](../README.md) · [matrix](../matrix.md)

## 1. What it is and where caching lives

| Layer | What it is | Cache status here |
| --- | --- | --- |
| (a) xAI provider | Grok API at `api.x.ai` (`grok-4.7`, `grok-build-0.1`, …) | **Documented**: automatic exact-prefix cache, read-only discount, `cached_tokens` usage field |
| (b) Grok Build harness | the `grok` CLI (TUI, `-p` headless, ACP), default model Grok 4.7 | **Unknown**: Apache-2.0 Rust since 2026-07-15, so inspectable, but not inspected here |
| (c) Client / gateway | OpenRouter (`x-ai/grok-4.7`), LiteLLM, proxies, editor forks | **Unverified**: header/marker survival is gateway-specific; one proxy datapoint in §4 |
| (d) Folklore | vendor hit-rate claims, forum numbers | one vendor claim (§4); no forum evidence used |

The deployment's lane is (b) on (a): a grok-configured harness maps to `grok --always-approve` headless argv,
the `grok` provider reads the X Subscription pool percentage, and [`profile.rs`](../../../../src/profile.rs)
already declares the route **strict** with no warm TTL. The open question is therefore not whether
xAI caches — it does — but whether the CLI keeps a prefix stable and tells the API which conversation
it is. Provider-side the mechanism is one sentence: messages from the start of the array that match a
previous request exactly are served from cache, with no marker, breakpoint API, or opt-in. Affinity is
a **hint** — `x-grok-conv-id` (Chat Completions) or `prompt_cache_key` (Responses) — described as
routing requests to the same server.

## 2. Strategy table

| Strategy | Mechanism observed | Evidence | Grade |
| --- | --- | --- | --- |
| Prefix / prompt caching (context re-use) | automatic, exact prefix from `messages[0]`; `cached_tokens` reported; cached rate; conv-id hint | fetched xAI docs (3 pages) + price card | **high** (provider mechanism, fields) |
| Deterministic ordering | provider: front-load static, append only; harness: no doc, source unread | xAI Best Practices snippet; the harness's open-source existence (secondary) | **medium** / **unknown** |
| Tool & scratchpad output caching | nothing at (a) or (b); a changed tool block breaks the prefix | xAI exact-prefix rule; no tool-cache page found | **none documented** |
| Semantic caching | nothing documented at the provider layer; the harness layer was not inspected | xAI docs pages read; no harness doc | **none documented** (provider) / **unknown** (harness) |
| Speculative / subagent execution caching | nothing documented; subagent sharing unstated | no doc; source unread | **unknown** (not "none") |
| Tool / function cache | no function-result cache; only the prefix consequence of a stable surface | xAI docs | **none documented** |
| Exact request cache | no response cache; "full cache hit" on an identical request is a 100 % prefix hit | xAI Usage & Pricing table | **high** (that it is a prefix hit) |

## 3. Findings by strategy

### Prefix / prompt caching (context re-use)

- **Automatic, no marker.** "The xAI API performs prompt caching automatically." No `cache_control`
  analogue, no breakpoint budget, no write API — so a harness can only help by keeping bytes stable.
- **Two nested usage shapes.** Chat: `usage.prompt_tokens_details.cached_tokens`; Responses:
  `usage.input_tokens_details.cached_tokens`. `0` = miss; equal to `prompt_tokens` = full hit. **No
  cache-write field** exists in either shape (unlike Anthropic's `cache_creation_input_tokens`).
- **Read discount only.** The billing table lists non-cached prompt, cached prompt, completion, and
  reasoning tokens — no write line. At the 2026-10-09 price card `grok-4.7` is 2.00 / 0.50 / 6.00 per
  1M (short context) and 4.00 / 1.00 / 12.00 at ≥200 k prompt tokens; `grok-build-0.1` is
  1.00 / 0.20 / 2.00. Documented read multiplier: **1/4** (`grok-4.7`), **1/5** (`grok-build-0.1`).
- **Long-context pricing counts cached tokens.** A 200 k-token prefix that is 95 % cached still bills
  at the long-context rate. A Grok-specific trap absent from the other providers in
  [01](../../prompt-caching/01-provider-mechanics.md).
- **Not guaranteed.** The docs state caching "is not 100% guaranteed", that entries can be evicted
  under memory pressure, and that requests "may be routed to different servers"; the conv-id exists
  to bias that routing — the same shape as OpenAI's `prompt_cache_key`, a hint and never a lease.
- **Undocumented: TTL and minimum.** None of the pages read states a retention window or a minimum
  cacheable length. They may exist; they are not published, so a dispatch plane must not invent them.
- **Reasoning models need their own bytes back.** The What-Breaks-Caching page (snippet) calls a
  missing `reasoning_content` replay "the top cause of cache misses", with `previous_response_id` as
  the stateful alternative: the harness must replay encrypted reasoning verbatim.
- **The harness half is open.** Whether `grok` sets `x-grok-conv-id` per session, reuses one id across
  `--resume`, or sends `prompt_cache_key` is **undocumented**; the source makes it answerable, not answered here.

### Deterministic ordering

Provider advice is explicit and generic: front-load static content, never edit/remove/reorder earlier
messages, append only — the same append-only contract as every provider, so the
[three reusable patterns](../../prompt-caching/02-harness-practices.md) apply unchanged.

What the harness does is **unknown**: layer order, whether per-turn state (cwd, git status, todo list)
sits before or after the stable prefix, whether compaction rewrites history in place. Grok Build's docs
describe sessions (`~/.grok/sessions`, `-s/--session-id`, `-r/--resume`, `-c/--continue`) but not
request assembly, and a session id surviving `--resume` is only a plausible carrier for
`x-grok-conv-id` — inference, not evidence.

### Tool & scratchpad output caching

Nothing documented at either layer. The only cache-relevant fact about tools is negative — a changed
name, description, schema, or order breaks the exact prefix, by the provider's own definition of a
hit. No page describes caching tool *results* or *scratchpad* content (thinking, plans, summaries) as
reusable artifacts. Treat such claims as folklore until a source appears.

### Semantic caching

**None documented** at (a) or (b); prefix matching is the only mechanism described. Local caution:
a normalized-text routing cache stores **routing decisions** across normalized task
text and quota buckets — a local in-process decision cache that never touches a prompt and must not be
counted as Grok reuse.

### Speculative / subagent execution caching

**Unknown, not "none".** Nothing describes speculative reuse or subagent-level cache sharing. Two
things decide the design later: whether parallel subagents share a conv-id (making a shared prefix
warm) and whether a subagent opens a new conversation id (cold-starting it). The X Subscription pool
is shared across Grok Build/Cursor/Chat/Imagine/Voice/Bot, but that is **quota**, not cache — a shared
pool does not imply a shared cache.

### Tool / function cache

No function-result cache documented. The ordinary rule applies: a stable tool surface is a cache
prerequisite, not a cache. The deployment's Grok argv already varies by dispatch (model/effort injection),
a prefix invalidator if xAI keys either in — undocumented, and treated as an invalidator in
[01](../../prompt-caching/01-provider-mechanics.md).

### Exact request cache

None. An identical resend can produce `cached_tokens == prompt_tokens` (the docs call it "rare"), but
that is the prefix cache returning a complete hit; the completion is still generated and billed, so no
output-side call can be saved this way.

## 4. Measured / quantified evidence

| Datapoint | Value | What it measures | Grade |
| --- | --- | --- | --- |
| Vendor hit-rate claim, `grok-code-fast-1` (2025-08-28) | "regularly achieving cache hit rates above 90% when used with our launch partners"; that model priced 0.20 / 1.50 / 0.02 per 1M | vendor marketing over unnamed partner workloads, no token counts; its 1/10 cached ratio is **not** `grok-4.7`'s 1/4 | **low** as a number, **high** as a claim and as a dated price |
| Grok lane cost (arithmetic) | $1.70/run at `grok-4.7` rates on the seven-run Codex shape (96.73 % cached); ≈ $6.04 at 0 % | arithmetic on a **borrowed** token shape, not a Grok run | **medium** arithmetic, not measurement |
| Third-party proxy log, 2026-10-02 | Grok CLI on a non-xAI backend: 13.6 % cached vs Codex 97.1 % on the same model, `new_bind` on all 88 | CLI affinity **through a proxy to another provider**; says nothing about xAI's own cache | **low** for xAI, **medium** for "affinity is not always pinned" |
| Deployment Grok runs | **0** | no credential, no binary, no run row; `command -v grok` empty, `~/.grok` absent today, matching the 2026-10-23 ledger note | **high** (no measurement exists) |

Honest summary: *Grok's own cache share is
unmeasured here.* Sensitivity is large — 96.73 % of prompt tokens assumed cached means a 0 %-cache run
costs ~3.6× the projection — so cached share is the most valuable Grok measurement, above pool size.

## 5. Failure modes and invalidators

Provider-documented (high): editing/removing/reordering an earlier message; omitting
`reasoning_content` when replaying a reasoning model; no stable conversation id (or a fresh UUID per
request) causing server hops; eviction and ordinary routing; crossing the long-context threshold with
cached tokens counted; a "prefix" that is not one (per-turn cwd/timestamp/todo block inserted first).

Layer-specific and unverified: **gateway hops** — OpenRouter/LiteLLM sit between harness and provider,
and the pattern in [01](../../prompt-caching/01-provider-mechanics.md)/[02](../../prompt-caching/02-harness-practices.md)
is that a gateway may drop the affinity header, so a hit rate measured through one is not a provider
number (medium). **Auth-mode split** — a third-party client derives `x-grok-conv-id` from
`prompt_cache_key` only on the API-key path, not OAuth; if Grok Build shares that split (unverified),
the X Subscription login and the `XAI_API_KEY` route could differ in affinity (low/medium).
**Resume ≠ warm** — `--resume`/`-c` continue a session on disk; whether the request reuses the conv-id
is unverified, and Claude Code's `#40524` full-prefix rewrite is why it must be measured. A model or
effort change mid-session, if xAI keys those in, is assumed to invalidate it.

## 6. Consequences for `warmpath`

**Today's correct behaviour is what the code already does: strict, no TTL, no savings claim.**

- `warmpath::cache::semantics` has no `xai`/`grok` family, so `("grok", …)` resolves strict with
  `CacheTtl::Unknown` and unit multipliers (asserted by the `("grok","grok-5")` case in a
  cache-identity test). Keep it: there is **no documented TTL or minimum** to encode,
  and inventing one lets the warm decision admit a cold resume. If xAI is ever added, the honest
  contract is *automatic + unknown TTL + read 1/4 or 1/5 + unknown minimum + no documented write
  charge* — and that is a `SEMANTICS_VERSION` bump, not a quiet table edit.
- `warmpath::profile::grok()` sets `warm_ttl: None` and `CacheClass::Strict`, and
  the classifier marks Grok `Unreported`, so the prefix gate **excludes** Grok runs from numerator and
  denominator rather than scoring 0 %. That is the right failure direction; do not "fix" it by
  guessing a class.
- **The profile's usage paths are probably wrong — fix first after a real run.**
  `src/profile.rs` declares flat `usage.cached_input_tokens` and
  `usage.cache_creation_tokens`; xAI documents nested `usage.prompt_tokens_details.cached_tokens` /
  `usage.input_tokens_details.cached_tokens` and has **no** creation field. The CLI's own
  `--output-format json` envelope is a separate unverified shape (the recorded cost model, §9.6 step 5). If
  unchanged: reads parse as `NotReported`, `cache-gate` likely reports `NoData` (exit 5), and the ledger
  keeps saying "unmeasured" after a run.
- **`InputShape::Exclusive` also looks wrong.** The docs equate `cached_tokens` with `prompt_tokens`
  for a full hit, i.e. cached is a **subset** of the input count — `Inclusive` in warmpath's
  vocabulary. Under `Exclusive`, fresh and read are counted disjoint when they are not, overstating
  fresh input and understating cached share on any reporting run (medium-high).
- **Minimum measurement, one dispatch:** the raw JSON usage object verbatim; whether `cached_tokens` is nested where the docs say; per-turn `cached_tokens`
  across a session (monotone growth ⇒ append-only, flat/zero ⇒ a break, and the break is the
  finding); whether `--resume` starts at the previous high-water mark; the pool percentage delta
  (the recorded cost model, §9.6). Two numbers close the gap: cached share of prompt tokens, and of a resumed run.
  And no warm-session bet yet: back-to-back Grok scheduling is a spend decision on an undocumented
  TTL, so the only defensible rule is "consecutive if at all", plus a measured cold/warm split.

## 7. Confidence and gaps

| Claim class | Confidence | Why |
| --- | --- | --- |
| automatic prefix caching; usage field names; cached billing; long-context rule | **high** | three fetched primary pages |
| no documented write charge / TTL / minimum | **high** as "not documented", **unknown** as behaviour | pages and snippet omit them; absence is not proof |
| `grok-4.7` / `grok-build-0.1` prices and read ratios | **high** | xAI pricing page (snippet, 2026-10-09) + a 2026-10-23 price check |
| affinity is a hint, not a lease | **high** | docs: not guaranteed, eviction, routing |
| provider advice (append-only, front-load, replay reasoning) | **medium-high** | read as search snippets, not fetched |
| Grok Build is open source (Apache 2.0, Rust) and inspectable | **medium** | two secondary write-ups agree; not confirmed from GitHub itself |
| what the CLI sends, in what order, with which conv-id | **unknown** | no doc page describes assembly; source unread |
| subagent sharing; tool/scratchpad/semantic/exact-response caches | **unknown / none documented** | nothing found; distinction stated per row |

**Could not verify, and what was checked.** The binary is absent (`command -v grok` empty, `~/.grok`
missing), so no version or `--help`; the Homebrew formula named `grok` is jordansissel/grok, an
unrelated log tool, and the npm names were 404 at the 2026-10-23 ledger. The Grok Build source was
**not** read — the fetch budget went to the three provider pages — so every harness-level claim is
"unverified", not "checked and absent"; `x.ai/news` and `docs.x.ai/developers/pricing` were reachable
only as snippets. The search index renders docs.x.ai titles as "SpaceXSI Docs" while the fetched bodies
are ordinary xAI API docs, so only body content is relied on. No source read says anything about cache
**isolation** (per organization, per key, per conv-id) — a real gap for a multi-account plane.

## 8. Sources

- [Prompt Caching](https://docs.x.ai/developers/advanced-api-usage/prompt-caching) — xAI docs, updated 2026-03-16, accessed 2026-10-28 — supports: caching is automatic, exact match from the start of the array, reduced cached rate, `x-grok-conv-id` recommended.
- [How It Works](https://docs.x.ai/developers/advanced-api-usage/prompt-caching/how-it-works) — xAI docs, updated 2026-03-16, accessed 2026-10-28 — supports: prefix definition; "not 100% guaranteed"; eviction and server routing.
- [Usage & Pricing](https://docs.x.ai/developers/advanced-api-usage/prompt-caching/usage-and-pricing) — xAI docs, updated 2026-05-10, accessed 2026-10-28 — supports: both `cached_tokens` paths; miss/hit/full-hit table; cached billing and long-context rule; no write-charge line.
- [Best Practices & FAQ](https://docs.x.ai/developers/advanced-api-usage/prompt-caching/best-practices) — xAI docs, 2026-05-09, accessed 2026-10-28 (snippet) — supports: stable conversation id, never modify earlier messages, front-load static content, handle misses.
- [What Breaks Caching](https://docs.x.ai/developers/advanced-api-usage/prompt-caching/multi-turn) — xAI docs, 2026-10-09, accessed 2026-10-28 (snippet) — supports: any earlier-message change breaks the cache; missing `reasoning_content` is the top cause of misses.
- [API Pricing](https://docs.x.ai/developers/pricing) — xAI docs, 2026-10-09, accessed 2026-10-28 (snippet) — supports: `grok-4.7` 2.00/0.50/6.00 and 4.00/1.00/12.00 long at ≥200 k; `grok-build-0.1` 1.00/0.20/2.00.
- [Grok Build overview](https://docs.x.ai/build/overview) — xAI docs, accessed 2026-10-28 (snippet) — supports: `grok` CLI, headless and ACP entry points, custom models via `~/.grok/config.toml`.
- [Headless & Scripting](https://docs.x.ai/build/cli/headless-scripting) — xAI docs, 2026-06-09, accessed 2026-10-28 (snippet) — supports: `-p`, `-m`, `-s/--session-id`, `-r/--resume`, `-c/--continue`, `--output-format`, sessions under `~/.grok/sessions`.
- [Grok Build is now open source](https://appwrite.io/blog/post/grok-build-open-source) — Appwrite blog, 2026-07-15, accessed 2026-10-28 — supports: Apache-2.0 open-sourcing of the Grok Build harness.
- [xAI open-sources Grok Build after privacy disclosure](https://dataphoenix.info/news/xai-grok-build-open-source) — Data Phoenix, 2026-07-16, accessed 2026-10-28 — supports: Rust-based terminal coding-agent harness published under Apache 2.0 on 2026-07-15.
- [Getting Started — xai-org/grok-build](https://deepwiki.com/xai-org/grok-build/1.1-getting-started) — DeepWiki (derived index), 2026-09-19, accessed 2026-10-28 — supports: repository layout and the `xai-grok-pager` binary shipped as `grok`.
- [Grok Code Fast 1](https://x.ai/news/grok-code-fast-1) — xAI announcement, 2025-08-28, accessed 2026-10-28 (snippet) — supports: vendor >90 % hit-rate claim with launch partners; that model's 0.20/1.50/0.02 pricing.
- [Issue: Grok Build on ChatGPT-backed OpenAI models gets almost no prompt-cache reuse](https://github.com/lidge-jun/opencodex/issues/6481) — third-party issue, 2026-10-02, accessed 2026-10-28 — supports: proxy-side observation of low cached share and `new_bind` on every request.
- [Commit: prompt-cache affinity — `x-grok-conv-id` from `prompt_cache_key`](https://github.com/lidge-jun/opencodex/commit/a7a99d3f3ff00ac70a04f76372cd85681c52f6f5) — third-party commit, 2026-06-17, accessed 2026-10-28 — supports: the header is the affinity lever and a client may set it only on the non-OAuth path.
- Local: [provider mechanics](../../prompt-caching/01-provider-mechanics.md), [harness practices](../../prompt-caching/02-harness-practices.md), [sources and confidence](../../prompt-caching/04-sources-and-confidence.md), `src/{cache,profile}.rs`.
