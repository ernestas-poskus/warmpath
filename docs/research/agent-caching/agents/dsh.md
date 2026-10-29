# dsh (DeepSeek Harness) — caching behaviour

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research, 2026-10-28
- **Scope:** DSH as a harness a dispatch plane can run headless (`dsh --profile headless`). How one step's request is assembled, which layers are static, what rewrites them, and which caches exist. The DeepSeek disk prefix cache is background only — it is the billing fact this doc measures against, not DSH's own mechanism.
- **Method:** DSH implementation checkout — commit `22ddee3` (`git describe` → `dsh-v0.2.1-rc.3-1533-g22ddee3792`), root `package.json` version `0.2.1-rc.3`, HEAD dated 2026-10-23; local checkout files listed in §8; 1 web search + 1 fetch (DeepSeek's own cache guide, for provider facts the checkout cannot state); access date 2026-10-28.
- **Companion:** [index](../README.md) · [matrix](../matrix.md)

## 1. What it is and where caching lives

DSH is a Cordis plugin composition (`docs/architecture.md:23`). Profiles `web`, `headless`, `sdk`, `sdk-minimal`, `acp` stack bundles; `dsh-agent-loop` drives each turn through model requests, streamed responses, tool execution and a durable session log (`packages/core/agent-loop/README.md`). There is no separate "cache layer" anywhere: reuse is a property of how the request is *derived* from that log.

The organizing rule is explicit in the architecture note that owns request construction: **model-visible ⟺ durably referenced**, and "Prefix-cache stability is corollary #1, not the headline: an append-only log projected by a per-node pure function yields requests that are append-extensions of their predecessors whenever the header is unchanged — stability is emergent, not managed" (`.agents/notes/implemented/architecture/2026-07-05-reconstructable-requests.md:19`).

Three layers must be kept apart when reading DSH:

1. **Provider (DeepSeek) disk prefix cache** — automatic, prefix-unit based, best-effort, billed via `prompt_cache_hit_tokens` / `prompt_cache_miss_tokens`; cleared "usually within a few hours to a few days" once unused ([DeepSeek, Context Caching](https://api-docs.deepseek.com/guides/kv_cache)). DSH's DeepSeek route sends **no** cache markers — `grep cache_control packages/llm/llm-deepseek/src/` → 0 hits.
2. **DSH harness-level reuse** — append-only derived history, canonical request headers, in-history prompt appends, compaction and pruning as *logged* replacements, resume, fork/spawn children. This is §3.
3. **Process-local caches that never reach the model** — model-listing cache (`packages/llm/llm-deepseek/src/listing-cache.ts`), session-projection checkpoints (`docs/capability-seams.md:701`), the `deriveMessages()` memo (§3), tool-pairing cut balances (`packages/compaction/compaction/README.md:105`), the file-id index. Useful context, zero prompt effect.

## 2. Strategy table

| Strategy | Mechanism observed | Evidence | Grade |
|---|---|---|---|
| Prefix / prompt caching (context re-use) | Append-only derived history + canonical header; prompt is a surface message, not a request field; `in-history` routes append prompt changes instead of rewriting node 0; no explicit markers (automatic on the DeepSeek route) | `agent.ts:721`, `session/index.ts:999`, `runtime-context.ts:93`, `request-cache.e2e.ts` | high |
| Deterministic ordering | Prompt sections sorted by `(order, name)`; tool schemas sorted by name or an explicit `toolOrder`; header compared structurally before it is re-logged | `system-prompt/src/index.ts:236,657,678` | high |
| Tool & scratchpad output caching | None. Tool-result pruning is one lossy pass at compaction pressure, logged as a surface replace, not a cache | `compaction-tool-result-pruner/README.md:52`; `2026-07-05-reconstructable-requests.md:54` | high (absence) |
| Semantic caching | None found | §3 | high (absence) |
| Speculative / subagent execution caching | `fork` children inherit the parent's completed-turn seed and may reuse the byte-identical prefix; `spawn` children start a fresh, independent prefix; no warm-up barrier | `subagent-fork-in-process/README.md:120`, `subagent-spawn-in-process/README.md:117`, `subagent/README.md:53` | high |
| Tool / function cache | None. Every accepted call executes; results are logged, never memoized | §3 | high (absence) |
| Exact request cache | None. Every step sends a full request; no `previous_response_id`, no response reuse | §3 | high (absence) |

## 3. Findings by strategy

### Prefix / prompt caching (context re-use)

**Request assembly.** One request is exactly: `header.config` (provider, model, effort, adapter-resolved defaults, sampling), `deriveMessages()`, `header.tools`, `toolHistory`, `sessionId` — and "it carries no `system` field" (`packages/core/agent-loop/src/agent.ts:728-735`). The system prompt is *derived history*: a `system/message` surface event at node 0 (`.agents/notes/implemented/architecture/2026-09-02-system-prompt-as-surface-node.md`).

**Static head vs volatile tail.** The head is node 0 (rendered prompt) plus `header.config` and `header.tools`. Everything else — user messages, tool results, runtime context, injected context, steering — is appended after it. The prompt is deliberately *not* in the header, so a prompt change is a surface replacement rather than a header change (`2026-07-05-reconstructable-requests.md`, "The mechanism"). The volatile channel is called out as cache-safe by design: `PromptContext` is documented as "the cache-safe counterpart to `PromptSection`", materialized as a durable user-role snapshot "after retained model history only when it changed or compaction removed it" (`docs/subsystems/system-prompt.md:76-80`).

**Append-only enforcement.** `Session.deriveMessages()` folds the surface incrementally, keeps a per-node memo keyed by `contentGeneration`, and rebuilds only on a replacement (`packages/core/session/src/index.ts:999-1020`). Each request gets a fresh array over shared, deep-frozen messages, so "mutating logged history through a projection is unrepresentable (it throws)". The loop freezes the header and message array per request (`agent.ts:719-735`).

**Header logging is the cache-visible event.** A `request/header` snapshot is written only when the envelope is genuinely new: reason `initial`, `resume` (first request of a resumed instance), `change` (structurally different from the folded baseline), or `series` (same bytes, new message series — a surface replacement, a declared series start, or a tool-set change on a route without tool-update support) (`agent.ts:669-683`). Unchanged steps, retries and ordinary later turns *inherit* the previous snapshot. This is the cheapest possible assertion of prefix identity: if no header event appears, the head bytes are the head bytes.

**Prompt updates do not necessarily rewrite the head.** `SystemPromptProjection.project()` chooses between rewriting node 0 and appending (`packages/core/agent-loop/src/runtime-context.ts:93-108`): a route declaring `systemPromptUpdate: 'in-history'` **appends** changed non-empty prompt text after cached history inside a continuing series; an incapable route, a cleared prompt, or a series start whose effective text differs **replaces node 0**. It also refuses to move identical text: "a series start whose latest surviving node already holds the rendering emits nothing: moving the same text to node 0 would change the request prefix without changing the prompt" (`runtime-context.ts:88-92`). Route capability is per model (`packages/llm/llm/src/types.ts:436-438`); the static DeepSeek catalog declares `systemPromptUpdate: 'in-history'` + `toolUpdate: 'addition-only'` for `deepseek-flash` only, and declares nothing for `deepseek-v4-pro` (`packages/llm/llm-deepseek/src/models.ts:14-24`). A followed `/models` listing can supply per-model facts (`listed-catalog.ts:57-68`), so live behaviour depends on the listing, not just the shipped catalog.

**The one real cache test.** `packages/core/agent-loop/tests/request-cache.e2e.ts` runs a two-step tool turn plus a follow-up turn against the live API and asserts, for **every** request after the first, `usage.cacheReadTokens > 0` (lines 96-104). Its header comment states the design intent — "log-derived requests translate into real provider cache hits … the per-step usage recorded on `assistant/message` events is the production observable for cache behavior" — and its fixture is padded so the shared prefix "comfortably spans the provider's cache-block granularity (64 tokens)" (line 22). That 64-token figure is DSH's own assumption; DeepSeek's guide documents "persistence at fixed token intervals" without publishing the interval ([DeepSeek, Context Caching](https://api-docs.deepseek.com/guides/kv_cache)). The test is key-gated (`describe.skipIf(!process.env.DEEPSEEK_API_KEY)`), so it proves a hit only when a key is present.

**Summarization replays the prefix instead of rebuilding it.** `dsh-compaction-basic` makes its summarize call with the conversation's own system prompt, tool schemas and leading messages replayed verbatim, and delivers the directive as the *final user message*: "Keeping the conversation's own system prompt, tools, and message prefix in front of it makes the auxiliary call a genuine prefix of the last routed request, so the provider's KV cache is reused instead of invalidated" (`packages/compaction/compaction-basic/src/summarizer.ts:24-30`, input contract at `:75-85`). This is the exact failure the earlier harness survey found elsewhere (OpenHands' condenser, Gemini CLI's compressor) — here it is inverted by design.

### Deterministic ordering

Ordering is enforced, not incidental. Prompt sections sort by ascending `order` then code-unit `name` (`packages/core/system-prompt/src/index.ts:657`); the final request order sorts by `order` (`:678`). Tool schemas sort by name, or follow an explicit `toolOrder` list with the remainder name-sorted (`:236-242`). Message order is the surface order, and a replacement node may carry a *higher* durable seq than a node it precedes, so consumers must read surface order, not seq order (`docs/subsystems/token-meter.md`, `TokenSurfaceNode`).

Header equality is a structural comparison of the canonicalized envelope (`canonicalHeader`, `agent.ts:658-663`), which is what makes "no header event" a sufficient proof that the head did not change. Determinism here is by construction — same fold, same bytes — not by hashing.

### Tool & scratchpad output caching

No result cache exists. The optional `dsh-compaction-tool-result-pruner` replaces any tool result whose text exceeds `thresholdChars` with head + `"middle pruned"` marker + tail (defaults 8192 / 4096 / 1024 code points) — one deterministic pass, only after a compaction trigger qualifies, making no model call (`compaction-tool-result-pruner/README.md:44-54`). The original stays in the log; the replacement is a logged surface replace under the same `callId`. Cache effect: the request now differs from the first replaced result onward, and the reconstructability note accepts that explicitly — "compaction-family, replay-correct, cache-bust batched by the same pressure logic" (`2026-07-05-reconstructable-requests.md:54`). Batching is the point: one bust at pressure time rather than one per step.

`deferLoading: true` exists on tool definitions and is preserved through registry projection and prompt assembly (`packages/core/tools/README.md:201`), but I did not find a DSH-level tool-search mechanism that defers MCP schemas behind a discovery tool.

### Semantic caching

None found in the checkout. `grep -r semantic_cache` over `packages/` and `apps/` → 0 files. The only "reuse" of prior model output is durable replay of logged assistant messages, which is history, not a cache.

### Speculative / subagent execution caching

Two local backends with opposite cache stories:

- **fork** seeds the child with the parent's balanced completed-turn prefix and keeps the request head identical: no child-only system-prompt section, no child-only tool schema; the parent id and return guidance ride in the initial user task *after* the inherited history. "The child may reuse the inherited byte-identical prefix under the same provider and model" (`packages/subagent/subagent-fork-in-process/README.md:120`). Shipped fork tools deliberately expose no child model selection "so the copied history remains eligible for KV Cache reuse" (`:144`). The enabling decision is `.agents/notes/implemented/architecture/2026-08-10-fork-children-stay-one-shot.md`.
- **spawn** starts an empty conversation: "The child's request cache is independent of the parent's" (`packages/subagent/subagent-spawn-in-process/README.md:117`). It still inherits provider/model/effort/output cap by default; `request.agentOptions` can override (same file, "Child-agent request").

No warm-up barrier or staged fan-out exists. Children are admitted concurrently up to `maxActiveSubagents` (default `8`), reservations taken "before yielding" (`packages/subagent/subagent/README.md:53`). Parallel forks of one parent therefore all request the same inherited prefix at once; each pays its own cache read, and nothing sequences them.

DSH's own honesty note is worth keeping: "The harness tests equality of its assembled request-head inputs, not a provider's cache behavior" (`2026-08-10-fork-children-stay-one-shot.md:43`).

### Tool / function cache

None. `ctx.tools.execute()` dispatches every accepted batch; the executor owns ordering, concurrency, cancellation and logging, not memoization (`packages/core/tools/src/batch.ts`, `executor.ts`). The tool-pairing `cut cache` (`compaction/README.md:105`) caches boundary *balance*, not results. Sub-agents are not a result cache either: an activation is executed once, and external backends (ACP, DSH SDK, Codex, Claude Code) cannot be continued at all.

### Exact request cache

None. Each step sends a complete request. `previous_response_id` and `prompt_cache_key` appear in no DSH adapter source — the only hits in the checkout are in a Codex test fixture (`packages/subagent/subagent-codex/tests/responses-fixture.ts:71-72`). The reconstruction invariant is the opposite of an incremental protocol: "anyone holding the log … reconstructs every loop request byte-for-byte" (`2026-07-05-reconstructable-requests.md:19-21`). The one route-identifying value DSH passes is `sessionId` (a top-level body field plus `x-deepseek-harness-session-id`), which the extension registry documents as "model-hidden provider metadata" that "do not alter the serialized model-input prefix" (`packages/llm/deepseek-llm-api-extensions/README.md`, KV Cache effect). The same value is handed to `llm-pi-ai` as a stream option (`packages/llm/llm-pi-ai/src/adapter.ts:420`); what that library does with it is outside this checkout.

## 4. Measured / quantified evidence

- **A measured DSH session in one deployment:** 1,353,561 fresh / 177,797,000 cached / 262,709 output tokens for the turns 154–157 DSH orchestration run. Arithmetic over that row: cache share = 177,797,000 / 179,150,561 ≈ **99.24 %**. It is a pooled single session, not a controlled measurement, and the same record warns that DSH "needs complete per-step usage before its aggregate can be read as a complete total".
- **In-tree cache assertion:** ≥ 3 requests, every one after the first with `cacheReadTokens > 0` (`request-cache.e2e.ts:96-104`). No hit-rate figure is asserted.
- **No cache benchmark ships.** `grep -n cache BENCHMARK.md` → nothing; the benchmark tree measures session open, memory and throughput, not cache.
- **Tuning constants that move the break points:** compaction trigger `floor(min(W × 0.8, W − 65536))`, retention `0.16 × (W − O)` (`compaction-basic/README.md:64-70`); pruner `8192/4096/1024` code points; `maxActiveSubagents` 8; official context window 1,048,576 / output cap 256,000 (`packages/llm/llm-deepseek/src/defaults.ts:6-8`).

## 5. Failure modes and invalidators

Each line names the mechanism, not a guess. `warmpath::cache::CacheBreakReason` vocabulary in brackets.

- **Prompt rewritten at node 0** `[prefix_rewritten]` — a changed rendered prompt on a route without `in-history` (`deepseek-v4-pro` in the static catalog), on a new series with different text, or after a cleared prompt. Any edit to a `PromptSection`'s text or the persona moves the rendered prompt and therefore takes this path; plan mode does it through `mode:policy` (`docs/subsystems/plan.md:24-35`). Full miss from node 0, i.e. the whole conversation.
- **Plan mode re-entry** `[policy_change]` — plan guidance is a system-prompt section, and `exit_plan_mode` stays admitted for the rest of the Session (`plan.md:33`). On an `in-history` route the toggle appends (prefix preserved, and every retained prompt version is then paid on each step until compaction shadows it — `agent-loop/README.md:166`); on a leading-only route it replaces node 0.
- **Tool surface change** `[tool_surface_changed]` — MCP connect/disconnect, dynamic registration. The header records the active set, so a real change is a `change` snapshot; additions/removals also emit one `developer/message` (`agent.ts:686-697`). On `addition-only` routes additions ride in history and the head survives, but a *removal* cannot be represented in history (`packages/llm/llm/src/content.ts:471`) and the changed tools array breaks the head. Tool-schema *text* edits break reuse from the first altered token even when the name set is unchanged (`agent-loop/README.md:164`).
- **Compaction** `[compaction]` — the post-compaction request is a new series whose head is one checkpoint `user/message` followed by the retained tail: "A successful backend replacement invalidates reuse from the first shadowed history token" (`packages/compaction/compaction/README.md:148`). The summarize call itself is the exception — it replays the prefix (§3).
- **Tool-result pruning** `[prefix_rewritten]` — same class, one batched pass at pressure.
- **Image offload** `[prefix_rewritten]` — an over-budget image occurrence is replaced by logged placeholder text (`image/offload`), rewriting an older node (`packages/llm/llm/README.md:127`).
- **Route change** `[route_changed]` — provider/model/effort is captured before assembly and a mid-step change rejects the step; the recorded choice applies to the next step (`agent-loop/README.md`, "Request headers and adapter defaults"). A model switch also triggers destination-capacity compaction before commit (`compaction-basic/README.md`, "Switching models").
- **Cold start** `[cold_start]` — every fresh `dsh --profile headless` with a new session id, and every `spawn` child.
- **Resume drift** `[prefix_rewritten]` — `--session-id` re-adopts persisted history; a differing assembled head logs a `resume` snapshot and re-prices from the first differing token. Resume with an identical head is byte-identical and therefore *eligible*, but eligibility is not a hit: the provider may have evicted (best-effort, hours-to-days).
- **Provider miss** `[provider_miss]` — DeepSeek persists a shared prefix as its own unit only after it observes the commonality, so a repeated *shared* head across otherwise-different sessions hits from the *third* request onward, not the second ([DeepSeek, Context Caching](https://api-docs.deepseek.com/guides/kv_cache), Example 2).

**False-zero hazards (all guarded).** The DeepSeek adapter maps `cache_read_input_tokens` → `cacheReadTokens` and `cache_creation_input_tokens` → `cacheWriteTokens`, skipping absent fields rather than defaulting them to 0 (`packages/llm/llm-deepseek/src/translate.ts:34-41`), then sets `totalTokens = input + output + cacheRead + cacheWrite` (`:159`). `pi-ai` is the opposite shape — it reports zeros, not absence, so the adapter only surfaces a bucket when `> 0` (`packages/llm/llm-pi-ai/src/stream.ts:20-32`). `TurnTokenUsage` keeps an optional bucket "present only when every attempt reported it", and `normalizeUsage` rejects the whole attempt when `totalTokens − outputTokens ≠ input + cacheRead + cacheWrite` with both buckets present (`packages/llm/token-meter/src/turn-usage.ts:80-124`). The headless `--json` projection publishes a step's usage only when every attempt reported a sample — "a partial sum would read as an exact total" (`packages/bundle/headless/src/json-stream.ts:191-203,279-288`). The chat UI refuses to round a partial hit to 100 % (`packages/client/ui-chat/src/client/chat/token-format.ts:58-79`).

## 6. Consequences for `warmpath`

**Mirror:**

1. **A prefix digest must cover the header, not just messages.** DSH's head is (system node 0 + `header.config` + `header.tools`). `warmpath::cache::CacheIdentity` already carries prefix + harness + provider + model + effort + session (`src/cache.rs:319-371`); a digest computed over derived messages alone would call two different DSH request heads equal when only the tool set changed. Feed the tool-schema digest into `PrefixDigest`.
2. **Attribute breaks with DSH's own vocabulary.** DSH produces exactly the causes `CacheBreakReason` already names — `prefix_rewritten` (node-0 replacement, pruning, image offload), `tool_surface_changed`, `compaction`, `route_changed`, `cold_start`, `policy_change` (plan mode) (`src/cache.rs:281-303`). The `request/header` event with reason `change` is the cheap, durable signal for the first two; a `system/message` replace or a `compaction/summary` is the signal for the rest.
3. **Keep the completeness rule.** DSH will omit a step's usage rather than publish a partial sum; a dispatch-plane contract should record exactly that triple — "summing every `status`/`step_end` event (`inputTokens` uncached, `outputTokens`, `cacheReadTokens`)". Treat an absent `usage` as not reported, never zero.
4. **Prefer the append channel.** Anything the dispatch plane wants the model to see (added context, a caller preamble, per-run policy notes) belongs in a user-role message appended after history, exactly as DSH's own runtime context does (`agent.ts:461-469`). Injecting it as a prompt section is what forces a node-0 rewrite on leading-only routes.

**Avoid:**

5. **Do not assume a shared request head warms the next dispatch.** Two dispatches over the same head in different worktrees share only the system+tools head, and DeepSeek persists a shared prefix as a unit only after observing it. The first two pay full price; the third may hit the shared unit. Treat that as a hypothesis to measure, not a planning assumption.
6. **Do not budget `cached > 0` as "prefix stable".** A non-zero `cacheReadTokens` is consistent with a much shorter persisted unit (DSH's own e2e asserts only `> 0`). The eligible-expected-prefix ratio, not the raw share, is the metric.
7. **Do not read `cache_creation_input_tokens` as a DeepSeek write charge.** DeepSeek's contract charges no separate write (`src/cache.rs:236-246`: `write_multiplier` 1:1, TTL unknown); a `cacheWriteTokens > 0` from the Messages route is a field the provider chose to send, not evidence of a write premium.
8. **Do not model DSH's TTL.** A 300 s warm window for Dsh is a conservative local default, explicitly "not measured" and not provider-declared. DSH itself carries no TTL knowledge anywhere in the checkout.

## 7. Confidence and gaps

- **High (source-verified):** append-only derivation and its memo (`session/index.ts:999`); the header snapshot rules and reasons (`agent.ts:669-683`); request = config + messages + tools + toolHistory + sessionId with no `system` field (`agent.ts:728-735`); node-0 replace vs in-history append (`runtime-context.ts:93-108`); compaction and pruning as logged replacements with the stated invalidation point (`compaction/README.md:148`); the summarize call replaying the prefix (`summarizer.ts:24-30`); fork/spawn prefix claims; usage parsing and the false-zero guards; absence of markers, `prompt_cache_key`, `previous_response_id`, semantic cache, tool cache.
- **Medium:** that `deepseek-flash`/`deepseek-v4-pro` capabilities hold at dispatch time (shipped catalog vs followed `/models` listing drift); the 64-token cache-block assumption; that the Messages route's `input_tokens` is exclusive of cache reads — `TokenUsage` says adapters "subtract them out" for providers that fold cache hits into a total, but no adapter in the checkout performs that subtraction, so the DeepSeek route simply trusts the protocol convention (nothing verifies it, and `normalizeUsage` would drop an inconsistent measurement rather than misreport it); that a replayed summarize call actually hits.
- **Unknown:** provider TTL and eviction; any real hit rate for a headless dispatch or a fork child; whether DeepSeek's common-prefix persistence behaves as the guide's Example 2 in practice; what `llm-pi-ai` does with `sessionId` downstream.
- **Not observed in the checkout** (named greps): `prompt_cache_key`, `previous_response_id`, `cache_control` under `packages/llm/llm-deepseek/`, `semantic_cache`, `response_cache`, an automatic tool-search that defers MCP definitions, any cache-hit benchmark.

## 8. Sources

DSH checkout (commit `22ddee3`, 2026-10-23):

- `.agents/notes/implemented/architecture/2026-07-05-reconstructable-requests.md:19` — prefix stability as corollary #1; `:31` the with-key enforcement; `:52` what still costs full price; `:54` pruning is a batched cache bust.
- `.agents/notes/implemented/architecture/2026-08-10-fork-children-stay-one-shot.md:21` — the byte-identical head requirement; `:43` the harness tests assembled inputs, not provider cache.
- `.agents/notes/implemented/architecture/2026-09-02-system-prompt-as-surface-node.md` — prompt as derived history at node 0.
- `packages/core/agent-loop/src/agent.ts:435-452`, `:640-700`, `:721-735` — render, in-history decision, header snapshots, tool add/remove messages, frozen request.
- `packages/core/agent-loop/src/runtime-context.ts:88-115` — `SystemPromptProjection.project`.
- `packages/core/session/src/index.ts:999-1020` — cached incremental `deriveMessages()`.
- `packages/core/system-prompt/src/index.ts:236-242,657,678` — tool and section ordering.
- `packages/core/agent-loop/README.md:164-172` — the harness's own KV-cache statement per request surface.
- `packages/core/agent-loop/tests/request-cache.e2e.ts:20-24,96-104` — the live cache-hit assertion and the 64-token assumption.
- `packages/llm/llm/src/types.ts:171-187` — disjoint token accounting; adapters subtract inclusive totals.
- `packages/llm/llm/src/content.ts:420-480` — `projectToolUpdates` per route capability.
- `packages/llm/llm-deepseek/src/translate.ts:34-41,159` — cache field mapping and total composition.
- `packages/llm/llm-deepseek/src/models.ts:14-24`; `src/config.ts:81-82` — per-model `systemPromptUpdate` / `toolUpdate` declarations.
- `packages/llm/llm-pi-ai/src/stream.ts:20-32` — zeros-not-absence mapping.
- `packages/llm/token-meter/src/turn-usage.ts:80-124` — consistency rejection and per-bucket completeness.
- `packages/bundle/headless/src/json-stream.ts:191-221,279-288` — step usage omitted rather than partially summed.
- `packages/compaction/compaction-basic/src/summarizer.ts:24-30,75-85` — cache-preserving summarization input.
- `packages/compaction/compaction/README.md:148` — KV Cache effect of a surface replacement.
- `packages/compaction/compaction-tool-result-pruner/README.md:44-54` — trim budgets and trigger.
- `packages/subagent/subagent-fork-in-process/README.md:120,144`; `packages/subagent/subagent-spawn-in-process/README.md:117`; `packages/subagent/subagent/README.md:53` — fork reuse, spawn independence, activation capacity.
- `packages/llm/deepseek-llm-api-extensions/README.md` — registry fields are model-hidden and do not alter the prefix.
- `docs/subsystems/system-prompt.md:76-80`; `docs/subsystems/plan.md:24-35`; `docs/subsystems/compaction.md`; `docs/cli-help.md:100-130` — prompt contexts, plan-mode surface, compaction events, headless flags.
- `docs/architecture.md:23,27,43` — profile composition.

Local crate:

- `src/cache.rs`:31-140,203-303,319-371 — `CacheMode`, `CacheContract`, DeepSeek semantics, `CacheBreakReason`, `CacheIdentity`.
- The deployment's recorded cost model — the measured DSH session token shape and the cold-start/completeness caveats.
- The sibling prompt-caching documents — DeepSeek disk prefix cache ([01](../../prompt-caching/01-provider-mechanics.md)); harness failure patterns compared in §3 ([02](../../prompt-caching/02-harness-practices.md)); claim grading ([04](../../prompt-caching/04-sources-and-confidence.md)).

Web:

- [Context Caching](https://api-docs.deepseek.com/guides/kv_cache) — DeepSeek, undated (retrieved 2026-10-28) — supports: automatic disk prefix cache, prefix-unit matching, persistence at request boundaries / common prefixes / fixed token intervals, `prompt_cache_hit_tokens` & `prompt_cache_miss_tokens`, best-effort with no guaranteed hit rate, cleared within hours-to-days.
