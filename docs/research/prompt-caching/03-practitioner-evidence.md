# Practitioner evidence

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research, 2026-10-26
- **Method:** GitHub issues/PRs, Hacker News, maintainer posts, and measured local ledgers
- **Reddit gap:** direct Reddit JSON/HTML access was blocked (403/429). Reddit-sourced claims
  that surfaced secondhand are not treated as primary evidence and are not used for a number here.

## Measured cases

| Case | Measurement | Lesson | Confidence |
| --- | --- | --- | --- |
| Claude Code TTL analysis ([#46829](https://github.com/anthropics/claude-code/issues/46829)) | 119,866 calls on two machines; **$5,561 actual vs $4,612 with 1h TTL**, **17.1%** overpayment; March alone 25.9% | TTL choice is workload-dependent; the counterfactual was disputed by maintainers, so treat the percentage as a scenario, not a universal bill cut | high measurement, medium counterfactual |
| $38k Bedrock bill ([HN 47933355](https://news.ycombinator.com/item?id=47933355)) | 6.47B uncached input, 1.67B cache reads, 101M writes, 25M output; **$37,901.73** gross | “Supported at every layer” is not “actually caching”; a metering proxy is not a spend cap | medium-high |
| Claude Code random UUID ([#27786](https://github.com/anthropics/claude-code/issues/27786)) | random settings UUID in the Bash tool description; **~16,800 writes, 0 reads per turn**, ~12× input penalty | tool definitions are inside the prefix; a random byte in a description is a full cache break | high |
| Claude Code resume ([#40524](https://github.com/anthropics/claude-code/issues/40524)) | a resumed session did **305,735 cache-write tokens** in one turn | resume is not automatically warm; the system/project layers may differ | medium |
| Claude Code git status ([HN 47754795](https://news.ycombinator.com/item?id=47754795)) | empty commit changed a warm 18k-read/0-write session into 11k read / 6k write | volatile git state in the system prompt is a cache breaker; disable or freeze it | medium-high |
| Codex breakpoint ([#35300](https://github.com/openai/codex/issues/35300)) | stable ~9k prefix + fixed key: **0% hits / ~9,060 writes**; with explicit breakpoint **98.6% / 123–128 writes** | implicit-only caching can miss a stable prefix followed by a changing suffix | high |
| Codex compaction ([#37305](https://github.com/openai/codex/issues/37305)) | normal turns 96–99% cached; local compaction omitted tools, re-billed ~235k | compaction must include the tool surface and match the session prefix | medium-high |
| Codex fresh sessions ([#21796](https://github.com/openai/codex/issues/21796)) | identical 11,749-token startup prefix, only **~55%** cached because the key came from the thread id | a per-session key can prevent sharing a byte-identical startup prefix | medium |
| Cline tool loops ([#15307](https://github.com/cline/cline/issues/15307)) | cache reads flatlined at ~15k while autonomous steps paid full input | breakpoints must advance with tool results in agent loops | high |
| Cline Bedrock ([#12913](https://github.com/cline/cline/issues/12913)) | `cache_control` sent where Bedrock reads `cachePoint`; zero reads/writes; ~**$421** wasted | provider dialect bugs are silent | high |
| Cline OpenRouter ([#10596](https://github.com/cline/cline/issues/10596)) | sequential requests hit different providers; **0% hits**, 3.3–4.6× cost | provider pinning is part of cache correctness | medium-high |
| Continue turn caching ([#10801](https://github.com/continuedev/continue/pull/10801)) | large tool result **62.3% → 99.4%**, 8-turn **77.5% → 94.0%** | cache the conversation turn, not only system+tools | high |
| Goose prefix fix ([#11019](https://github.com/aaif-goose/goose/issues/11019)) | **77% → 91%**, input cost **−42%**; OpenAI-compatible **75.6% → 95.0%** | append volatile context; never relocate already-sent bytes | high |
| OpenHands condenser ([#7893](https://github.com/OpenHands/OpenHands/pull/7893)) | 53,755-token conversation, 22,206-token fresh condenser prompt | same-prefix side calls can make compaction mostly cached | medium-high |
| Aider keepalive ([#4372](https://github.com/Aider-AI/aider/issues/4372)) | user saw normal 5m cache reads but no pings after 15 minutes; issue open | keepalive is not a guaranteed feature, and a ping is billable | medium |
| Local DSH measurement | one workload: 1.35M fresh, 177.8M cached, 262.7k output | high cache share coexists with real fresh-input cost; use separate counters | high (local ledger) |
| Local codex batch | 685,887 fresh / 20,281,216 cached / 59,228 output = **96.73% cache share** | cache share alone says little about cache-write and cold-start cost | high (local ledger) |

## What correlates with misses

**Verified against docs/source:**

- model switch, effort-level change, speed-mode toggle;
- tool add/remove/reorder when tools are in the prefix and not deferred;
- rewriting or shortening earlier history;
- compaction that changes the prefix or omits tools;
- a resume after TTL, or a stale session whose system/project context changed;
- gateways/aggregators that drop or mistranslate cache markers;
- provider routing that places requests on different machines/providers;
- volatile content before a breakpoint (git status, timestamps, per-turn context).

**Conditional:**

- MCP add/remove only when tool search is off/unsupported;
- denying a tool only when tool search is off;
- editing `CLAUDE.md` mid-session: the edit does **not** apply until `/clear`, `/compact`, or restart,
  so it does not invalidate the running prefix;
- permission-mode changes: cache-safe except a model-switch mode such as `opusplan`;
- keepalive pings: they cost tokens and can become writes if the prefix changed.

## Myths

| Myth | Evidence |
| --- | --- |
| “Caching is automatic and unbreakable.” | Exact-prefix rule; many silent-miss cases above. |
| “A 99% hit rate means cost is low.” | Hit rate excludes cold starts and writes; cache-read volume can dwarf output but is not work. |
| “1-hour TTL is always cheaper.” | It costs 2× writes; one-shot and subagent work often prefers 5m. |
| “Compaction is cheap because it uses cache.” | Only a same-prefix side call; a fresh summarizer prompt is a full re-read. |
| “Model switching is free.” | Each model has its own cache. |
| “Tool changes are harmless.” | Tool definitions and order are part of the prefix. |
| “Parallel fan-out shares the first write.” | On Anthropic the entry becomes available only after the first response starts. |
| “TTL counts only idle time.” | Anthropic measures from the start of the request that writes/reads. |
| “Keepalive is free.” | Pings are billable requests. |
| “Any common prefix hits.” | DeepSeek requires a persisted unit match; OpenAI has eligible boundaries. |
| “OpenAI writes are always free.” | Model-dependent; newer models charge writes. |
| “Cache is shared org-wide.” | Routing, workspace, org, and provider boundaries apply. |

## Status of the findings (2026-10-27)

The evidence table mixes three different kinds of claim, and a reader needs to know which is
which. The statuses are part of the evidence, not a footnote:

| Finding | Status | What that means for a decision |
| --- | --- | --- |
| Claude Code random UUID in a tool description (`#27786`) | **Observed, unfixed upstream** | The failure mode is real and reproducible; no shipped change removes it, so a harness must keep volatile data out of the cached prefix itself. |
| Claude Code 1-hour TTL overpayment (`#46829`) | **Measured; counterfactual disputed** | The measured spend is evidence; the "17.1 % saved" counterfactual was disputed by maintainers, so it is a scenario, not a forecast. |
| Claude Code resume cache-write spike (`#40524`) | **Observed, unfixed upstream** | "Resume" is not warmth. Measure the resumed run's cache reads; do not infer them from the resume. |
| Codex local compaction omitting tools | **Open at `rust-v0.164.0`** (re-checked 2026-10-28) | The previous "fixed upstream" status was not verified against a tag; the local builder still sends `tools: []` / `parallel_tool_calls: false` (#37305 open). Treat a local-compaction turn as structurally unhittable and exclude it from expected-prefix until a release closes it. |
| Cursor/Goose token-cost improvements | **Vendor-published before/after** | Directionally useful, not independently replicated; do not reuse the percentages as universal constants. |
| Provider cache minimums and write multipliers | **Primary documentation** | Stable enough to encode in a contract, with a version and a date, because they change between model generations. |
| "TTL decays exponentially" | **Not supported** | No provider documents a decay curve. A TTL is a step: warm before the expiry instant, not warm at it. `warmpath` models it as a step and calibrates by age band rather than assuming a curve. |

## Correction (2026-10-28)

The Codex compaction status above was corrected by the [coding-agent caching
research](../agent-caching/agents/codex.md), which read the source at tag `rust-v0.164.0`: the local
compaction builder still omits the tool surface, so a local-compaction turn cannot hit the provider
cache by construction. The measured before/after remains valid as evidence that the fix *would*
work; it is not a statement about every build. The same pass also re-checked the agy zero-hit rows
(pre-telemetry-fix, not evidence) and a borrowed Grok cache share.
