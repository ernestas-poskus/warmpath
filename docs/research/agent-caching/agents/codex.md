# Codex CLI — caching behaviour

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research, 2026-10-28
- **Scope:** OpenAI Codex CLI, `codex-cli 0.164.0` (installed, `codex --version`); source read at tag `rust-v0.164.0`. `rust-v0.165.0` shipped 2026-10-27 and `main` is ahead, so line references are tag-pinned, not "current". Model request path in `codex-rs/core` only; TUI, app-server and cloud surfaces are out of scope.
- **Method:** local — [01](../../prompt-caching/01-provider-mechanics.md), [02](../../prompt-caching/02-harness-practices.md), [04](../../prompt-caching/04-sources-and-confidence.md), the companion harness research shipped with the `workcell` crate, [warmpath DESIGN](../../../../docs/DESIGN.md), [cache.rs](../../../../src/cache.rs), and one deployment's cost model, ledger, and dispatcher docs; 19 source/test files at `rust-v0.164.0` via the GitHub API; 1 web search; 3 page fetches (1 succeeded — [#37305](https://github.com/openai/codex/issues/37305); the OpenAI prompt-caching guide 404s at both known paths). Access date 2026-10-28.
- **Companion:** [index](../README.md) · [matrix](../matrix.md)

## 1. What it is and where caching lives

Codex is a Rust agent with **one** wire protocol: `WireApi` has a single variant, `Responses`; `wire_api = "chat"` fails to deserialize (`` `wire_api = "chat"` is no longer supported ``, [provider-info:102](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/model-provider-info/src/lib.rs#L102)). Every route — OpenAI, Bedrock, Ollama, a custom base URL — must present a Responses-shaped body.

Four layers, kept apart:

- **(a) Provider.** OpenAI's automatic prefix cache addressed by `prompt_cache_key`. The request struct has **no** `prompt_cache_breakpoint`, **no** `prompt_cache_retention`, no marker — the field list is exhaustive ([common.rs:279-303](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/codex-api/src/common.rs#L279-L303)). `store` is hardcoded `false` ([client.rs:1119](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/src/client.rs#L1119)).
- **(b) Harness reuse.** Append-only item history; a WebSocket continuation sending `previous_response_id` plus only the new items; deferred/incremental tool declarations; remote and local compaction.
- **(c) Client caches.** WebSocket connection reuse inside a turn, a memoized BM25 index for `tool_search`, in-process caches. None is a model-response cache.
- **(d) Folklore.** "Codex caches tool results" / "Codex has a response cache" — none documented and none in the request path (§3).

## 2. Strategy table

| Strategy | Mechanism observed | Evidence | Grade |
| --- | --- | --- | --- |
| Prefix / prompt caching (context re-use) | Automatic OpenAI prefix cache; `prompt_cache_key` on every request (`SessionDerived` = session id by default); `cached_input_tokens` in usage; no explicit breakpoint | request struct; `PromptCacheKey`; usage struct | high |
| Deterministic ordering | Fixed front: base-instructions item → permissions → bundled contextual user message → user turn; tools as top-level array (or prepended `AdditionalTools` on Responses Lite); UUIDv5 item ids from `thread_id` + payload | ordering test asserting exact indexes and byte stability | high |
| Tool & scratchpad output caching | None local. Truncation at record time; outputs are history items never edited in place; `retained_tool_outputs` keeps marked outputs whole or drops them | `record_items(_, truncation_policy)`; `compact_retained_tool_outputs.rs` | medium |
| Semantic caching | None documented; no embedding or response-dedupe path | absence in request path | high (absence) |
| Speculative / subagent execution caching | Child sessions share the parent's `prompt_cache_key` (`"{source}:{parent_thread_id}"`); full-history forks preserve the prefix; WS warmup `generate: false` | `session_derived_cache_key`; fork test; `ResponseCreateWsRequest.generate` | high |
| Tool / function cache | No provider-side function-result cache; `tool_search` memoizes its BM25 index in-process; `WebSearchMode::Cached` is a server-side index | `ToolSearchHandlerCache`; `WebSearchMode` | medium |
| Exact request cache | None. One turn-scoped `last_request` kept for diffing on the WS transport | `ModelClientSession` docs | high (absence) |

## 3. Findings by strategy

### Prefix / prompt caching (context re-use)

- **`prompt_cache_key` is sent by default.** `PromptCacheKey` is `SessionDerived` (default) / `Custom(String)` / `Omit` ([thread_manager:273](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/src/thread_manager.rs#L273)); `SessionDerived` resolves to `metadata.session_id` or a host override ([client.rs:666](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/src/client.rs#L666)). A test asserts it is identical across turns and survives per-turn model/effort overrides ([prompt_cache_key.rs:60-95](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/tests/suite/prompt_cache_key.rs#L60-L95)).
- **`session-id` / `thread-id` headers also go out**, and a source comment says ChatGPT derives cache affinity from the session-id header — which is why a non-root agent keeps its own header while inheriting the parent's key ([client.rs:680-692](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/src/client.rs#L680-L692)).
- **`previous_response_id` exists only on the WebSocket transport.** `ResponseCreateWsRequest` carries it (plus `generate`); the HTTP SSE struct does not ([common.rs:332-360](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/codex-api/src/common.rs#L332-L360)). A turn is incremental only when all of: the provider advertises `supports_websockets` (true for built-in OpenAI, explicitly rejected for Bedrock — [provider-info:306-310](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/model-provider-info/src/lib.rs#L306-L310)), the connection is alive, and the request strictly extends the previous one. Otherwise `mode="full"` re-sends the whole `input`.
- **The non-incremental reason is enumerated in source:** `no_previous_request`, `no_previous_response`, `no_previous_response_id`, `model_changed`, `tools_changed`, `tool_choice_changed`, `parallel_tool_calls_changed`, `reasoning_changed`, `store_changed`, `stream_changed`, `include_changed`, `service_tier_changed`, `prompt_cache_key_changed`, `text_changed`, `input_shortened`, `input_mismatch`, plus `restored_history` when a resumed thread has no live WS state ([client.rs:347-465](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/src/client.rs#L347-L465)).
- **It is recorded, but not where a run log can see it:** a `trace!` line and the OTEL counter `codex.websocket.continuation{mode,reason,previous_item_type,current_item_type,phase,after_prewarm}` ([names.rs:20](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/otel/src/metrics/names.rs#L20)). `codex exec --json` is a closed event enum carrying only token usage, so "why did this turn miss" needs metrics, not the log.
- **No explicit boundary.** Codex cannot emit `prompt_cache_breakpoint`; [#35300](https://github.com/openai/codex/issues/35300) is open. A replay that added one moved hits 0% → 98.6% ([02](../../prompt-caching/02-harness-practices.md)) — that measures the gap, not a Codex feature.
- **`store: false` with `previous_response_id`.** Both appear on the WS body; whether the provider retains a response referenced that way under `store:false` is not documented in-repo — **not verified**.

### Deterministic ordering

The front is fixed and asserted by test. A default session's first request `input` is exactly four items ([prompt_caching.rs:412-460](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/tests/suite/prompt_caching.rs#L412-L460)): `[0]` base instructions **as a message item** (the request struct has no top-level `instructions` field), `[1]` permissions/approval context, `[2]` one bundled contextual user message holding user/`AGENTS.md` instructions **and** environment context (including `cwd`), `[3]` the user turn.

Tools are a top-level array on classic Responses; on Responses Lite they become a prepended `AdditionalTools` item whose id is `uuidv5(uuidv5(NAMESPACE_OID, thread_id), tools_json)` ([client.rs:1010-1029](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/src/client.rs#L1010-L1029)). The base-instructions id is `uuidv5(prefix_namespace, text)` — deterministic **within a thread**, different across threads: ordering is stable-first (good), but two byte-identical fresh sessions in different threads do not present identical bytes ([#21796](https://github.com/openai/codex/issues/21796), open). Pinned by `prompt_tools_are_consistent_across_requests`, `prefixes_context_and_instructions_once_and_consistently_across_requests`, `overrides_turn_context_but_keeps_cached_prefix_and_key_constant` and `per_turn_overrides_keep_cached_prefix_and_key_constant` — the last two also assert the key does not move.

### Tool & scratchpad output caching

- No result cache. Tool output is a `FunctionCallOutput` history item, never rewritten in place; only *recording* truncates (`record_items(…, truncation_policy)`). `retained_tool_outputs` keeps explicitly marked outputs "whole or dropped" across compaction.
- **A newly loaded tool does not invalidate the prefix.** `tool_search` is no longer a flag — registry entry `Stage::Removed`, "retained as a no-op now that tool_search is always enabled", and "MCP tools are always deferred when tool_search is available" ([features:1630-1640](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/features/src/lib.rs#L1630-L1640)). Discovery results append a `ToolSearchOutput` item; the tools array and earlier bytes are untouched.
- **A catalog change appends.** `incremental_tools` is `Stage::Stable`, default on, and `incremental_tools_append_changed_catalog_without_rewriting_history` runs both wire shapes ([scenarios_incremental_tools.rs:143](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/tests/suite/scenarios_incremental_tools.rs#L143)). The one remaining break is the **classic `tools` array**: any difference yields `tools_changed`, forcing a full request and a new provider prefix.

### Semantic caching

None documented and none found: no embedding index over history, no response dedupe, no near-match shortcut. The only BM25 index is `tool_search`'s catalog search (`ToolSearchHandlerCache`), which caches a **tool index**, not a model answer.

### Speculative / subagent execution caching

- **Children share the cache key, not necessarily the prefix.** `session_derived_cache_key` returns `"{source}:{parent_thread_id}"` for `SessionSource::Internal(..)`, while `responses_session_id` keeps the child's own session id for non-root agents ([client.rs:666-692](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/src/client.rs#L666-L692)) — a spawned worker is routed to the parent's affinity but presents a different prompt.
- **A full-history fork does share it:** "Full-history forks preserve the cached prompt prefix" ([spawn.rs:120](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/src/agent/control/spawn.rs#L120)); the prediction-fork test asserts the child's `input[..parent.len()]` equals the parent's input exactly ([scenarios_incremental_tools.rs:41](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/tests/suite/scenarios_incremental_tools.rs#L41)).
- **Warmup is connection setup, not a cache entry:** the WS payload supports `generate: false`, and a warmup response is excluded from rollout traces. No speculative execution (no replayed results, no branch memoization).

### Tool / function cache

No provider-side function-result cache. Locally the `tool_search` handler memoizes its built index and embedder per registry, and `WebSearchMode` is `Disabled | Cached (default) | Indexed | Live` — `Cached` selects a **server-side** web index, not a local response store ([config_types:376](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/protocol/src/config_types.rs#L376)).

### Exact request cache

None. A `ModelClientSession` keeps `last_request` for one turn so the next request on the same WebSocket can be diffed into a delta; it is dropped with the turn. No "same request → stored response".

## 4. Measured / quantified evidence

| Measurement | Figure | Source | Grade |
| --- | --- | --- | --- |
| Seven-run Codex batch (2026-10-22) | 685,887 fresh / 20,281,216 cached / 59,228 output → **96.73% cached** | recorded runs | high (recorded runs) |
| Three-work-item run (2026-10-21) | 51,301 / 1,045,504 / 6,716 → 95.3% cached, $0.3668 list | recorded runs | high |
| Warm-resume calibration | codex 6/6 hits in `<2m`/`2–5m`/`5–10m`, 0/2 in the `10–30m` control | warm-resume calibration report | medium |
| Explicit breakpoint replay | 0% → 98.6% hits; writes ~9,060 → 123–128 | [02](../../prompt-caching/02-harness-practices.md) | medium (controlled replay) |
| Local compaction bug | ~+70% total uncached input; 65–88% cached after including tools | [#37305](https://github.com/openai/codex/issues/37305) (0.146.1) | medium |

The 96.73% figure is the strongest signal: with a stable request head and a long append-only thread, OpenAI's automatic cache already serves almost the whole prefix — Codex adds no marker and does not need one for the steady state. The 3.27% tail is the per-turn delta plus the cold start.

## 5. Failure modes and invalidators

- **Model / effort / verbosity / reasoning-summary change** — `model_changed`, `reasoning_changed`, `text_changed`.
- **Tool set or order change** — `tools_changed`; `parallel_tool_calls` and `service_tier` are compared as their own properties.
- **History rewrite** — `input_shortened` / `input_mismatch`; trimming, reordering or re-projecting history starts a new prefix.
- **Resume from a rollout** — `restored_history` forces a full request on the first turn even when the provider still holds the prefix, so a resumed thread is not automatically incremental.
- **Local compaction** — the one *shipped* path that deliberately sends a different front: it builds `Prompt { ..Default::default() }`, so `tools: []` and `parallel_tool_calls: false` ([compact.rs:318](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/src/compact.rs#L318)), while every regular turn sends the router's specs and usually `true`. Base instructions *are* preserved (read back from `BaseInstructionsFragment`), but the miss is structural. Still present at 0.164.0; [#37305](https://github.com/openai/codex/issues/37305) open since 2026-08-05. Remote v2 compaction is the contrast: it appends a `CompactionTrigger` item and sends `tool_router.model_visible_specs()` with `parallel_tool_calls: true` ([compact_remote_v2_attempt.rs:167-182](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/src/compact_remote_v2_attempt.rs#L167-L182)).
- **Per-thread item ids** — UUIDv5 from `thread_id`, so an identical fresh session in a new thread is a cold prefix.
- **`cwd` inside the cached contextual message** — a different worktree is a different prefix, exactly the per-work-item worktree shape a dispatch plane can create.
- **Route** — Bedrock has no WebSocket transport; a custom `model_providers` entry may ignore `prompt_cache_key` and report no `cached_input_tokens`. `is_openai` gates several request fields, so a non-OpenAI route is a different contract, not a variant.
- **Accounting** — `turn.completed.usage` is the thread running total, so per-turn hit rate needs baseline subtraction (adopted: a thread-total usage correction in the ledger). Cached tokens still count toward TPM, so a 96% hit rate does not buy quota.

## 6. Consequences for `warmpath`

1. **`cache::semantics("codex", model)` models the provider, not the harness** — Automatic, 1,800 s TTL, 43,200 s long TTL, 1,024-token minimum, 0.1× read, 1.25×/2.5× writes. That matches source: no marker is sent, so the only levers are key stability and prefix stability.
2. **`CacheIdentity` should carry a tool-set digest, not just a prefix digest.** `tools`, `parallel_tool_calls`, `service_tier`, `text` and `reasoning` are named members of the request-property comparison, so two runs with the same request head but a different MCP/plugin surface are not the same cache identity.
3. **Resume is still the cheapest retry, but the first post-resume turn is `restored_history`.** Treat it as ineligible for expected-prefix, not as a miss — the same "excluded, not zero" rule already applied to cold starts.
4. **Exclude compaction turns from EEPR on the local path.** Remote v2 mirrors a normal turn; local sends `tools: []` and cannot hit by construction. Until #37305 closes, a compaction miss is expected behaviour, not a routing failure.
5. **The 600 s codex warm TTL is conservative for current models** (OpenAI documents 30 min for GPT-5.6+; `warmpath` already encodes 1,800 s). The 6/6 in-TTL calibration makes a longer TTL defensible, but routing is machine-local and best-effort, so it needs measurement.
6. **No per-turn "why" in the run log.** A dispatcher sees only `cached_input_tokens` (thread-cumulative); miss attribution requires Codex's OTEL counter, a collector configuration rather than a log field.

## 7. Confidence and gaps

| Claim class | Confidence | Why |
| --- | --- | --- |
| Request fields, key policy, ordering, invalidation reasons | high | source + tests at `rust-v0.164.0` |
| Local vs remote compaction request shape | high | both builders read; #37305 open and matches |
| Incremental-vs-full WS behaviour | high/medium | source; no live wire capture |
| `store:false` + `previous_response_id` server semantics | unknown | not documented in-repo, not measured |
| Cross-thread item-id effect on the provider cache | medium | ids differ by construction; whether the provider hashes them into the prefix is unverified |
| Route-specific cache fields (Bedrock, proxies) | low | no per-route cache code path found |
| Warm-TTL economics | medium | measured runs + documented TTLs, not an A/B |

Not verified: no model was invoked and no request captured, so everything here is source-level. `~/.codex/config.toml`, credentials and the state DB were not read, so the host's effective feature state is unknown. `codex-rs/core/src/agent/host/spawn.rs` 404'd at the tag, so subagent history seeding comes from `spawn.rs` + `session_derived_cache_key` only. The OpenAI prompt-caching guide was unfetchable on 2026-10-28, so provider-layer fields are corroborated from the request struct and local research notes rather than a fresh read.

## 8. Sources

- [openai/codex `core/src/client.rs`](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/src/client.rs) — OpenAI, tag `rust-v0.164.0`, accessed 2026-10-28 — supports: key policy, incremental reasons, request assembly, UUID ordering.
- [openai/codex `codex-api/src/common.rs`](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/codex-api/src/common.rs) — tag `rust-v0.164.0` — supports: exhaustive request fields; `previous_response_id` WS-only.
- [openai/codex `core/src/compact.rs`](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/src/compact.rs) — tag `rust-v0.164.0` — supports: local compaction omits tools, sets `parallel_tool_calls: false`.
- [openai/codex `core/src/compact_remote_v2_attempt.rs`](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/src/compact_remote_v2_attempt.rs) — tag `rust-v0.164.0` — supports: remote compaction sends model-visible specs and appends `CompactionTrigger`.
- [openai/codex `core/src/tasks/compact.rs`](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/src/tasks/compact.rs) — tag `rust-v0.164.0` — supports: strategy chosen by provider capability `remote_compaction`.
- [openai/codex `features/src/lib.rs`](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/features/src/lib.rs) — tag `rust-v0.164.0` — supports: `incremental_tools` stable/on; `tool_search` removed-as-no-op, MCP always deferred.
- [openai/codex `model-provider-info/src/lib.rs`](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/model-provider-info/src/lib.rs) — tag `rust-v0.164.0` — supports: Responses-only wire API; Bedrock rejects websockets.
- [openai/codex `core/tests/suite/prompt_caching.rs`](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/tests/suite/prompt_caching.rs) — tag `rust-v0.164.0` — supports: fixed input order; prefix and key stability tests.
- [openai/codex `core/tests/suite/scenarios_incremental_tools.rs`](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/tests/suite/scenarios_incremental_tools.rs) — tag `rust-v0.164.0` — supports: catalog changes append; fork prefix equality.
- [openai/codex #37305](https://github.com/openai/codex/issues/37305) — opened 2026-08-05, open at access — supports: the local-compaction miss and its measured cost (0.146.1).
- [openai/codex #35300](https://github.com/openai/codex/issues/35300) — open at access — supports: no explicit `prompt_cache_breakpoint`.
- [openai/codex #21796](https://github.com/openai/codex/issues/21796) — open at access — supports: per-thread ids prevent cross-session startup reuse.
- [openai/codex #30425](https://github.com/openai/codex/issues/30425) — open at access — supports: hits remain best-effort with a stable key (unverified here).
- Local: recorded cost model §1.2, §1.4 — 2026-10-21/22 batches — supports: 96.73% and 95.3% cache shares.
- Local: recorded ledger — supports: codex 600 s warm TTL calibration; thread-total usage fix.
- [Local: `src/cache.rs`](../../../../src/cache.rs) — supports: the OpenAI `CacheContract` checked against here.
