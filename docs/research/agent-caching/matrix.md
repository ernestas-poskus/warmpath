# Coding-agent caching matrix

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research, 2026-10-28
- **Scope:** the seven agents × seven strategy classes, at the layer that owns each mechanism,
  with the local contract status and the resulting build/measure/don't-build triage.
- **Reading key:** **M** marker placed by the client · **A** automatic provider prefix cache ·
  **B** explicit breakpoint · **S** strict, no declared contract · **–** none documented.
  Grades: **H** source/vendor-verified, **M** official prose or source-adjacent, **L**
  practitioner, **U** unknown. Cells summarise; the evidence is in
  [agents/](agents/) and [strategies/](strategies/).
- **Companion:** [index](README.md) · [provider mechanics](../prompt-caching/01-provider-mechanics.md)

## The matrix

| Agent | 1. Prefix / prompt cache | 2. Deterministic ordering | 3. Tool & scratchpad output | 4. Semantic | 5. Speculative / subagent | 6. Tool / function | 7. Exact request |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **claude** | **M**, `cache_control` placed by CLI, 5 m/1 h buckets depending on auth; subagent fresh, fork inherits **H** | Documented stable-first layers; five stabilisers; residual cwd/git-status/per-session URL **H** | Prefix-only; no dedupe; compaction and `/rewind` rewrite; no tool cache **M** | – **M** | Fork inherits parent prefix; workflow fan-out holds same-prefix agents 5 s; subagents pay fresh **H** | – (definitions are prompt content, the main invalidator) **M** | – **M** |
| **codex** | **A** + `prompt_cache_key` always (derived from session id); no breakpoint ever sent; WS `previous_response_id` only **H** | Fixed front asserted by test (`[0]` instructions, `[1]` permissions, `[2]` context, `[3]` turn); `uuidv5(thread_id,…)` ids **H** | Truncation at record time; `retained_tool_outputs`; local compaction still sends `tools: []` at 0.164.0 **H** | – **H** | Child sessions share `{source}:{parent}:key`; full-history forks share prefix; WS warmup `generate:false` **H** | Tool-search BM25 index memo; `WebSearchMode::Cached` is server-side **M** | – (turn-scoped `last_request` for WS diffing only) **H** |
| **agy** | Gateway: no marker surface, no flag; Gemini implicit ≥ 4,096 on 3.8/Argon; Claude path unverified **H/M** | Deliberate: rules/paths sorted since 1.1.6; model sticky per turn; membership drifts with host rules/MCP **H** | Local conversation/summary store, `/diff`; model-visible results are prefix only **H** | – **H** | `/fork` clones into a new conversation id; warmth unknown **M** | – (declarations re-sent, retained through compaction) **H** | – **H** |
| **grok** | **A** (xAI), exact prefix from `messages[0]`; conv-id hint, not a lease; no TTL/minimum published; harness source unread **H provider / U harness** | Provider advice only; harness order unknown **M/U** | Unknown **U** | – provider **U** harness | Unknown (not "none") **U** | – **–** | – (a "full hit" is a 100 % prefix hit) **H** |
| **dsh** | **A** (DeepSeek disk cache); append-only derived history; node-0 rewrite vs in-history append is per-model capability **H** | Enforced by construction: sections sort `(order, name)`, tools sorted, header snapshot proves the head did not move **H** | Pruner is one batched pass at compaction pressure; summarize replays the prefix verbatim **H** | – **H** | Fork children reuse a byte-identical completed-turn seed; spawn children start fresh; no barrier **H** | – (every accepted call executes) **H** | – (every step sends a full request) **H** |
| **kimi** | **A** (Moonshot implicit): 5 m/1 h, writes billed 1.0×/2.0×, read 0.1×; CLI assembly undocumented **H provider / L harness** | Provider advice only; CLI order not documented **M/L** | Unknown **U** | – **H undocumented** | `/fork` is an independent copy; warmth unmeasured **U** | – **H undocumented** | – **H undocumented** |
| **perplexity** | Gateway + hosted agent loop: Orchestrator API (preview) exposes OpenAI-dialect key/ttl/breakpoint; Agent API pins one stable key; TTL contradictory **H/M** | Server-side; effort bundles head bytes; exact order undocumented **M/U** | Tool results re-enter the loop; no store **M** | – **M** | No subagent primitive; `max_steps` loop **–** | – customer-visible; `web_search` index is internal **U** | – **M** |

## Provider contract status in `warmpath`

`cache::semantics(provider, model)` resolves a `CacheContract` per family; anything unknown is
`CacheMode::Strict` with unit multipliers and no TTL. The built-in profiles pass the *harness*
name as the provider, so `agy`, `dsh`, `grok` resolve strict unless a dispatch plane sets
`provider` in configuration (one deployment does: the configured provider wins). That is the correct
default, not a gap — but it means the row below describes the *library default*, not the
deployment.

| Provider literal | Family | Contract | State |
| --- | --- | --- | --- |
| `claude` / `anthropic` | Anthropic | Marker · 300 s / 3,600 s · read 0.1× · write 1.25× / 2× · model-dependent minimum (a documented top-level automatic mode is an open question, not a contract change) | declared |
| `codex` / `openai` | OpenAI | Automatic · 1,800 s / 43,200 s · read 0.1× · write 1.25× / 2.5× · 1,024-token minimum; the 12 h long TTL is uncorroborated by the current reference snippet and needs verification | declared |
| `deepseek` | DeepSeek | Automatic · TTL unknown · read 0.1× · write 1.0× · no minimum | declared |
| `gemini` / `google` | Gemini | Automatic · TTL unknown · read 0.25× · write 1.0× · 2,050-token rule · explicit caching legacy-only | declared |
| `agy` / `antigravity` | Unknown → strict | none | **correct**: a gateway may rewrite the request |
| `grok` / `xai` | Unknown → strict | docs: automatic, read 0.25× (`grok-4.7`) / 0.2× (`grok-build`), no TTL, no minimum | candidate family; needs a decision on partial contracts |
| `kimi` / `moonshot` | Unknown → strict | docs: automatic, 300 s / 3,600 s, read 0.1×, write 1.0× / 2.0×, minimum unpublished | candidate family; best-sourced of the three |
| `perplexity` / `sonar` | Unknown → strict | preview, contradictory TTL, no minimum | **keep strict** until the preview settles |

## Build / measure / don't build

| Strategy | Verdict | Why (one line) | Next step |
| --- | --- | --- | --- |
| 1. Prefix / prompt caching | **Keep and invest** | the only reuse that is both safe and large; already measured at 95–99.99 % steady state | remaining: same-prefix compaction in the other planes, tool deferral, an invariance test |
| 2. Deterministic ordering | **Build (cheap)** | is the enabling condition for (1) and the only guarantee a dispatch plane controls | an invariance assertion over scripted steps; tool-manifest digest in `CacheIdentity`; `request_head_digest` for the rendered first message |
| 3. Tool & scratchpad output | **Don't build cross-run; keep artifacts non-model** | reuse mostly moves bytes; staleness is unbounded; the safe win is fewer bytes at record time | keep `ArtifactStore` for documents and artifacts; compaction/pruning stay declared breaks; measure the compaction-pair |
| 4. Semantic caching | **Don't build** | residual is small, match is over the tail, a wrong hit is silent and lands in a code-modifying run | keep any normalized-text routing cache labelled as decision memoization, never as reuse; require the 7-condition evidence bar if ever revisited |
| 5. Speculative / subagent | **Build the barrier only** | the only documented cross-agent reuse is fork-shaped; a warm barrier gets N children onto one write | a barrier grouped by prefix digest + route + workspace, N ≥ 2, never a process pointing at another's session id |
| 6. Tool / function cache | **Build L2, not L1** | tool *definitions* are prefix bytes; a result cache has no attachment point and no duplicate-call measurement | production producer for `canonical_tool_surface`; tool digest in identity; `ToolSurfaceChanged` attribution |
| 7. Exact request cache | **Don't build** | a replay zeroes the usage triple, bypasses verification, and hits exactly when staleness is most expensive | keep single-flight + dispatch idempotency; ensure any proxy replay is off or reported as its own stage |

## Defects and gaps found in the crate

Recorded, not fixed by the research pass: a research task must not change a public compatibility
surface. Rows 4–6 now have their crate half in `warmpath` 0.2.0 (2026-10-28); the request-path
wiring those rows describe is still open.

| # | Where | Finding | Action |
| --- | --- | --- | --- |
| 1 | `src/profile.rs` `grok()` | reads `usage.cached_input_tokens` / `cache_creation_tokens`; xAI exposes nested `prompt_tokens_details.cached_tokens` and no write field; `InputShape::Exclusive` is wrong (cached ⊆ prompt) | fix before any grok measurement; `grok-v2` + profile schema bump |
| 2 | `src/profile.rs` `agy()` | reads `usage.reasoning_tokens` while agy emits `thinking_tokens`; `resume_argv` uses `--resume` where agy's flag is `--conversation`; `Exclusive` unverified | `agy-v2` |
| 3 | one deployment's profile loader | per-agent override exists for `cache_read` but not `cache_write`; a provider with billed writes cannot be pointed at its field | add the override; then Kimi write spend becomes visible |
| 4 | `src/cache.rs` `CacheIdentity` | **Fixed in `warmpath` 0.2.0**: `tools: Option<PrefixDigest>` with `with_tools`/`with_tool_manifest`; the identity digest includes the declared surface and `None` means unknown | request-path producer (`ToolManifest` from the configured MCP/tool surface) still absent |
| 5 | `src/cache.rs` `first_request_break` | **Crate half fixed in `warmpath` 0.2.0**: takes `tool_change: Option<PrefixChange>` and names `ToolSurfaceChanged` before the prefix rewrite it causes | the deployment's adapter still passes no tool change; adopt when the producer lands |
| 6 | `src/prefix.rs` `is_append_only` | **Primitive shipped in `warmpath` 0.2.0**: `check_append_only` + `PrefixVariance` name the first non-extending step (a truncated step is a variance, not an append) | a scripted-sequence assertion over recorded steps is still open |
| 7 | one deployment's first-message renderer | the rendered first message puts the volatile **Budget** block (spend, remaining, reset epoch) before **The plan** and **Workspace**; the prefix digest stops before the message, so a budget change silently rewrites the plan bytes | add `request_head_digest` first; reorder the tail only with a fixture showing the rewrite |
| 8 | Codex 0.164.0 | local compaction still sends `tools: []` / `parallel_tool_calls: false` (#37305 open); turns after resume are `restored_history` | exclude compaction and first-post-resume turns from EEPR, report separately |
| 9 | Codex `exec --json` | `turn.completed.usage` is a thread running total, and non-incremental reasons exist only in an OTEL counter | subtract a baseline; accept that miss attribution needs the collector |
| 10 | `agy` calibration rows | every recorded zero-hit row predates the `cache_read_tokens` fix and carries synthetic ids / 2024 epochs | not evidence; re-measure on 1.4.8 and record `agy --version` |
| 11 | run records | no run row stores the harness CLI version, so a measurement cannot be tied to a build | record version per run (the harness notes already require it) |

## The one-line conclusion

Every strategy on the list except prefix caching is either a **store** that must be invalidated,
an **anti-cache** that trades context for prefix, or a **scheduling trick** with side effects.
The repository already owns the only mechanism that matters; the remaining work is to make the
request bytes provably stable (ordering test, tool manifest, head digest), get N children onto
one cache write (barrier), and stop misreporting the three providers whose profiles or usage
paths are wrong.
