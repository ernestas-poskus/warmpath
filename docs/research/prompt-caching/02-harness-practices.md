# Harness practices

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research, 2026-10-26
- **Scope:** public mechanisms and failure modes in popular coding-agent harnesses
- **Caveat:** Cursor and Amp publish behavior but not source; Codex `main` moves quickly.
  Pin a tag before relying on a specific line.

## Summary table

| Harness | Cache-safe mechanism | Cache-hostile failure |
| --- | --- | --- |
| **Claude Code** | Three layers: system + tools → project context → conversation; plan/skills/style updates appended as messages; tool search defers MCP definitions; compaction repeats the same prefix; forks inherit it; `/rewind` truncates to an already-cached prefix | Model/effort/fast-mode switch; tool-set change when tool search is off; gateway that drops `cache_control`; cold-session `/compact`; git-status or auto-memory paths changing the prefix |
| **Cursor** | Stable tools/system first; explicit breakpoints after stable layers on GPT-5.6+; variable setup in a “phantom user message” past the breakpoints; dynamic MCP and built-in tool loading; system prompt trimmed 66% | Newly loaded tool inserts content before the tail; per-subagent model switching; anything before the breakpoint changing |
| **Codex CLI** | Prefers incremental Responses requests (`previous_response_id` + deltas); records why a request became non-incremental; compaction re-injects base instructions/context | Builds that cannot emit `prompt_cache_breakpoint`; local compaction omitting tools/`parallel_tool_calls`; per-thread cache key preventing identical fresh sessions from sharing startup prefix |
| **Aider** | Fixed order: system → read-only files → repo map → editable files → chat; `--cache-prompts`; optional keepalive pings | Repo map changes mid-prefix; editing attached files mutates cached content; cache stats unavailable while streaming; keepalive reported ineffective in an open issue |
| **Cline** | Provider-native `cache_control`/`cachePoint`; batched read rewrites to preserve prefix; disjoint cache accounting; agentic compaction claims to reuse cache | “Next” engine stopped advancing Anthropic breakpoints in tool loops; Bedrock received `cache_control` where it needed `cachePoint`; OpenRouter routing not pinned → zero hits |
| **Continue** | Pluggable Anthropic strategy; turn-level caching of system + tools + last two user messages; live API cache tests | `optimized` caches the in-flight user turn when large, paying a write that is never read; multiple breakpoints spend the four-slot budget |
| **OpenHands** | Condenser keeps system + initial user + recent events; usage split per `usage_id`; cache-preserving condenser experiment | Original condenser built a brand-new prompt, bypassing the 53,755-token conversation cache with a 22,206-token cold prompt |
| **Goose** | Explicit rule: bytes sent are never moved or edited; per-turn context appended as an immutable block; provider `CacheSemantics`; cache telemetry | Relocating turn context out of the cached prefix cost 23 points of reuse before the fix; OpenRouter/LiteLLM breakpoints on the volatile tail paid writes never read |
| **Gemini CLI** | Compression at 50% of window; keep recent 30%; protect last tool turns; refuse summaries that inflate tokens | Compressor uses a different `systemInstruction` and a verification turn, so it cannot reuse the main prefix; old tool outputs rewritten in place |
| **Amp** | No compaction; truncation to an earlier prefix; handoff starts a new thread with extracted facts | No public cache controls; handoff pays a fresh cold prefix |

## Measured before/after

| Harness | Change | Result |
| --- | --- | --- |
| Cursor | Explicit breakpoints + moving variable setup past them | cold cache misses **−20%**; total token cost **−7%** |
| Cursor | Dynamic MCP tools | **−46.9%** tokens in sessions that call an MCP tool |
| Cursor | Dynamic built-in tool descriptions | **−60%** static-context description tokens |
| Cursor | Line numbers every 10th line | **−1.6%** cache-read tokens |
| Continue | Turn-level caching (last two user messages) | large tool result **62.3% → 99.4%**; 8-turn chat **77.5% → 94.0%**; tool follow-up **81.9% → 99.5%** |
| Goose | Move volatile turn context out of cached prefix | reuse **77% → 91%**, input cost **−42%**; OpenAI-compatible path **75.6% → 95.0%** |
| Codex | Add explicit `prompt_cache_breakpoint` in a controlled replay | **0% → 98.6%** hits; writes **~9,060 → 123–128** |
| Codex | Fix local compaction to include tools | compaction hit **65–88%** cached instead of full re-bill; ~**+70% uncached input** avoided on the measured workload |
| OpenHands | Same-prefix condensation experiment | the original 22,206-token condenser prompt becomes a mostly cached read instead of fresh input |
| Claude Code | `/rewind` instead of `/compact` | truncates to an already-cached prefix rather than building a new one |

## The three reusable patterns

1. **Append-only request assembly, enforced in one place.** Per-turn content goes out as a new
   immutable block; old bytes are never moved or edited. This is the shape every provider's cache
   accepts.
2. **Compaction as a same-prefix side call, then preserve the front.** Repeat system prompt,
   tools, and history; append the instruction; summarize; rebuild history with the stable front
   and the summary at the end.
3. **Make cache health observable.** Separate fresh/read/write counters, a hit ratio, and a named
   reason for the last break. Claude Code surfaces a per-session hit ratio, miss count, warm
   state, and “likely cause” for the last miss; Codex records why a request became non-incremental.

## The three repeated mistakes

1. Volatile content before a breakpoint: timestamps, git status, per-turn context, todo/status
   blocks, or a whole-file repo map before editable files.
2. Caching content that will never be read back: the in-flight user turn, a relocated volatile tail,
   or a per-session key where sessions share a byte-identical startup prefix.
3. Rewriting already-sent history instead of appending: in-place tool-output compression, per-request
   re-collection of extension parts, or a wrong provider marker dialect.

## Correction: slot accounting and resume (2026-10-27)

Two rows above need the detail that changes a design:

- **"Multiple breakpoints spend the four-slot budget" is true, but an automatic marker also spends
  one.** Some harness surfaces place a `cache_control` marker without asking; the budget is four
  slots total, explicit plus automatic, so a harness that adds its own breakpoints before the first
  turn has three left rather than four.
- **A resumed conversation is not a cache hit.** The Claude Code resume evidence (`#40524`) shows a
  resumed session rewriting the entire prefix when the system/project layers moved. Treat resume as
  a *continuation* mechanism and measure the cache reads of the resumed run; `warmpath`'s warm
  decision keeps identity and TTL as gates and reports realized-versus-expected reuse separately.
