# agy (Antigravity CLI) — caching behaviour

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research, 2026-10-28
- **Scope:** what `agy` (Google Antigravity CLI, repo-studied release **1.4.5**) caches, forwards or loses across the three layers it sits between, and what its telemetry supports. The flag surface is the companion harness research shipped with the `workcell` crate (agy-deep, agy-web); provider mechanics are [01](../../prompt-caching/01-provider-mechanics.md).
- **Method:** local files read (§8); 3 web searches, 3 pages fetched; access date 2026-10-28. CLI touched for version context only: `command -v agy`, `agy --version` → **1.4.8**, bundled `agy changelog` (head **1.4.9**) — read-only, zero tokens, no model turn.
- **Companion:** [index](../README.md) · [matrix](../matrix.md)

## 1. What it is and where caching lives

`agy` is Google's Antigravity CLI: one binary (logs name Go sources such as `cache.go:135`) running one agent loop that serves **several model families from one account** — Gemini (`3.8/3.7/3.6-flash`, `3.1-pro`), the separate `gemini-4-argon` allowance, Anthropic Claude (`sonnet/opus-5-5`, `4-6`) and `gpt-oss-120b`. Those are "leading frontier reasoning models from Google DeepMind and **partner providers**" [M1]; `agy models` lists 13 effort-suffixed slugs on 1.4.5 (agy-web §7); one deployment splits their quota into three pools by recorded decision. agy is harness *and* gateway, as `warmpath`'s design says — and its only documented base-URL seam is Google-shaped (`GOOGLE_GEMINI_BASE_URL`, "the CLI appends `/v1beta/...` itself", agy-web §5).

| Layer | Who owns the prompt cache | Evidence |
| --- | --- | --- |
| (a) provider | Gemini **implicit** prefix cache, automatic, no request-side control | [M2] |
| (b) agy harness | conversation store + resume, compaction, rule ordering — *prefix stability*, not markers | changelog 1.1.6/1.1.8/1.4.4; [M3] |
| (c) gateway | Claude/GPT: Google's partner serving; in a dispatch, a compressing and/or rewriting proxy via `GOOGLE_GEMINI_BASE_URL` | [M1] |

The house contract holds: `HarnessProfile::agy()` sets `provider: "agy"`, a deployment writes `provider = "antigravity"`, and both spellings are `ProviderFamily::Unknown` → `CacheMode::Strict`: no marker, no TTL, unit multipliers, no claimed saving (`src/cache.rs:203-259`; pinned by the `unknown_routes_are_strict…` test). **No prompt-cache logic lives in agy** — no cache-shaped key in `/config`'s 40 keys, no flag, no doc or changelog line naming `cache_control`, `cachedContent` or a breakpoint.

## 2. Strategy table

| Strategy | Mechanism observed | Evidence | Grade |
| --- | --- | --- | --- |
| Prefix / prompt caching (context re-use) | Gemini implicit only: prefix-based, automatic, ≥4,096 tokens on `gemini-3.8-flash` / `gemini-4-argon`; no agy marker, TTL or breakpoint; explicit `CachedContent` unused and unsupported on Argon+ | [M2]; changelog 1.1.8 | **medium** (mechanics high; a real hit: unmeasured) |
| Deterministic ordering | Rules and paths sorted deterministically since **1.1.6** to avoid "needless prompt-cache misses"; model sticky inside a turn | changelog 1.1.6; [M1] | **high** |
| Tool & scratchpad output caching | No model-facing cache; local state caches only (conversation JSON, summary store, `/diff` turns, changelog file) | changelog 1.0.4/1.0.11/1.4.8; agy-deep §5 | **high** |
| Semantic caching | None documented: no key, flag, page or release note | `/config`; changelog grep; 3 searches | **high** |
| Speculative / subagent execution caching | Subagents carry their own `conversation_id`; `/fork` clones history into a new conversation; no speculation | agy-deep §4/§8; [M3] | **medium-high** |
| Tool / function cache | None; declarations are re-sent every request and deliberately *retained* through compaction | changelog 1.4.4; agy-deep §8 | **high** |
| Exact request cache | None at the model layer; "cache" in agy means files | changelog 1.0.4/1.4.8; [M3] | **high** |

## 3. Findings by strategy

### Prefix / prompt caching (context re-use)

- **Provider (high).** Gemini implicit caching is on by default for 2.5+, "savings not guaranteed", prefix-based ("the model doesn't make any distinction between cached tokens and regular input tokens; cached content is a prefix"), hits in `usage_metadata`. Minimums: **4,096** on Gemini 4 Argon, 3.8 Flash, 3.6 Flash, 3 Pro preview; **2,048** on 2.5. No TTL published. Explicit caches (1 h default, storage-priced) are "not supported on Gemini 4 Argon and later" [M2]. `warmpath` still encodes the 2,050-token 2.5-era minimum (`src/cache.rs:247-257`) — unreachable for agy.
- **agy (medium-high): no marker surface.** No flag, no `settings.json` key, no `cache_control`/`cachedContent` in the envelope, no docs page. With a Gemini-dialect client and a Google-shaped base URL an Anthropic breakpoint has nowhere to go. Absence of markers does **not** prove absence of caching on the Claude path: some Anthropic surfaces place an automatic marker themselves ([01](../../prompt-caching/01-provider-mechanics.md) §Corrections), and for agy's Claude/GPT families the cache is whatever the partner gateway does — undocumented, unobservable.
- **What agy does deliberately is keep the prefix stable**, and says so three times:
  - 1.1.6 — *"Improved customization discovery by sorting rules and discovered paths deterministically, preventing unstable prompt ordering and needless prompt-cache misses."*
  - 1.1.8 — *"The JSON usage object emitted by `json` and `stream-json` now reports token accounting including `cache_read_tokens`, so non-interactive consumers can attribute prompt-cache hits."*
  - 1.4.4 — *"Improved `/compact` and automatic compaction to retain tool declarations on summarization requests so the model API reuses its cached prompt prefix."*

That is an **automatic-cache posture**: shape the request so the provider notices a repeated prefix rather than spend breakpoints. Codex-like, not Claude-Code-like — and compaction is cache-aware by design, which [02](../../prompt-caching/02-harness-practices.md) lists as a common failure.
- **Telemetry is the only observation point.** `usage.cache_read_tokens` is *reported*, and agy reports 0 when nothing was served. After the path fix (§4) a 0 is a measurement; before it, 0 was an artefact. Both exist in the recorded runs and must not be pooled.

### Deterministic ordering

- **Within agy (high).** 1.1.6 makes rule/path order deterministic *for cache reasons* — the only first-party admission that ordering is a cache variable. Order is fixed by the build, not per run.
- **Model identity is sticky inside a turn (high).** "Your choice of reasoning model persists between messages within a conversation… if you change the reasoning model while the agent is running, it continues to use the previously selected reasoning model until it completes its steps for that turn" [M1]. No mid-turn route change; across turns it may change, and a slug suffix aliases `--effort`: `gemini-3.8-flash-medium` + `--effort high` resolves elsewhere.
- **Across providers, ordering is not agy's decision.** The five `[[agents]]` entries (`agy-claude-opus`, `agy-claude-sonnet`, `agy-gpt`, `agy`, `agy-argon`) are one family each, ordered by the dispatcher's `prefer` chain. agy has no notion of "prefer the family whose prefix is warm"; a family switch is a new conversation with a new prefix shape.
- **Host state leaks into the prefix (medium).** Rules are **cumulative** (workspace + `~/.gemini` + `~/.gemini/config` + `~/.gemini/antigravity-cli/rules` + plugin rules) with a 24 KB per-file cap and a 20,000-token budget that demotes the largest files to pointers (agy-deep §7). Ordering is deterministic; the *input set* is not.

### Tool & scratchpad output caching

- No model-facing cache: tool results are re-sent with history and are cheap only when the provider serves that prefix. No per-tool memoisation, no scratchpad store reused across turns.
- Local caches buy latency, not tokens: `cache/projects.json`, `last_conversations.json`, the summary/trajectory SQLite store, a persistent `/resume` metadata cache (1.0.11), a `/diff` finished-turn cache (1.4.8: "caching finished turns so reopening `/diff` only recomputes the latest one"), a bundled changelog file cache (1.0.4, made synchronous).
- One token-reducing (not cache) change: 1.4.7 returns on-demand MCP results up to 10 KB inline instead of saving them to a file for a follow-up read.

### Semantic caching

- None documented — not in the docs pages, not in ~1,000 changelog lines, not as a `/config` key, not as a flag. Two byte-identical requests are two provider calls. Grade **high**, and the absence is broad enough that an undocumented switch is unlikely.
- What agy calls caching is file/serialisation caching: every changelog line containing "cache" that I checked is a local file cache — do not read one as a prompt-cache claim.

### Speculative / subagent execution caching

- Subagents get their own conversation and `conversation_id` (`stream-json` `subagent_info`, agy-deep §4), their own system prompt and tool set, and headless runs skip their questions (1.3.6). No shared prefix by construction; no documented speculative pre-execution or cross-subagent prefix sharing.
- `/fork` (alias `/branch`) is the one reuse primitive: it "clones the entire conversation history into a new independent session" with a new id [M3] — byte-identical prefix at the fork point, then divergence. Useful to a human; useless to a dispatch plane, whose warm identity is keyed on the captured `conversation_id` plus work item, plan, and agent.
- 1.2.5 puts the subagent roster into a custom agent's system prompt: a per-agent prefix difference, so a subagent's history is never a suffix of its parent's.

### Tool / function cache

- None. Tool declarations ride every request — which is exactly why 1.4.4 keeps them on the summarisation request: dropping them breaks the prefix. A plugin install/disable, a `hooks.json` change or a user-level MCP server appearing changes the tool block and therefore the prefix, and agy has no per-run MCP file, so that block is **host state** (agy-deep §8, §10).

### Exact request cache

- None at the model layer. The closest reuse is `--input-format stream-json`: one process, one conversation, many prompts, cumulative usage — continuity, not an exact-request cache. Every "cache" identifier in the binary's vocabulary is a file (§3, local caches).

## 4. Measured / quantified evidence

| Measurement | Value | Source | What it supports |
| --- | --- | --- | --- |
| Fresh short turn, 1.4.5 | `input 11,261 / output 28 / thinking 19 / cache_read 0`, 3.86 s | agy-deep §5 | ~11.3 k fixed prefix, clears 4,096; a first turn has nothing to reuse |
| Recorded agy run | 176,182 fresh / **0 cached** / 18,256 out; ≈$0.2006 on Flash list | recorded cost model | a real billed run, but it **predates** the path fix |
| Warm-resume calibration, agy | 6 rows, 0 hits in `<2m`, `2m–5m`, `5m–10m`; 0 cached tokens | warm-resume calibration data | **not a provider measurement**: the rows carry synthetic ids (`agy-w1-1`) and **2024** epochs |
| Field availability | `usage.cache_read_tokens` since **1.1.8**, confirmed on 1.4.5 | changelog 1.1.8; agy-deep §4 | the field exists and is the right one |
| Field extraction | `cache_read_tokens` first in `HarnessProfile::agy()`; adopted 2026-10-22 | `src/profile.rs:321-331` | the extraction defect is **fixed**; recorded zeros predate it |

Two extraction mismatches remain, both biasing the cache picture (found here, not recorded elsewhere):

1. **Reasoning path.** The profile reads `usage.reasoning_tokens` (`src/profile.rs:330`), but the envelope emits **`usage.thinking_tokens`** (agy-deep §4) — every agy run records zero thinking tokens, so the 19-token figure above never reaches the profile path.
2. **Input shape.** The profile declares `InputShape::Exclusive` (input is fresh, reads additive). Nothing documents that, and Gemini reports cache reads as a *subset* of prompt tokens [M2]. If agy passes that through, `fresh = input` understates the denominator and the share is biased low — the Codex 47.3 %-instead-of-89.77 % defect, mirrored. Testable only once reads are non-zero.

**What can be concluded:** a zero was *reported*; it was not always *measured*. The defensible sentence is "0 reported in every recorded agy run, none of which post-dates the fix and none of which is a live calibration sample".

## 5. Failure modes and invalidators

- **Route change.** Any model or effort change is a new route (slug suffix aliases `--effort`), hence a cold prefix; the warm identity re-check treats `model_changed`/`effort_changed`/ `provider_changed` as cold.
- **Prefix-input drift.** Cumulative rule discovery, home `~/.gemini` rules, plugin rules and the 20 k demotion budget: same checkout, different host, different system prompt.
- **Tool-block drift.** Plugins, hooks and user-level MCP servers are host state; a plugin load failure is non-fatal and silently changes the toolset (1.4.3–1.4.5 fixes). Headless runs block on MCP startup.
- **History rewriting.** Compaction (`/compact`, automatic) rebuilds the middle; 1.4.9 drops unmatched tool calls/results and outdated thinking blocks; 1.4.7 fixed a loop where one oversized message caused repeated re-summarisation. Each rewrite shifts the prefix, mitigated only by 1.4.4's retained tool declarations.
- **Unwritable `$HOME`.** The summary/trajectory store fails to open, `--conversation` resume degrades to fresh, and a warm record becomes a lie (agy-deep §5) — while the run still exits 0.
- **A rewriting proxy in front.** The deployment terminates agy's base URL with a compressing proxy and optionally a request-rewriting proxy (compression only; cache-alignment and exact-cache modes are off). Live-zone compression rewrites bytes before the provider, and rewriting already-cached content turns a read into a write; pre-provider savings are never provider cache savings.
- **Unknown TTL.** No TTL is published for implicit caching; 300 s is a conservative house default.
- **Version drift.** Research pins 1.4.5; this host reports **1.4.8** with a changelog headed **1.4.9**, and no run row records the CLI version.
- **Exit-code traps that look like cache facts.** `--print-timeout` expiry returns partial output with exit 0 (1.1.28); a soft-denied tool exits 0 with nothing changed.

## 6. Consequences for `warmpath`

1. **Keep the provider literal outside the four known families.** `agy`/`antigravity` → strict is correct. Relabelling an agy agent `provider = "gemini"` is a one-word change that would hand a gateway route a 2,050-token minimum and a 0.25 read multiplier with no evidence about the gateway — and 2,050 is wrong for the models agy lists (4,096). If a deployment does it, that is a claim.
2. **Re-measure before any warm-ranking conclusion.** Two runs, same frozen request, second run `--conversation <id>`; record `agy --version`; check `cache_read_tokens > 0`; check whether `input_tokens` stays flat as cached rises (settles Exclusive vs Inclusive); note whether a compressing/rewriting proxy was in the path, since a rewriting proxy confounds the result.
3. **Fix the profile as a version bump.** It is `agy-v1` with `PROFILE_SCHEMA_VERSION` 1, so each of these is `agy-v2`: `resume_argv` is `["--resume", "{session}"]`, **not** agy's form (`--conversation <id>`; one deployment overrides it, a library consumer would not); the reasoning path is `reasoning_tokens` instead of `thinking_tokens`; `input_shape: Exclusive` is unverified.
4. **Do not pool pre-fix and post-fix zeros.** AICS and EEPR must exclude runs recorded before the fix: a reported zero from a correct path is a measurement, a zero from the wrong path an absence — "unknown is not zero" applied to a *run*, not to a field.
5. **Keep 300 s, labelled.** It is a policy bound with no provider TTL behind it, and calibration must not tune it from agy until reads are non-zero: six zero-hit rows cannot distinguish "provider evicted" from "we never asked correctly".
6. **Build nothing for markers.** There is no surface to place a breakpoint on and no flag to pass one through. The only levers are a byte-stable prefix (frozen request, pinned rules, pinned tool set) and conversation continuity — both already the dispatch plane's job.
7. **Treat cross-host warm reuse as unsound** until the tool/rule block is pinned: the same checkout on two hosts can send different system prompts and tool sets, so an agy warm record is a record about a host as much as a workspace.
8. **A returned cache number is not a cache contract.** `cache_read_tokens` is the only number available, and it is exactly the kind `warmpath` was built to distrust: present, plausible, and meaningless without knowing which layer served it.

## 7. Confidence and gaps

- **High:** no documented prompt-cache control surface; deliberate cache-shaping is prefix stability (1.1.6, 1.4.4); the field is `usage.cache_read_tokens` since 1.1.8; no semantic, exact-request, tool or speculative cache is documented; local file caches exist and are not model caches.
- **Medium:** that every family is requested through one Gemini-dialect client (inferred from the single documented base-URL seam and the `/v1beta` append — no wire capture); that `input_tokens` is inclusive of reads (Gemini reports it so; agy's envelope is undocumented); that host rule/tool state invalidates cross-host reuse.
- **Low / unknown:** whether the Claude path gets Anthropic-style caching from the partner gateway; whether agy emits a marker the docs omit (needs a body capture); whether agy's turns have *ever* hit a cache.
- **Not verified:** a live two-run measurement on 1.4.8; a request-body capture (the seams exist — a compressing proxy already terminates the base URL — but its contract is metadata-only, so it would be a deliberate isolated experiment); subagent tokens inside the parent's `usage`; the conversation store's retention; whether `/fork` stays warm provider-side. `agy-deep`/`agy-web` predate 1.4.8 and their per-version claims were not re-derived here.

## 8. Sources

Local (accessed 2026-10-28; each supports the fact named after it):

- The companion harness research shipped with the `workcell` crate, agy-deep — agy 1.4.5, 2026-10-22 — envelope fields; resume and `conversation_id`; confined-`$HOME` failures; host MCP/plugin surface; the one measured turn.
- The same companion research, agy-web — agy 1.4.5, 2026-10-22 — `usage.cache_read_tokens` as the cached field; the extraction defect; `GOOGLE_GEMINI_BASE_URL`/`/v1beta`; `/config` keys.
- [01-provider-mechanics](../../prompt-caching/01-provider-mechanics.md) — 2026-10-26/27 — Gemini implicit vs explicit; the automatic-marker caveat; prefix invalidators.
- [02-harness-practices](../../prompt-caching/02-harness-practices.md) — 2026-10-27 — compaction as a failure mode; the three reusable patterns.
- [warmpath DESIGN](../../../../docs/DESIGN.md) — "the provider is a field on `HarnessProfile`, never inferred… `agy` is a gateway"; unknown routes are strict.
- [src/cache.rs](../../../../src/cache.rs) — `ProviderFamily`, `semantics()`, the strict contract, the agy-is-strict test.
- [src/profile.rs](../../../../src/profile.rs) — `HarnessProfile::agy()` fields: `provider`, `resume_argv`, reasoning path, `input_shape`, `warm_ttl`.
- The deployment's profile loader — configured `provider` wins over the harness name; usage-path override semantics.
- Its recorded cost model — the 176,182/0/18,256 run and its pre-fix caveat.
- Its warm-resume calibration records — the six agy zero-hit rows **and** the synthetic ids / 2024 epochs that make them illustrative.
- The 2026-10-23 recorded decision on model pools — three pools; one entry per family; slug suffix as effort alias.
- The cost-attribution ledger — `agy-cached-token-path` (2026-10-22), `warm-cache-ttl-calibration` — the fix's adoption and its stated limit.
- `agy --version` (1.4.8), `agy changelog` (bundled, head 1.4.9), `command -v agy` — version context and the 1.1.6/1.1.8/1.4.4/1.4.8/1.4.9 first-party statements quoted in §3.

Web (2026-10-28):

- [M1] [Models — Google Antigravity Docs](https://antigravity.google/docs/models) — Google, undated (lists Gemini 4 Argon and Claude 5.5, so 1.4.x-era) — "models from Google DeepMind and partner providers"; the per-plan model table; sticky model choice within a turn.
- [M2] [Context caching — Gemini API](https://ai.google.dev/gemini-api/docs/generate-content/caching) — Google, updated 2026-10-23 — implicit caching automatic on 2.5+ with no saving guarantee; minimums 4,096 (Argon/3.8 Flash) and 2,048 (2.5); no TTL; cached content is a prefix; explicit caching unsupported on Argon+.
- [M3] [Managing conversations — Antigravity CLI](https://antigravity.google/docs/cli/conversations) — Google, undated — history scoped to the working directory ("directory-scoped session cache"); `-c`/`--continue`; `/fork` cloning history into a new id.
- [google-antigravity/antigravity-cli CHANGELOG](https://github.com/google-antigravity/antigravity-cli/blob/main/CHANGELOG.md) — Google, `main` at 1.4.9 — seen only as a search result; the same entries were read locally via `agy changelog`, which is the citation used above.
