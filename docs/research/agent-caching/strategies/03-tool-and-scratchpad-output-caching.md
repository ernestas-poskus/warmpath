# Tool & scratchpad output caching

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research, 2026-10-28
- **Scope:** how a coding harness reuses **tool outputs and scratchpad content** — file reads, command
  output, repo maps, extracted symbols, summaries, artifact stores — and the mechanisms that *replace*
  them (compaction, pruning, truncation, dedupe). **Not** memoizing one function call (doc 06), and
  **not** the provider prefix cache (doc 01; mechanics in [provider
  mechanics](../../prompt-caching/01-provider-mechanics.md)).
- **Method:** local files (7 agent docs, prompt-caching research, `warmpath` source, one deployment's
  cost and ledger docs); `web_search` ×1 (2 queries), 2 fetches — 1 retrieved, 1 region-blocked;
  access date 2026-10-28.
- **Companion:** [index](../README.md) · [matrix](../matrix.md)

## 1. What the strategy is (and is not)

Four different things get called "caching tool output", at different layers with different owners, and
only the first is a cache in the provider sense.

| # | Mechanism | Where it lives | Owner | Reuses | Invariant it must keep |
| --- | --- | --- | --- | --- | --- |
| a | **In-prefix reuse** | the request itself | provider billing + harness assembly | the *prefix* those bytes became | sent bytes are never edited; results are appended |
| b | **Artifact / scratchpad store** | outside the prompt (files, SQLite, notes) | harness or dispatch plane | a *derived* object (summary, repo map, symbol index, document) | it may be stale: identity is checked before reuse, and the model is told it is a digest, not evidence |
| c | **Compaction / pruning** | a pass that *replaces* sent results | harness | prompt space | the replacement starts a **new series**; it is a declared break, logged with a reason |
| d | **Dedupe / truncation at record time** | before the bytes are ever sent | harness | nothing — it prevents bytes | deterministic: same input, same emitted bytes |

Only (a) is a cache in `warmpath`'s sense. (b) is a **store**, not a cache — nothing checks whether the
derived object still matches the repository. (c) is *anti*-caching: it buys window and quality with a
prefix. (d) is the one lever that is both cheap and cache-safe, because it never touches a sent byte.

The invariant everything below inherits is the one Goose adopted and measured: **bytes sent are never
moved or edited**; new turn context is appended as an immutable block
([02](../../prompt-caching/02-harness-practices.md):19).
Claude Code states the strong form of (a) directly — there is "no per-file or per-segment caching", so
the reuse unit is the request prefix ([claude](../agents/claude-code.md):14-16).

## 2. How it works

**(a) In-prefix.** A result is an ordinary history item: paid fresh once (or written at the covering
breakpoint), then read at the cached rate. A large result is a *prefix hazard*, not an asset — Continue
measured a large tool result going **62.3% → 99.4%** cached once turn-level caching covered it
([03](../../prompt-caching/03-practitioner-evidence.md):23). No harness
in the seven studied reuses a result through a separate model-visible store, memo, or dedupe layer
(agy's local stores are latency caches — §3).

**(b) Artifact / scratchpad.** A derived object is written outside the conversation and re-injected
later — or, better, referenced by identifier and read just-in-time. Anthropic's framing for the latter
is that lightweight identifiers plus `head`/`tail`/targeted queries "bypass the issues of stale indexing
and complex syntax trees", and that note-taking is memory "outside of the context window"
([Effective context engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)).
The crate already has the store: `ArtifactStore` holds opaque bytes under
`subject-<digest[..8]>[-variant].ext`, produced under an `ArtifactIdentity` (crate/contract/profile/
harness versions + input digest), with no-clobber publish and `Stale { reason }` instead of silent
reuse ([DESIGN](../../../../docs/DESIGN.md):90-97, [artifact.rs](../../../../src/artifact.rs):1-13,127-186).

**(c) Compaction / pruning.** A pass replaces history — or specific results — with a summary or a
head+marker+tail stub, so the request stops extending its predecessor. DSH is the cleanest case:
`dsh-compaction-tool-result-pruner` rewrites any result over `thresholdChars` (defaults 8192/4096/1024
code points) in **one deterministic pass, only after a compaction trigger qualifies**, with no model
call; the original stays in the log and the replacement is a logged surface replace under the same
`callId` ([dsh](../agents/dsh.md):58). **The log stays replay-correct while the request stops being
prefix-continuous** — two invariants, and DSH is the only harness here that keeps the first.

The cache-safe form of (c) is the *same-prefix side call*: repeat system prompt, tools and history,
append the instruction, summarize. DSH does exactly this — "keeping the conversation's own system
prompt, tools, and message prefix in front of it makes the auxiliary call a genuine prefix of the last
routed request" ([dsh](../agents/dsh.md):48). Codex's local compaction does the opposite (`tools: []`,
`parallel_tool_calls: false`) and cannot hit by construction ([codex](../agents/codex.md):91).

**(d) Truncation at record time.** Codex caps output in `record_items(…, truncation_policy)` and
`retained_tool_outputs` keeps marked outputs "whole or dropped" across compaction
([codex](../agents/codex.md):51). Bytes never sent ⇒ no prefix to break.

## 3. Cross-agent evidence

| Agent | What it does with tool output | Owner / mechanism | Prefix effect | Grade |
| --- | --- | --- | --- | --- |
| **Claude Code** | No tool cache. Three documented rewrites: image/PDF caps drop a *batch* of oldest attachments and reprocess "from the earliest of those messages onward"; compaction replaces history; `/rewind` truncates. Repo file edits append a `<system-reminder>` and re-read — not a rewrite ([claude-code](../agents/claude-code.md):70-78) | CLI (closed) | append cheap; compaction and attachment caps break | high (vendor); medium ("no tool cache": absence) |
| **Codex** | No result cache. Truncation at record time; outputs never edited in place; `retained_tool_outputs`; local compaction omits tools, remote v2 appends a `CompactionTrigger` and sends model-visible specs ([codex](../agents/codex.md):49-53,91) | `codex-rs/core`, tag `rust-v0.164.0` | truncation none; local compaction structural miss | high (source) |
| **DSH** | No result cache. Optional pruner: one deterministic head+`"middle pruned"`+tail pass at compaction pressure, logged surface replace, original retained; summarize call replays the prefix ([dsh](../agents/dsh.md):48,56-58) | `compaction-tool-result-pruner`, `compaction-basic` | pruning = `prefix_rewritten`; summarize = hit | high (source + in-tree cache test) |
| **agy** | No model-facing cache. Local caches buy latency only: conversation JSON, summary/trajectory SQLite, `/diff` finished-turn cache, changelog file cache. 1.4.7 inlines MCP results ≤10 KB instead of a file + follow-up read; 1.4.4 **retains tool declarations on the summarization request**; 1.4.9 drops unmatched tool calls/results ([agy](../agents/agy.md):38-44,53-57,100) | CLI + its own gateway | 1.4.4 is the only first-party cache-aware compaction claim in the set | medium-high (changelog); **0 hits in every recorded run** |
| **Kimi** | Results are ordinary tail messages; cached only as prefix on later turns. Nothing documents a store, dedupe, or scratchpad reuse ([kimi](../agents/kimi.md):37,84-89) | Moonshot server-side prefix cache | append cheap; compaction unmeasured | medium (prefix) / none (store) |
| **Grok** | Nothing documented at either layer. Only the negative rule: a changed tool block breaks the exact prefix, and a missing `reasoning_content` replay is the top cause of misses ([grok](../agents/grok.md):66-68,84-89) | xAI provider; harness unread | — | none documented / **unknown** (harness) |
| **Perplexity** | No harness of its own on this path; results enter the hosted loop and are re-sent, re-read through the upstream cache. `web_search`/`fetch_url` reuse is internal, not visible as cache reads ([perplexity](../agents/perplexity.md):58,96-100) | upstream provider | append cheap | medium (inference from documented shape) |

**Provider-level note.** One provider now exposes (c) as an API: Anthropic's context editing
(`clear_tool_uses_20250919`, `keep` default 3, `trigger` 100 k input tokens or a tool-use count,
`clear_at_least`, `exclude_tools`, `clear_tool_inputs`). Two details decide the design here:
`clear_at_least` exists explicitly to judge "whether clearing is worth breaking your prompt cache", and
a cleared message *remains part of history* and must be re-sent identically — rebuilding it from
current state is modifying an earlier message, and the prompt cache fails from that point
([context editing](https://platform.claude.com/docs/en/build-with-claude/context-editing),
[mid-conversation system
messages](https://platform.claude.com/docs/en/build-with-claude/mid-conversation-system-messages);
both region-blocked at fetch on 2026-10-28, snippet only — grade **medium**). The provider agrees with
DSH: clearing is a break to be *budgeted*, not a free saving.

**Contradiction found.** [codex](../agents/codex.md):91 reads the source at `rust-v0.164.0` and finds local
compaction still sending `tools: []`, #37305 open since 2026-08-05; the practitioner-status table calls
that bug "**Fixed upstream**" ([03](../../prompt-caching/03-practitioner-evidence.md):79); and the
harness-practices table lists "compaction re-injects base instructions/context" as *cache-safe* for
Codex ([02](../../prompt-caching/02-harness-practices.md):14). All three cannot hold for one build; the tag-pinned source read is the
strongest, so treat "fixed" as unverified for the installed CLI.

## 4. Measured evidence and economics

**Reusing a tool output mostly moves bytes; it does not make them cheaper.**

- **Re-sending a stored result later costs what it would have cost the first time.** A store that lets a
  harness "reuse" a file read wins only by avoiding a *new observation* (a fresh read at the fresh rate,
  plus a turn). Once the bytes are in the prompt, reuse buys nothing.
- **The genuine wins are three, none of them a tool-output cache:** fewer turns (avoided tool calls),
  fewer bytes emitted per turn (truncation/dedupe), and avoiding a rewrite (same-prefix side calls).
  Cursor's dynamic tool descriptions (−60% static-context tokens) and dynamic MCP tools (−46.9% in MCP
  sessions) are record-time wins, not reuse ([02](../../prompt-caching/02-harness-practices.md):27-30).
- **Pruning after send is usually net-negative.** Arithmetic over declared contracts (illustrative, not
  measured): removing a 2,000-token result from a 20,000-token retained tail at Anthropic 5m rates
  (read 0.1×, write 1.25×) saves `2,000 × 0.1 = 200` token-equivalents per remaining turn and costs
  `20,000 × 1.25 = 25,000` once → break-even ≈ **125 turns**; under DeepSeek's 1:1 write, ≈ 100. So a
  standalone mid-session prune almost never pays — it pays only when the break is *already happening*
  (compaction pressure) or the alternative is exceeding the window. That is exactly DSH's rule (pruner
  runs only after a compaction trigger qualifies, [dsh](../agents/dsh.md):58) and why "batch the bust" is the
  design, not an optimisation.
- **Truncation at record time is the only unambiguously positive lever:** removed tokens are never sent,
  so the saving is `removed × 1.0` on every later turn with no write. Codex's `truncation_policy` is the
  reference.
- **A summary that hides the evidence is not free.** Compaction preserves decisions and drops "redundant
  tool outputs" plus keeps "the five most recently accessed files"
  ([Effective context
  engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)); the
  cost is that a later verification step must re-run the tool. Measured numbers exist only for the cache
  side: Codex local compaction re-billed ~235 k tokens (~+70% uncached input), reaching 65–88% cached
  once tools were included ([#37305](https://github.com/openai/codex/issues/37305),
  [03](../../prompt-caching/03-practitioner-evidence.md):18); OpenHands' same-prefix condenser turned a 22,206-token fresh
  prompt into a mostly-cached read ([03](../../prompt-caching/03-practitioner-evidence.md):25). No public measurement of
  *quality* loss from pruning tool results was found — **unknown**, not zero.

Layers stay apart: a pre-provider stage removes tokens **never sent**; a cache read discounts tokens
**were sent**; adding them double-counts. A scratchpad store is a third thing — it changes
what will be sent, so it belongs in neither column, and is a decision about turns.

## 5. Risks and failure modes

- **Stale file content (high, structural).** A repo map, symbol index, or file summary is a snapshot; the tree moves under it. This is why Anthropic's JIT argument prefers identifiers and re-reads, and why Aider's repo-map-in-prefix is a hazard when it changes mid-prefix ([02](../../prompt-caching/02-harness-practices.md):15). The mitigation already here is `ArtifactIdentity` + `Stale { reason }`: mismatch is refused, never silently reused ([DESIGN](../../../../docs/DESIGN.md):90-97).
- **Untrusted output cached and replayed (high, security).** Tool output is attacker-influenced: file contents, web pages, command output. Caching multiplies exposure — one injected instruction is re-read every turn and survives into a resumed or forked session. `ArtifactStore` validates bytes as untrusted input on read; a model-visible store cannot, because the model consumes it as instructions.
- **Non-determinism (high).** Output depends on time, cwd, env, network, branch. A cache keyed on the *command* is a lie; only a content digest of the actual bytes is honest. A summarizer that returns different text for the same input destroys warm resume — the side-call guarantee is worth nothing if its output is not stable.
- **Verification authority (high).** A cached result is **not evidence**: it proves what a previous process saw, not what is true now. A verifier that accepts a replayed result has verified nothing — the same rule as "resume is not warm" ([01](../../prompt-caching/01-provider-mechanics.md):103-107).
- **Compaction is a declared break (high, policy).** Every replacement is recorded with a reason — `CacheBreakReason::Compaction` / `PrefixRewritten` ([cache.rs](../../../../src/cache.rs):281-315) — and excluded from eligible-expected-prefix rather than scored a miss, as cold starts and Codex's local compaction already are ([codex](../agents/codex.md):102). A silent rewrite is the failure the vocabulary exists to prevent.
- **Layer confusion in reports (medium).** An agy or Kimi "cache" that is a file cache, or a compressing proxy's token reduction counted as a cache read, produces a number that looks like reuse and is not. The fix is the declared layer, not a better ratio.

## 6. Verdict for `warmpath`

1. **Use the existing `ArtifactStore` for non-model artifacts and published documents — and only those.** Publication already goes through `warmpath`'s async store, whose validated name renders to `{subject}-{planhash8}[-variant].md` so a frozen prefix never moves. Frozen, digest-named, identity-checked is the right shape.
2. **Keep compaction and pruning as declared breaks with a reason.** `CacheBreakReason` already has `Compaction` and `PrefixRewritten`; pruning is not wired anywhere. If a pruner is added, copy DSH: one deterministic pass, batched at pressure, original retained, replacement logged with a reason.
3. **Do not build a cross-run model-visible tool-output cache.** No layer to key against, it desynchronises transcript from prefix, it replays untrusted bytes into a new trust domain, and §4 says the saving is at best a wash — the conclusion already recorded for Claude ([claude-code](../agents/claude-code.md):169-176) and the "none documented" rows for Kimi, Grok, Perplexity.
4. **Prefer record-time truncation to post-send pruning** wherever both are possible; it is the only lever with no write cost.
5. **Measure a pair per compaction event, not a ratio:** the **compaction request's** cache reads (a same-prefix side call should read high; a re-collected prompt reads ~0 and writes), the **next turn's** reads and writes (expected: ~0 reads, a write of the retained tail), removed tokens per pruning pass (so §4's break-even is checked against the real tail), and tool-output bytes as a share of the prefix — if that share is small, none of this matters and the doc should say so before anyone builds a store.
6. **Keep reports separate.** Scratchpad/artifact work belongs to neither AICS nor the pre-provider savings total; a document published once is a turnaround win, not a cache claim.

## 7. Confidence and gaps

| Claim | Grade | Why |
| --- | --- | --- |
| No harness in the set has a model-visible tool-result/scratchpad cache | **high** for Codex/DSH (source), **medium** for Claude (absence, closed), **none documented** for Kimi/Grok/Perplexity | six independent agent docs converge; two are tag-pinned source reads |
| Tool results are prefix content and their rewrite is a break | **high** | vendor docs, source (DSH, Codex), measured cases (Continue, Codex #37305) |
| Pruning after send is usually token-negative | **medium** | arithmetic over declared multipliers + DSH's batched design; no controlled measurement |
| Truncation at record time is the only free lever | **high** | no bytes sent ⇒ no prefix stake; Codex source |
| Scratchpad stores go stale and must be identity-checked | **high** | `ArtifactIdentity`/`Stale` in code; Aider repo-map and JIT-retrieval arguments |
| Provider-side tool-result clearing (Anthropic context editing) | **medium** | snippet-only; both pages region-blocked at fetch |
| Quality cost of pruning/summarising tool output | **unknown** | no measurement found; never report it as zero |

Gaps: (1) no live run today — every number is second-hand from one deployment's ledgers or the cited sources; (2) the Anthropic context-editing pages were snippet-only, so `clear_at_least` semantics and the exact cache-failure rule are quoted from search excerpts; (3) no public measurement of task quality after tool-output pruning was found; (4) Kimi, Grok and Perplexity have no harness source read here, so their rows mean "none documented", not "none exists"; (5) the §4 break-even is illustrative arithmetic on two contracts (Anthropic 5m, DeepSeek 1:1) and must be recomputed against a real tail before it drives policy.

## 8. Sources

Web (2026-10-28):

- [Effective context engineering for AI agents](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents) — Anthropic Applied AI, undated, fetched — supports: compaction keeping decisions and "the five most recently accessed files" while discarding redundant tool outputs; tool-result clearing as "one of the safest lightest touch forms of compaction"; note-taking outside the context window; JIT identifiers bypassing stale indexing.
- [Context editing](https://platform.claude.com/docs/en/build-with-claude/context-editing) and [mid-conversation system messages](https://platform.claude.com/docs/en/build-with-claude/mid-conversation-system-messages) — Anthropic, snippet only (region-blocked) — support: `clear_tool_uses_20250919` and its `keep`/`trigger`/`clear_at_least`/`exclude_tools` options; cleared messages must be re-sent identically or the cache fails from that point.
- [openai/codex #37305](https://github.com/openai/codex/issues/37305) — opened 2026-08-05, open at access — supports: local compaction omitting tools and its measured cost.
- [anthropics/claude-code #40652](https://github.com/anthropics/claude-code/issues/40652), [#63930](https://github.com/anthropics/claude-code/issues/63930) — via [claude](../agents/claude-code.md):148-151 — support: the unconfirmed allegation that stored tool results are rewritten pre-request; tool-loop cache collapses.

Local: the seven agent docs and the three prompt-caching docs (linked above); [DESIGN](../../../../docs/DESIGN.md):90-97, [artifact.rs](../../../../src/artifact.rs):1-13,127-186, [cache.rs](../../../../src/cache.rs):281-315.
