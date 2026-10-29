# Claude Code — caching behaviour

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research, 2026-10-28
- **Scope:** Claude Code (Anthropic) as a coding-agent harness: where its prompt cache lives, what the harness does to keep a prefix reusable, what it caches itself, and which claims are folklore. Version ground truth is the installed
**2.1.303**; the vendor page is undated and describes the **2.1.290**-era. Provider mechanics are not restated — see [01](../../prompt-caching/01-provider-mechanics.md).
- **Method:** 13 local files read ([prompt-caching 01](../../prompt-caching/01-provider-mechanics.md)–[02](../../prompt-caching/02-harness-practices.md), [04](../../prompt-caching/04-sources-and-confidence.md), the
companion harness research shipped with the `workcell` crate, [warmpath DESIGN](../../../../docs/DESIGN.md), `src/{cache,profile}.rs`,
and one deployment's cost model, cost ledger, and warm-resume report); 6 web sources (3 fetched,
3 snippet-only); access date 2026-10-28.
- **Companion:** [index](../README.md) · [matrix](../matrix.md)

## 1. What it is and where caching lives

Claude Code is a **closed-source CLI** that re-sends the whole context every request — system prompt, project context, every prior message and tool result, then the new message — appending only the new part. The vendor states there is "no
per-file or per-segment caching": the reuse unit is the request prefix. (high: vendor page)

The cache is **not in the CLI** but in whichever infrastructure serves the model: Anthropic's (API key, subscription, Claude Platform on AWS); the cloud's (Bedrock, Google Cloud Agent Platform, Foundry — Azure-hosted on Azure,
Anthropic-hosted on Anthropic); or behind `ANTHROPIC_BASE_URL` / an LLM gateway, "wherever your requests are forwarded", with correctness depending on the gateway.

**Claude Code caches nothing of the model's output.** No response cache, tool-result memo or local prefix store is documented in any page checked; what it keeps is *session state* (transcripts, the recorded system prompt, session ids),
which makes a prefix *stable*, never warm. (medium: absence across the vendor page, CLI reference and three issue reads; closed source, so "none documented".)

Three layers stay apart: (a) **provider semantics**, a billing fact ([01](../../prompt-caching/01-provider-mechanics.md), `warmpath::cache::semantics`); (b) **harness context reuse**, what the CLI orders, appends and requests as a TTL; (c)
**client/gateway caches**, where a proxy may forward, move or strip the marker. `warmpath`'s 300 s window is a fourth thing: a decision threshold, not a claim about which bucket was billed.

## 2. Strategy table

| Strategy | Mechanism observed | Evidence | Grade |
| --- | --- | --- | --- |
| Prefix / prompt caching (context re-use) | Explicit Anthropic `cache_control`, placed by the CLI with no user action; exact match over system prompt + tools → project context → conversation | vendor page ("handles prompt caching for you"); vendor blog; 99.998% cached share measured in one deployment | high |
| Deterministic ordering | Fixed order, stable-first; per-machine content optionally moved out of the system prompt | vendor page; blog "static content first, dynamic content last" | high |
| Tool & scratchpad output caching | **none found** as a cache — tool results are conversation bytes, cached only as prefix, evicted or rewritten by image caps and compaction | vendor page; no dedup or tool-result cache documented | medium (absence) |
| Semantic caching | **none documented** | no embedding/nearest-match layer in any page; prefix match is exact | medium (absence) |
| Speculative / subagent execution caching | No speculation. One documented warm-start: workflow fan-out holds same-prefix agents up to 5 s so they read the first agent's cache; a subagent otherwise pays fresh; a fork inherits the parent's | vendor page §Subagents, §Workflow fan-outs | high for mechanism, **none** for speculation |
| Tool / function cache | **none found** — tool *definitions* are prompt content (and the main invalidator), not a cache | vendor page §MCP; blog "Never add or remove tools mid-session" | medium (absence) |
| Exact request cache | **none documented**; resume re-sends and re-reads, never replays a response | vendor page §Resuming; [01](../../prompt-caching/01-provider-mechanics.md) `#40524` | medium (absence) |

## 3. Findings by strategy

### Prefix / prompt caching (context re-use)

- **The CLI places the marker** ("Claude Code handles prompt caching for you, unless you disable it"). A documented exception proves it is the placer: it appends system context mid-conversation (file-change notices) and "marks that block
for caching on every provider and connection unless you set `CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS`". (high: vendor page)
- **Breakpoint count and positions: not documented.** The CLI is closed source and `cache_control` is not written to transcripts (the `#63930` reporter says so). Knowable: the API caps a request at four breakpoints and the CLI spends at
least one on that mid-conversation block, so a harness layering its own markers has at most three (inference, high, from vendor page + [01](../../prompt-caching/01-provider-mechanics.md)).
- **TTL is a per-request bucket choice, not a provider setting.** *Main conversation* (interactive, `-p`, Agent SDK, inline helpers) gets **1 hour on a subscription within plan usage** and **5 minutes on credits/API key/cloud**; *everything
else* (subagents, workflows, in-process teammates, forks, compaction, session titles) gets 5 minutes on both; over plan limit the main conversation drops to 5 min. Controls: `promptCacheTtl`/`CLAUDE_CODE_PROMPT_CACHE_TTL` and
`subagentPromptCacheTtl`/`CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL` (≥2.1.242; only `5m`/`1h`). Precedence: `FORCE_PROMPT_CACHING_5M=1` → bucket env var → bucket setting → subagent `experimental.cacheTtl` (≥2.1.248) →
`ENABLE_PROMPT_CACHING_1H=1` → bucket default. (high)
- **Warm reuse per action:** resume re-reads "whatever part of its prefix is unchanged and still within the cache lifetime"; `/rewind` truncates to a prefix "already cached" that later turns kept warm; `/compact`'s summarization request
repeats system prompt, tools and history, so while warm it "reads your prefix from the cache"; a fork shares the parent's prefix exactly; a resumed subagent can read the cache its original run warmed. (high: vendor page)
- **A resume is not warm by construction** — it continues and reads whatever survived. The `#40524` correction stands: a resumed session can rewrite the whole prefix ([02](../../prompt-caching/02-harness-practices.md) §Correction).

### Deterministic ordering

Documented layer order, stable first: **system prompt (core instructions + tool definitions)** → **project context (CLAUDE.md, auto memory, unscoped rules)** → **conversation (messages, responses, tool results)**; the first changes when the
tool set changes, the second at session start, `/clear` or `/compact`, the third every turn. (high: vendor page)

The vendor blog states the rule prescriptively — "static content first, dynamic content last" — as four layers (static system prompt & tools globally cached → CLAUDE.md per project → session context per session → conversation), and names
the three ways Anthropic broke it themselves: a timestamp in the static prompt, non-deterministic tool-order shuffling, and editing the Agent tool's parameters. (high: vendor blog, unmeasured)

Five documented mechanisms keep the order stable, each a rule a harness can copy: (1) `--system-prompt-snapshot on` (default) renders the prompt on the first request and replays it verbatim later and on resume until compaction — under
`--bare` recording is off unless passed; (2) `--exclude-dynamic-system-prompt-sections` moves cwd, env info, memory paths and the git-status snapshot into the first user message; (3) plan mode, skills, commands, output styles and plugin
text arrive as **messages**, never prompt edits; (4) plan mode keeps all tools and adds `EnterPlanMode`/`ExitPlanMode` as tools instead of swapping the tool set; (5) MCP tool search sends `defer_loading` stubs in a fixed order rather than
adding or removing definitions.

What is *not* stable **across runs**: the prefix is scoped to one machine and one directory. The system prompt embeds auto-memory paths, the conversation opens with cwd/platform/shell/OS, and sequential sessions share a prefix only when the
startup git-status snapshot matches; parallel sessions in one directory can read each other's cache. (high: vendor page §Cache scope.) For a dispatch plane that is decisive: a fresh worktree path and a dirty tree are prefix changes.

### Tool & scratchpad output caching

- **Cached:** tool results are conversation content, so once written they sit in the prefix and are read at the cached rate later. No separate tool-result cache, cross-session memo or dedup is documented anywhere checked.
- **Rewritten, three documented ways:** image/PDF limits drop a *batch* of the oldest attachments, changing the messages that held them and reprocessing "from the earliest of those messages onward"; compaction replaces history with a
summary; `/rewind` truncates. Repo file edits are *not* a rewrite — the CLI appends a `<system-reminder>` and Claude re-reads. (high: vendor page)
- **Tool loops:** appending a result is the cheap case — it lands after the last breakpoint and the prefix still hits. Two reported breakages, unconfirmed: a turn with many parallel `tool_use` blocks followed by `cache_read` collapsing to
the system+tools floor, and `ToolSearch` materialising a deferred tool zeroing `cache_read` (`#63930`, 2.1.158, one reporter). His theory is a *rolling* message-history breakpoint; the documented design says stubs stay in the same order and
deferred deltas arrive as message content. Design vs single report — not settled. (low)
- **Tool-surface change is the main invalidator**: "Never add or remove tools mid-session."

### Semantic caching

**None documented** — no embedding-based or normalised-text cache of responses, and the provider's match is exact ("a change anywhere in the prefix recomputes everything after it"), the opposite of semantic matching. Two deployment-level things are
easy to mistake for it and are not: a routing cache keyed on a normalized task string decides **routing**, and a compressing proxy rewrites request bytes before the provider. Neither stores or replays model output.

### Speculative / subagent execution caching

Documented and precise (high: vendor page §Subagents and the cache):

- A **subagent** "starts its own conversation with its own system prompt and tool set … Its first request doesn't read the parent's cache … and it warms a cache of its own across its turns." It sits in the *everything else* bucket — **5
minutes even on a subscription** unless `subagentPromptCacheTtl` says otherwise. The parent's cache is unaffected.
- A **fork** "inherits the parent's system prompt, tools, and conversation history exactly, so its first request reads the parent's cache"; a **resumed** subagent can read the cache its original run warmed.
- **Workflow fan-out** of same-prefix agents: Claude Code holds all but the first for up to 5 s (default) "so their first requests can read the prefix that the first agent cached" — a delay, not a prediction.

Not documented / not verified: whether the `--fork-session` **flag** preserves the prefix (the vendor's "fork" statements concern the subagent/session fork features), and there is no public API to pre-warm a prefix for a future process.
Consequence for a run spawning N subagents: N fresh first requests at the write rate, and `usage` excludes subagent tokens while `modelUsage`/`total_cost_usd` include them (the companion harness research, claude-web §1).

### Tool / function cache

**None found.** Nothing caches a tool invocation or its result. Tool *definitions* play the opposite role: static prompt bytes whose stability the cache depends on, and every tool-surface change is an invalidator. The documented cases where
a tool change is free are workarounds — the advisor tool's definition "sits after the cache breakpoint", `defer_loading` stubs keep the list fixed, and a plan-mode toggle is a tool call.

### Exact request cache

**None documented.** The only "same request again" mechanisms are session-level: `--resume` re-sends the transcript and hopes the prefix survived, `/rewind` truncates to a prefix later turns kept warm. Neither replays a stored response, and
neither guarantees a hit — TTL, model, effort, tool set, project context and the recorded system prompt must all still match.

## 4. Measured / quantified evidence

The strongest numbers are one deployment's own, from `usage.cache_read_input_tokens` / `usage.cache_creation_input_tokens`:

- **One claude implementation run (2026-10-21):** 28 fresh / 744,078 cached / 6,713 output tokens, **$0.2832** API-equivalent at Opus 5.5 list ($4 in, $0.20 cached, $20 out per M). The arithmetic closes only at ~**100.0%** input from cache:
`744,078 × $0.20/M = $0.1488`, `6,713 × $20/M = $0.1343`, `28 × $4/M = $0.0001`. (high: recorded runs)
- **A three-run claude batch:** 208 fresh / **8,639,036 cached** / 59,838 output → $2.9254 = **99.998%** of input cached; same list-price check (1.7278 + 1.1968 + 0.0008). (high: recorded ledger)
- **The seven-run implementation batch containing them:** 10,215,612 tokens, 9,344,700 cache reads (**93.05%**) — and **all seven runs are `warm=false`**: no warm resume at all. The reuse is within-run, turn-to-turn prefix growth; a plan
assuming warm-resume is where the savings live is pulling the smaller lever. (high: same ledger section)
- **TTL calibration for this harness:** claude 4/4 in-TTL hits (`< 2 min` 2/2, `2–5 min` 2/2), 0/2 in the `5–10 min` control band → the 300 s step is corroborated at n=6; the `10–30 min` and `> 30 min` bands are recorded unmeasured.
(medium: warm-resume calibration report)

## 5. Failure modes and invalidators

**Documented invalidators** (high: vendor page) — each a one-time slow, expensive turn, then a rebuilt prefix:

- model switch (`/model`, `opusplan` toggles, automatic fallback, a skill frontmatter naming a model) — caches are per model, and `/model` only asks confirmation while the cache is warm (≥2.1.238);
- effort change — per-effort caches on most models, **cache-safe** on Opus 5.5 / Sonnet 5.5 / Haiku 5.5 / Fable 5.1 with API key or subscription, but not on Bedrock, Agent Platform, a Claude apps gateway,
`CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS`, or a HIPAA org;
- fast mode on/off — separate caches per speed;
- MCP server connect / tool add (invalidates), server drop (keeps; calls error), remote reconnect (keeps, unless `WaitForMcpServers` is added once), deliberate tool removal (invalidates) — none of it applies when tool search defers the
tools, because the first request's list is pinned;
- plugin enable/disable — free for skills/commands/agents/hooks/themes (appended as messages), invalidating for one providing MCP servers unless deferred;
- a bare-tool deny rule — free with tool search active, invalidating when not; scoped rules (`Bash(rm *)`) and allow/ask rules never change the tool set;
- compaction (conversation layer only; project context survives only if CLAUDE.md and memory are unchanged); accumulating images/PDFs past the caps; upgrading Claude Code (new prompt or tool set — `DISABLE_AUTOUPDATER=1` is the documented
control).

**Not an invalidator, despite intuition:** editing `CLAUDE.md` mid-session. It is read once at session start and held in memory, so the edit neither invalidates the cache **nor applies** — it loads on the next `/clear`, `/compact` or
restart; nested `CLAUDE.md` and `paths:`-scoped rules load later on demand, so editing one before it loads does take effect. Permission-mode changes, output-style changes, skills/commands, `/recap`, `/rewind`, repo file edits and subagent
spawns are all cache-safe. (high: vendor page)

**Silent misses.** Nothing errors; cost and latency rise. The vendor's checklist: per-turn `cache_creation_input_tokens` vs `cache_read_input_tokens`; the `/usage` **`Prompt cache (main)`** line with hit ratio, miss count, warm state and a
"likely cause" for the last miss (e.g. `likely cause: tool definitions changed`; ≥2.1.251, likely cause ≥2.1.260); the statusline `prompt_cache` object; the OTel exporter per user/session. To settle **which TTL was billed**, run `claude -p
"hello" --output-format json` and read `usage.cache_creation` — one-hour writes report under `ephemeral_1h_input_tokens`, five-minute writes under `ephemeral_5m_input_tokens`.

**Gateways and enterprise surfaces — the most expensive failure.** Behind `ANTHROPIC_BASE_URL` or a gateway, one of three documented behaviours: (1) forwards markers unchanged → identical to the provider's own endpoint; (2) **rejects the
marked request with a 400 naming `cache_control`** → Claude Code re-sends with the marker moved onto the last conversation message and keeps it there until exit, so the block bills uncached but the conversation stays cached; (3) **removes
the markers and returns success** → the whole conversation bills uncached on every turn, and "a gateway that converts block-form system content to a plain string drops the marker the same way" — silent, catastrophic, visible only in usage
fields. Bedrock/Vertex/Foundry at their own endpoints cache the block as the Claude API does; and the 1-hour TTL partly travels in the `anthropic-beta` header, which a gateway must forward. (high: vendor page; header detail and the 2.1.307
`prompt-caching-scope` fix in the companion harness research, claude-deep §1)

**Reported, unconfirmed** (all low; none reproduced here). `#40652` (2026-03-28): the CLI find-and-replaces `cch=` billing hashes across *stored historical tool results* before each request, permanently changing prefix bytes when tool
output contained that hash — contradicts the append-only rule, and the mechanism is the reporter's theory; if real, it is exactly the "tool results get rewritten" failure. `#63930` (2026-05-29, 2.1.158 / Opus 4.8): parallel-tool turns and
`ToolSearch` materialisation collapsing `cache_read`; the reporter puts 74% of his `cache_creation` tokens (2.26M of 3.06M, ≈$42) down to it. No fix retrieved. `#89621` (2026-08-24, 2.1.245): a subagent with ~500–614K multimodal context
stopped hitting beyond the ~16K system prefix and rewrote ~500–600K tokens per request (~37.4M cache-write tokens in an afternoon), unexplained by the docs.

**Changelog evidence these classes are real:** maintainer entries (snippet-only, versions not captured) fix cache breaks on interactive resume of a `-p` conversation, on resuming a subagent, and on teammates re-announcing tools on turn two
— prefix breaks get changelog lines, so layer ordering is a tracked invariant.

## 6. Consequences for `warmpath`

**Profile fields already fit.** `HarnessProfile::claude` carries `provider = "claude"` (never inferred from the harness name), `CacheClass::Marker`, `InputShape::Exclusive`, usage paths including `usage.cache_creation_input_tokens`,
`assignable` via `--session-id`, `resume_argv = --resume {session}`, `warm_ttl = 300 s`. The provider field selects `cache::semantics("claude", model)` → marker mode, 300 s, read 1/10, write 1.25×, 1 h at 2×, model-dependent minimum
(Opus-class 4,096; Sonnet-4-class 1,024; Haiku 2,048). No new field needed: the claude-specific facts are provider facts the contract already owns.

**Cache identity maps 1:1 onto documented invalidators.** `CacheIdentity` = prefix + harness + provider + model + effort + session key, which is Claude Code's invalidator list seen from the harness side. One asymmetry deserves a code
comment rather than a change: effort is cache-safe on the 5.5-class models, so including it is *conservative* — it can only cost a missed warm claim, never create a false one.

**TTLs: three numbers, kept apart.** Contract default 5 min (`CacheTtl::Seconds(300)`, long option 3,600 s); deployment warm window 300 s; harness request bucket, 1 hour for a subscription main conversation. A
subscription-billed deployment resuming within 300 s leaves warm time unused; pinning `CLAUDE_CODE_PROMPT_CACHE_TTL=5m` is the alternative to raising the window. Never read the window as "the provider evicted" — `classify_cold` already
treats it as a policy step (`TtlExpired` is a decision, not a measurement). Subagents get 5 min even on a subscription.

**What not to build** — four strategies have no mechanism to attach to:

1. **An exact-request / response cache** — nothing in the CLI caches responses; a host-side one would break the `-p` result contract, the `usage` accounting every recorded number depends on, and the verifier's authority over what was
produced.
2. **A semantic cache of model output** — the provider's match is exact and the vendor design is "keep the bytes identical"; a near-match layer would claim unmeasured savings and could serve a stale answer into a code-modifying run.
3. **A tool-result / tool-function cache** — no layer exists to key against, tool results are conversation bytes by definition, and replaying one desynchronises transcript from prefix.
4. **Speculative subagent pre-warming, or a `--fork-session` warmer** — fork inheritance is documented for the subagent/session fork features, not the flag, and the vendor's own fan-out answer is a 5 s in-process hold; no documented surface
lets a dispatch plane pre-warm another process's prefix.

**What to keep, plus one addition.** Keep the one real lever: warm resume with identity and TTL as gates, realized-vs-expected reuse reported separately. Keep cold starts out of a prefix gate's denominator — for a
**marker-based** provider a cold start is not a miss. Keep the subagent caveat visible: `usage` excludes subagent tokens, so a run spawning subagents can report an
*over*-stated cached share. The addition worth a calibration run: read `usage.cache_creation.{ephemeral_5m,ephemeral_1h}_input_tokens` from one `-p` run and record the bucket beside the cached share, turning the 300 s window's assumed
bucket into a measured fact for that deployment.

## 7. Confidence and gaps

| Claim class | Confidence | Why |
| --- | --- | --- |
| Layer order, append-only assembly, per-bucket TTL, invalidators, subagent/fork behaviour, gateway handling, cache scope | **high** | one authoritative vendor page plus a vendor post stating the same rules prescriptively |
| Measured cached share and TTL corroboration here | **high / medium** | ledger + cost arithmetic exact to the cent; calibration n=6 |
| "No semantic / exact-request / tool / response cache" | **medium** | absence across every page and three issue reads; closed source → "none documented", not "none exists" |
| Breakpoint count and placement | **unknown** | not documented; `cache_control` absent from transcripts; the rolling-breakpoint theory is unconfirmed |
| Reported failure modes (`#63930`, `#89621`, `#40652`) | **low** | single reporters, pinned to 2.1.158 / 2.1.245 / March-2026 builds, no maintainer resolution, not reproduced |
| Changelog fix entries | **medium** | maintainer-authored, but snippets without version numbers |

Gaps, named: (1) **no live run today** — every number is second-hand from recorded runs; (2) the vendor page is undated and describes post-2.1.303 behaviour, so its `≥2.1.251`/`≥2.1.260`/ `≥2.1.290` gates may not hold on the
installed build; (3) three sources were snippet-only and are cited as leads, not evidence; (4) whether `#63930` is fixed by 2.1.303 is **not verified** — the retrieved issue page shows no state or maintainer reply; (5) the deployment's compressing
gateway was not exercised, so verbatim forwarding of `cache_control` and `anthropic-beta` is documented-required but unmeasured here; (6) `--fork-session` prefix behaviour is undocumented and untested.

## 8. Sources

- [How Claude Code uses prompt caching](https://code.claude.com/docs/en/prompt-caching) — Anthropic, undated live page (≥2.1.290-era), accessed 2026-10-28 — supports: layer order, per-bucket TTL, invalidator/kept lists, compaction and
`/rewind`, subagent vs fork, cache scope, gateway and Bedrock/Vertex/Foundry handling, usage fields and `/usage`.
- [Lessons from building Claude Code: Prompt caching is everything](https://claude.com/blog/lessons-from-building-claude-code-prompt-caching-is-everything) — Anthropic (Thariq Shihipar), page undated, a secondary source dates it 2026-04-30,
accessed 2026-10-28 — supports: static-first rationale, the three self-inflicted prefix breaks, messages-instead-of-prompt-edits, plan-mode-as-tools, `defer_loading`, cache-safe compaction fork.
- [anthropics/claude-code#63930 — Prompt cache fully re-created after turns with many parallel tool calls](https://github.com/anthropics/claude-code/issues/63930) — GitHub, reported 2026-05-29, 2.1.158 / Opus 4.8, accessed 2026-10-28 —
supports: the rolling-breakpoint hypothesis, transcript measurement method, 74% wasted cache writes (unconfirmed).
- [anthropics/claude-code#89621 — Cache misses beyond ~16K prefix in long-running subagent](https://github.com/anthropics/claude-code/issues/89621) — GitHub, 2026-08-24, 2.1.245, snippet only, accessed 2026-10-28 — supports: reported
subagent cache collapse with large multimodal context (unconfirmed).
- [anthropics/claude-code#40652 — CLI mutates historical tool results via `cch=` hash substitution](https://github.com/anthropics/claude-code/issues/40652) — GitHub, 2026-03-28, snippet only, accessed 2026-10-28 — supports: the allegation
that stored tool results are rewritten pre-request (unconfirmed; contradicts the append-only rule).
- [anthropics/claude-code CHANGELOG.md](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md) and its `feed.xml` mirror — GitHub, snippet only, accessed 2026-10-28 — supports: maintainer fixes for cache breaks on interactive
resume of a `-p` conversation, resumed subagents changing their tool list/system-prompt prefix, and teammates re-announcing tools on turn two.

**Local (2026-10-28):** [prompt-caching 01](../../prompt-caching/01-provider-mechanics.md), [02](../../prompt-caching/02-harness-practices.md), [04](../../prompt-caching/04-sources-and-confidence.md); the companion
harness research shipped with the `workcell` crate (`harness` README, claude-deep, claude-web);
`src/cache.rs` (`CacheContract`, `CacheIdentity`, `classify_cold`), `src/profile.rs`
(`HarnessProfile::claude`).
