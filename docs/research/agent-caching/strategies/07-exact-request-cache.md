# Exact request cache

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research, 2026-10-28
- **Scope:** caching a **model response** against a byte-identical request, for coding-agent dispatch:
  the four layers it could sit on, all seven harnesses, and the consequences for `warmpath`. Other
  reuse mechanisms appear only where contrasted.
- **Method:** local files (agent docs, prompt-caching research, `warmpath` source, and one deployment's
  request and ledger paths); 5 web sources (2 fetched, 3 snippet-only); access date 2026-10-28.
- **Companion:** [index](../README.md) · [matrix](../matrix.md)

## 1. What the strategy is (and is not)

An **exact request cache** stores a model response under a hash of the *whole request body* and, when
the same body arrives again, returns the stored response **without calling the model**. Nothing is
generated, no KV cache is read, no provider is billed.

| Mechanism | Model runs? | Reused | Where the evidence lives |
| --- | --- | --- | --- |
| **Provider prefix cache** | **yes** — new tokens still generated | KV state for an identical *prefix* | `cache_read_input_tokens` / `cached_tokens`, a **billing** fact ([01](../../prompt-caching/01-provider-mechanics.md)) |
| **Exact request cache** (this doc) | **no** | the response body | nothing — the request never happened |
| **In-process response dedupe** | no, once per process lifetime | a promise/result map | not in any surveyed harness |
| **Dispatch-level idempotency** | **yes** | the *decision to dispatch* | claim rows, idempotency keys (§6) |

Row 1 is the one confused with row 2. A prefix cache recomputes nothing but **still generates**; a 100%
"cache hit" on an identical request is a 100% *prefix* hit, not a replayed answer
([grok](../agents/grok.md):43). Row 2 skips the generation, so its usage is not small — it is absent
(§4). Row 4 is legitimate and different: deduping **dispatch requests** (claim a work item once, post the
state comment once) is a control-plane guarantee that the model still runs.

## 2. How it works

**The key.** A hash over the whole request body. LiteLLM's exact-match backends "key on a hash of the
whole request" and, on `/v1/responses`, "key on the request body, so a request that carries
`previous_response_id` is a different key from the one that inlines the conversation"
([LiteLLM](https://docs.litellm.ai/docs/proxy/caching)); Portkey builds its key "from the request body
and URL" ([Portkey](https://docs.portkey.ai/docs/product/ai-gateway/cache-simple-and-semantic)). A safe
key includes model id, route/provider, the full message array with tool results, tool schemas **and
order**, temperature/top_p/seed/max_tokens, reasoning effort, response format, stop sequences, the
stream flag, and tenant scope. Each omission is a silent collision bug.

**Storage and TTL.** Redis/Valkey is the production default; disk, SQLite, S3/GCS and per-process memory
exist. TTL is a *deployment choice*, never a provider fact — LiteLLM exposes `ttl`/`s-maxage` per
request. The closest documented analogue is Stripe's idempotency layer, which prunes keys "at least 24
hours old" and replays the saved status and body
([Stripe](https://docs.stripe.com/api/idempotent_requests)). No request invalidates an entry; it
expires.

**Why a coding agent's request is almost never byte-identical twice.** The request *is* the
conversation, and the conversation is append-only by contract
([01](../../prompt-caching/01-provider-mechanics.md), `#40524` correction). Request N+1 carries at least
one new message, tool result or assistant turn, so its hash differs by construction. The harnesses also
embed volatile state: Codex sends `cwd` and environment context ([codex](../agents/codex.md):45),
Claude Code opens with cwd/platform/shell plus a git-status snapshot ([claude](../agents/claude-code.md):§3),
DSH sends a per-request `sessionId` ([dsh](../agents/dsh.md):36-38). Byte identity is the accident, not
the steady state.

The one *natural* duplicate is **a retry of the same turn** — the same body re-sent after a transport
failure, and exactly when a replay is most dangerous (§5). A dev loop is the only other case.

| Layer | Offers it? | Evidence |
| --- | --- | --- |
| **Provider** | **No.** OpenAI's `store` + `previous_response_id` is *conversation state addressed by an id*, not a content hash — a handle, not a dedupe key; Codex hardcodes `store: false` ([codex](../agents/codex.md):14), and a WS continuation body keys differently from the same conversation inlined (LiteLLM, above). What retention does under `store:false` is **undocumented** ([codex](../agents/codex.md):41). | §3 |
| **Gateway / proxy** | **Yes, shipped and common.** LiteLLM (Redis/S3/local/disk + semantic), Portkey (simple = "exact match on input"), `llm-cache-proxy`. LiteLLM's doc says semantic mode "goes badly wrong on agentic traffic". **A pre-provider compressing proxy is not one**: its per-run store expires with the run, so it is not a durable response cache. | fetched docs |
| **Harness** | **No.** All seven report none (§3). | agent docs |
| **Dispatch plane** | **No.** A dispatch plane keeps a claim, a session key, a warm-prefix lease and a ledger of *observed* requests — no response store. | local |

## 3. Cross-agent evidence

| Agent | Response cache | Retry replay | Idempotency surface | Evidence | Grade |
| --- | --- | --- | --- | --- | --- |
| **claude** | none documented; resume re-sends and re-reads, `/rewind` truncates to a warm prefix | none — `--resume` continues, never replays a body | `--session-id` (assignable up front), `--resume` | [claude](../agents/claude-code.md):36, :104, :171 | medium (absence; closed source) |
| **codex** | none; a turn-scoped `last_request` exists **only** to diff the next WS delta, dropped with the turn | continuation, not replay: `exec resume <id>`; `previous_response_id` on WS only | `thread_id`; `prompt_cache_key` — a routing hint, explicitly not a lease | [codex](../agents/codex.md):29, :35, :37, :71 | high (source-verified) |
| **agy** | none at the model layer; every "cache" identifier in the binary is a file cache | none; `--input-format stream-json` is one conversation, many prompts | `conversation_id` | [agy](../agents/agy.md):61, :76 | high |
| **grok** | no response cache; a "full cache hit" is a 100% prefix hit | none; `previous_response_id` named as the *stateful alternative* to replaying reasoning bytes | `x-grok-conv-id` (routing affinity) | [grok](../agents/grok.md):43, :67-70 | high (prefix hit) / unknown (harness half) |
| **dsh** | none; every step sends a **full** request, no `previous_response_id` in any adapter | none; the invariant is "anyone holding the log reconstructs every loop request byte-for-byte" | `sessionId` (body field + header, model-hidden) | [dsh](../agents/dsh.md):30, :83 | high (source-verified) |
| **kimi** | none documented; identical-request replay is not a feature | none | `-s/--session-id`, `-r/--resume` | [kimi](../agents/kimi.md):41 | high that it is undocumented |
| **perplexity** | none on any API surface; consumer-app identical-query reuse is **folklore** | none | `prompt_cache_key` (routing hint) | [perplexity](../agents/perplexity.md):49, :62, :125 | absent, medium |

Four nuances survive the table. **(1) Codex's `last_request` is not a cache** — one turn's request,
held for transport diffing so the next frame can be a delta, never consulted for an answer
([codex](../agents/codex.md):71). **(2) DSH is the strongest negative datum** — every step is a complete
request because the design guarantees byte-level reconstructability from the log
([dsh](../agents/dsh.md):83); a response cache would be a second derivation path beside the log, the
exact thing that invariant forbids. **(3) Perplexity's consumer reuse is folklore**
([perplexity](../agents/perplexity.md):49, :125). **(4) "Replay" is already taken**: these docs use it
for replaying prompt *bytes* so the provider can re-read them ([claude](../agents/claude-code.md):62;
[dsh](../agents/dsh.md):48) — the *opposite* strategy, which re-sends in order to re-read and still
generates. A design note using "replay" must say which one.

## 4. Measured evidence and economics

No source here measures a coding-agent response cache, because none of the seven has one. What follows
is derived from recorded runs and the accounting types, and is labelled as derivation.

**A hit is worth real money.** A replayed request pays nothing — no fresh input, no cache-read, no
output. The recorded Claude batch spent **$1.7278 on 8,639,036 cached tokens alone**
([claude](../agents/claude-code.md):§4), so replaying one such request avoids that read plus its output
share.

**But the hit rate is ≈ 0.** §2: consecutive turns never repeat. The only byte-identical pairs are {a
retransmitted turn, a deliberately re-run prompt}; in a dispatch plane the first is the retry path,
which a dispatch plane already handles by *continuing* the captured session — paying the read rate
rather than zero, but keeping the generation real.

**And the evidence loss is total.** A replayed step has no usage triple. `warmpath` derives `fresh()` and
`expected()` from the prompt half of `UsageReport` (`src/usage.rs`:125-150), and `Reported<T>`
is deliberately **two-valued** — `NotReported` or `Reported(T)`, no `Default`, no `Deref` — because a
provider that prints `cached: 0` has measured zero while one that prints nothing has measured nothing
(`src/reported.rs`:1-16). A replay is neither: the request did not happen. `Reported(0)` claims
a measured zero-token request; `NotReported` is indistinguishable from a harness that printed nothing.
**The type cannot express the third state**, and either encoding is a lie that propagates into
the cache ledger (a replayed step produces no entry, so the array under-counts and the ledger no
longer explains the run), into `classify_cold` (a replay is neither `Warm` nor any cold class —
[`cache.rs`](../../../../src/cache.rs):449-471), and into a cost summary whose contract is that an
unrecorded figure reads `unavailable`, never `0`.

**Verification is bypassed, and it cuts both ways.** A run succeeds only when the child exits 0, **the
tree changed** (when the plan declares files), and every verification command exits 0. A replayed text-only response changes no tree, so the
run fails *after* its cost was "saved" — the saving buys a false failure. With tool calls it is worse:
the agent re-executes them against a tree that already holds the first attempt's edits.

**A retry is exactly the wrong moment for a stale answer.** A dirty-checkout retry policy defaults to
refusing the retry, so at retry time the world has usually moved while the request bytes either (a) moved too — a miss and no
saving — or (b) did not, giving a hit and an answer computed against a tree that no longer exists.

## 5. Risks and failure modes

1. **The zero-triple accounting hole** (§4) — the only failure that is silent *and* permanent: it
   corrupts the denominator every cache metric divides by and leaves no ledger entry to attribute the
   step.
2. **A key that omits a field** — seed, temperature, tool order, route, tenant. Two different requests
   collide; the wrong answer is served with no error.
3. **Staleness at retry** — the likeliest hit is the most dangerous one (§4). The TTL is a config value
   with no provider contract behind it, so a twenty-minute-old hit and a two-second-old hit look alike.
4. **Shared-store disclosure** — a response cache stores the request, file contents and tool output
   verbatim; a multi-tenant Redis is a cross-tenant read surface. (Risk, not measured here.)
5. **Gateway mechanics** — an in-memory cache "lives inside one worker process, so a proxy running four
   workers keeps four separate caches and the hit rate drops roughly by the worker count" (LiteLLM); and
   a 0% hit rate invites "just add embeddings", which that doc says "goes badly wrong on agentic
   traffic" — the failure being a fluent, on-topic answer to a neighbouring question
   ([perplexity](../agents/perplexity.md):104-107).

## 6. Verdict for `warmpath`

**Do not build a model-response cache for coding dispatch.** The hit population is byte-identical
retries, which warm resume already covers while keeping the generation real; the accounting cannot
represent a replay ([`reported.rs`](../../../../src/reported.rs):11-16); and a hit is likeliest exactly
when a stale answer is most expensive.

**Keep and strengthen the three legitimate cousins**, clearly separated:

- **(a) Single-flight coalescing of concurrent identical requests.** No staleness: every coalesced
  caller wants the *current* answer and it is computed once, now. A dispatch plane may need none today — at
  most one in-flight run per slot and an exclusive claim — but single-flight is the right mechanism if a
  fan-out warm-up barrier ships: serialize the first request, coalesce the rest, never replay.
- **(b) Dispatch-level idempotency.** A claim is taken "unless a live one already exists"; a warm-prefix
  lease's granted variant is documented as idempotent on repeat, with a test; and external writes carry a
  client-minted UUIDv4 when the write must be idempotent. This is the pattern to grow.
- **(c) Prefix-cache warm resume.** Re-sends, re-reads, still generates — already measured, and not a replay.

**If a proxy in the chain offers replay, ensure it is off, or measure it separately.** A deployment
running LiteLLM/Portkey with `cache: true` intercepts requests before its dispatcher sees them, and the
usage triple arrives pre-zeroed; any new proxy stage must answer the question explicitly. If replay is
ever enabled for a dev loop, treat it like the pre-provider savings counters: a **separate stage** whose
tokens never enter `cached` or `fresh`, plus a distinct outcome class (`replayed`) so a step reports
"not measured" rather than a false zero.

## 7. Confidence and gaps

| Claim | Confidence | Why |
| --- | --- | --- |
| No surveyed harness has a model-response cache | **high** for codex/dsh/agy (source or docs-verified absence), **medium** for claude/kimi (closed source) | §3 |
| No provider sells one; gateways ship exact-match replay hashed on the whole body | **high** for the code path (Codex `store:false`, DSH sends everything) and for the gateways; **unknown** what OpenAI retains under `store:false` | [codex](../agents/codex.md):41; LiteLLM + Portkey, fetched |
| A coding agent's requests are almost never byte-identical | **high** for the append-only argument; **medium** that no two seeds of one turn collide in practice (unmeasured here) | 01, agent docs |
| A replay corrupts the accounting | **high** — a direct reading of `Reported<T>` and the ledger's explanation rule | `reported.rs` |
| A replay would fail the tree-change gate | **high** for text-only responses; **medium** for tool-call responses (duplicated side effects inferred) | the run contract |
| Per-hit saving magnitude | **medium** — derived from list-price arithmetic, not a measured replay | claude-code.md §4 |

Gaps: (1) no replay was run against any harness, so every §5 failure mode is a reading of the contract,
not an observation; (2) the seven agent docs grade this absence claim inconsistently — claude and
perplexity **medium**, codex/dsh/agy/kimi **high** — so §3 quotes grades per agent, and `perplexity.md`
drifts internally ("absent, medium" in its table, "absent" in §7); (3) whether any gateway in the
deployment has replay on was **not checked** — a sweep found idempotency and dedupe machinery and no
response-cache call site; (4) LiteLLM/Portkey under `stream: true` and tool-calling is unfetched.

## 8. Sources

- [LiteLLM — Caching](https://docs.litellm.ai/docs/proxy/caching) — fetched 2026-10-28 — exact-match
  backends hash the whole request body; `/v1/responses` keys on the body, so `previous_response_id`
  differs from an inlined conversation; per-worker memory splits hit rate; semantic mode is wrong for
  agentic traffic.
- [Portkey — Cache (Simple & Semantic)](https://docs.portkey.ai/docs/product/ai-gateway/cache-simple-and-semantic)
  — snippet only, 2026-10-22 — simple = exact match on input, enabled per config or per request.
- [Portkey — Caching concepts](https://mintlify.wiki/portkey-AI/gateway/concepts/caching) — snippet only,
  2026-03-02 — key built "from the request body and URL", checked before the provider call.
- [Stripe — Idempotent requests](https://docs.stripe.com/api/idempotent_requests) — fetched 2026-10-28 —
  saved status and body replayed on retry (including 500s); keys pruned at ≥24h; parameter mismatch
  errors; 409 on concurrent reuse.
- [llm-cache-proxy (PyPI)](https://pypi.org/project/llm-cache-proxy/0.1.1/) — snippet only, 2026-05-17 —
  localhost replay cache keyed on "(same method, path, body)", marketed for dev loops.
- **Local (2026-10-28):** [claude](../agents/claude-code.md):20, :36, :62, :104, :171 ·
  [codex](../agents/codex.md):14, :29, :35, :37, :41, :45, :71 · [agy](../agents/agy.md):61, :76 ·
  [grok](../agents/grok.md):43, :67-70 · [dsh](../agents/dsh.md):30, :36-38, :48, :83 ·
  [kimi](../agents/kimi.md):41 · [perplexity](../agents/perplexity.md):49, :62, :104-107, :125 ·
  [01](../../prompt-caching/01-provider-mechanics.md):5-15, :103-107 ·
  [03](../../prompt-caching/03-practitioner-evidence.md):12-28 · `src/reported.rs`:1-16 ·
  `src/usage.rs`:125-150 · `src/cache.rs`:319-398, :449-471.
