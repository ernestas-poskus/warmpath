# Speculative / subagent execution caching

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research, 2026-10-28
- **Scope:** three mechanisms that do work before it is demanded and then reuse the prefix they warmed — (a) prefix pre-warming / a fan-out hold, (b) fork inheritance of a byte-identical prefix, (c) speculative execution of steps or tool calls. For each: what it buys, what it costs, which of the seven agents documents it, and whether a process-per-run dispatch plane can reach it. Provider mechanics are not restated — see [01](../../prompt-caching/01-provider-mechanics.md).
- **Method:** local files read (agent docs ×7, [02](../../prompt-caching/02-harness-practices.md), [03](../../prompt-caching/03-practitioner-evidence.md), `src/{warm,schedule,prefix,cache}.rs`, [warmpath DESIGN](../../../../docs/DESIGN.md), one deployment's sandbox, concurrency, warm-session, and batch documentation and its engine and scheduling source); 1 web search (2 queries), 2 fetches, 5 web sources; access date 2026-10-28.
- **Companion:** [index](../README.md) · [matrix](../matrix.md)

## 1. What the strategy is (and is not)

The three mechanisms share one asset: a provider **prefix** that some later request reads at the cached rate instead of writing at the fresh rate. They differ only in who produces the prefix and when the reader arrives.

| Mechanism | Reuse asset | What it buys | What it costs |
| --- | --- | --- | --- |
| (a) Pre-warm / fan-out hold | one prefix, written once, read by N−1 siblings | `(N−1) × prefix × (w − r)` per token; the second child alone recovers ~46% of the duplicated write (§4) | wall-clock hold; a serialization point inside a parallel batch |
| (b) Fork inheritance | the parent's whole completed-turn prefix | the entire head at read rate with no hold and no extra write | needs a harness fork surface; only valid while the child's bytes stay an append-extension |
| (c) Speculative step/tool execution | *nothing* — a pre-executed result is not a cache entry | latency only (PASTE: −43.5% completion time, 1.8× lower tool latency) | duplicated side effects and spend on every miss; a result only becomes prefix bytes once it is *sent* |

**Not speculative decoding.** Speculative decoding drafts and verifies tokens *inside one inference call*; it creates no request, warms no prefix, and has no interaction with any provider cache. Conflating the two is the common error: one is an inference-kernel optimisation, the other a scheduling decision about which request goes first.

Also excluded, for the same reason: semantic caching (a near-match store, [claude](../agents/claude-code.md) §Semantic caching finds none), keepalive pings (billable requests that hold a TTL, not work done early), and `prompt_cache_key` affinity (a routing hint that moves a request to a machine, not a prefix to a cache).

## 2. How it works

### 2.1 The barrier is three facts and an event

A pre-warm is correct only if all four hold:

1. **Same prefix bytes** — a digest over system+prompt, tool surface, and project context ([`prefix.rs`](../../../../src/prefix.rs) computes exactly this, length-delimited, and names the change).
2. **Same route** — same harness, provider, model, effort, speed class. A model switch is a different cache entirely ([01](../../prompt-caching/01-provider-mechanics.md):12–13).
3. **Same workspace** — the prefix is scoped to one machine and one directory; a fresh worktree path is a prefix change ([claude](../agents/claude-code.md):67–68), and codex puts `cwd` *inside* the cached contextual message ([codex](../agents/codex.md):93).
4. **An event: "the first request has started"** — on Anthropic a parallel request does not share a freshly written entry until the first response starts ([01](../../prompt-caching/01-provider-mechanics.md):25; the myth is named in [03](../../prompt-caching/03-practitioner-evidence.md):62). Releasing the group on *process spawn* does not establish this.

Fact 4 is where every implementation is a heuristic. Claude Code's own answer is a fixed delay: in a workflow fan-out of same-prefix agents it "holds all but the first for up to 5 seconds by default, so their first requests can read the prefix that the first agent cached" ([claude](../agents/claude-code.md):92, vendor page fetched 2026-10-28) — a delay, not a prediction, and not a lease.

### 2.2 Fork inheritance is not resume and not spawn

- **Spawn** — the child opens its own conversation with its own system prompt and tool set; its first request reads nothing of the parent's. Codex routes such a child to the parent's *affinity* (`prompt_cache_key = "{source}:{parent_thread_id}"`) while it presents a different prompt: shared key, not shared cache ([codex](../agents/codex.md):61).
- **Resume** — one lineage continues; it reads whatever survived the TTL and may rewrite the whole prefix. A resume *consumes* warmth, it does not create it ([claude](../agents/claude-code.md):91; `#40524` in [03](../../prompt-caching/03-practitioner-evidence.md):15).
- **Fork** — a sibling starts from byte-identical history. Claude: "a fork inherits the parent's system prompt, tools, and conversation history exactly, so its first request reads the parent's cache"; a `/fork` session copy appends its isolation instruction at the *end* so the copied prefix stays intact ([claude](../agents/claude-code.md):91–92). Codex: "full-history forks preserve the cached prompt prefix", asserted by a test that the child's input prefix equals the parent's ([codex](../agents/codex.md):62). DSH: the fork child may reuse the inherited byte-identical prefix, and the shipped fork tools deliberately expose **no child model selection** "so the copied history remains eligible for KV Cache reuse" ([dsh](../agents/dsh.md):70).

That last restriction is the design lesson: a fork stays a fork only while every child-only byte is appended *after* the inherited history. One child-scoped prompt section or one different tool schema and the "fork" is a spawn that paid to copy history.

### 2.3 Speculation earns latency, never tokens

The research line is real and measured: PASTE predicts concrete future tool invocations from recurring agent patterns and executes them while the model is still generating, "isolat[ing] speculative results until confirmed", for −43.5% average completion time and 1.8× lower tool latency across deep-research, coding and scientific-agent workloads ([PASTE, arXiv:2603.18897](https://arxiv.org/abs/2603.18897)); SPAgent reports up to 1.65× end-to-end speedup with speculative-action hit rates of 73.4% in the first step falling to ~11% in later steps; SpecHop formalises lossless speculation for multi-hop retrieval. Every one of these optimises *wall clock*. None reduces billed input tokens: a confirmed speculative result is appended and billed like any other tool result, and a rejected one is discarded work.

## 3. Cross-agent evidence

| Agent | (a) Barrier / pre-warm | (b) Fork inheritance | Child shares parent cache? | Evidence | Grade |
| --- | --- | --- | --- | --- | --- |
| [claude](../agents/claude-code.md) | **Yes** — workflow fan-out holds all but the first up to 5 s | **Yes** — fork inherits prompt/tools/history; `/fork` copy appends at the end | **No** for a subagent; **yes** for a fork; **no** for `--fork-session` (undocumented) | vendor page §Subagents (fetched 2026-10-28); claude-code.md:34, 89–95 | high (mechanism); none for speculation |
| [codex](../agents/codex.md) | **No** — WS warmup `generate:false` is connection setup, not a cache entry | **Yes** — full-history forks preserve the prefix, test-asserted | Key yes, prefix no (internal child); fork yes | `session_derived_cache_key`; `spawn.rs:120`; codex.md:27, 61–63 | high (source + tests) |
| [dsh](../agents/dsh.md) | **No** — "no warm-up barrier or staged fan-out exists"; children admitted 8-way and all request the same prefix at once | **Yes** — fork seeds the byte-identical completed-turn prefix; no child model selection | Fork yes; spawn no ("the child's request cache is independent") | fork/spawn READMEs; dsh.md:28, 70–75 | high (source) |
| [agy](../agents/agy.md) | **None** documented | **Partial** — `/fork` clones history into a new conversation id: identical at the fork point, then diverges | Unmeasured: **0** `cache_read_tokens` in every recorded run, so no hit has ever been observed | agy-deep §4/§8; changelog 1.1.8/1.4.4; agy.md:28, 66–68 | medium-high (mechanism), unknown (effect) |
| [kimi](../agents/kimi.md) | **None** documented | `/fork` is "an independent copy of the session"; warm fork "undocumented … a measurable question" | Undocumented | CLI sessions doc; kimi.md:39, 98–105 | unknown |
| [grok](../agents/grok.md) | **None** documented | Nothing documented; subagent sharing "unstated" | Unknown — explicitly "not 'none'" | no doc page describes assembly; grok.md:41, 98–104 | unknown |
| [perplexity](../agents/perplexity.md) | **None** | **Absent** — no subagent or branch primitive; `pplx_sdk` fan-out is Python *inside* one sandbox | Not applicable | presets/tools docs; perplexity.md:60, 109–113 | absent (high for the absence) |

Notes that change a decision:

- **One of seven documents a barrier, and it is the same vendor whose cache rule makes it necessary.** The other six either have no fan-out primitive or admit children blindly.
- **A shared `prompt_cache_key` is not a shared cache.** Codex is the trap: the key is inherited, the prefix is not ([codex](../agents/codex.md):61). A metric that reads key equality as warmth would report a hit that cannot exist.
- **DSH is the cleanest fork specimen** — byte-identical seed, child-only content appended, and a capability deliberately removed to protect the prefix. Its own honesty note applies to every row here: the harness "tests equality of its assembled request-head inputs, not a provider's cache behavior" ([dsh](../agents/dsh.md):75).
- **No agent exposes fork-on-disk.** Claude's, Codex's and DSH's fork mechanisms live *inside* a running harness (subagent/agent-control APIs). A dispatch plane that spawns one process per run can reach none of them. For that mode, (a) is the only available mechanism.

## 4. Measured evidence and economics

Multipliers from the crate's contracts ([`cache.rs`](../../../../src/cache.rs):214–250), with the provider facts in [01](../../prompt-caching/01-provider-mechanics.md):12–14:

| Route | read `r` | write `w` | long write | avoidable per token `w − r` |
| --- | ---: | ---: | ---: | ---: |
| claude | 0.10 | 1.25 | 2.00 (1 h) | 1.15 / 1.90 |
| codex (OpenAI) | 0.10 | 1.25 | 2.50 | 1.15 / 2.40 |
| dsh (DeepSeek) | 0.10 | 1.00 (a miss *is* the write) | — | 0.90 |
| agy (Gemini implicit) | 0.25 | unknown | — | unquantifiable |

`reuse_net_saving()` is exactly `w − r` ([`cache.rs`](../../../../src/cache.rs):137), and it is the same counterfactual the warm decision already reports as `cached × overlap × (w − r)` ([warmpath DESIGN](../../../../docs/DESIGN.md):56–61).

**Barrier arithmetic.** Without a hold, N same-prefix children each write: `N·w`. With a hold: `w + (N−1)·r`. Saving `(N−1)(w − r)` per token, which is positive for every N ≥ 2 and grows *linearly* in N−1 while the hold stays fixed:

| N | claude 5 m, cost in write-units | saving vs no hold | share of duplication avoided |
| ---: | ---: | ---: | ---: |
| 1 | 1.25 | 0 | 0% (a hold is pure latency loss) |
| 2 | 1.35 | 1.15 | 46% |
| 5 | 1.65 | 4.60 | 74% |
| 10 | 2.15 | 10.35 | 83% |

**Why that matters more than it looks.** Take the only measured fixed-prefix figure available — agy's first turn, 11,261 tokens ([agy](../agents/agy.md):82), the same order as claude's ~16 K system+tools head reported in `#89621` ([claude](../agents/claude-code.md):150). At claude list ($4/M in), one duplicate write costs ≈ $0.056, so four duplicate children ≈ $0.22 — comparable to the whole $0.2832 measured claude run ([claude](../agents/claude-code.md):111), because the run's *cached* reads bill at ~0.05–0.1× while a write bills at 1.25×. The duplicate head, not the body, is where a fan-out wastes money. Caveat: the first-request prefix of a per-run dispatch is **unmeasured**; the 11 K figure is borrowed from a different harness.

**When it does not pay.** One child (N = 1) — a hold buys nothing; a TTL that expires during the hold — the second request pays a fresh write anyway; a different tool surface or a different worktree — the group was never same-prefix, so the hold is a pure latency tax; a provider with an unknown write charge (agy/Gemini, grok, kimi, perplexity) — `strict()` claims no saving, and a barrier built there would be a bet on an unpublished number ([warmpath DESIGN](../../../../docs/DESIGN.md):41–45).

**The hold's cost is the crux, and it is a granularity question.** A *first-request* barrier holds the group for H seconds (claude: 5 s). A *completed-run* barrier holds it for one whole run D, because the only evidence such a barrier can read is a finished run — which is precisely what `WarmRecord` carries (`ended_at`, [`warm.rs`](../../../../src/warm.rs):220, 337). On a 30-minute run, H = 5 s is 0.3% of one run's wall clock; H = D doubles the batch's elapsed time — and D exceeds the TTL it hoped to exploit. A claude subagent sits in the "everything else" bucket at **5 minutes even on a subscription** ([claude](../agents/claude-code.md):89–90), so a completed-run barrier on any run longer than 5 minutes releases the group *after* the entry it warmed has expired: it pays the hold and still pays N writes. That alone rules the design out. **Any first-request barrier must therefore be specified as "the first request has started", not "the first run has finished"** — and a process-per-run dispatch plane has no live signal for either.

**Forks vs spawns for parallel verification.** A verifier that forks the implementation run reads the entire implementation prefix for free and appends its own check; N verifiers of one run therefore cost one write in total. N spawns each write the system+tools head — the barrier's case. Where a fork is documented (claude, codex, dsh) it strictly dominates; where only spawn exists (perplexity) or is undocumented (grok, kimi) the barrier is the fallback. Note that a claude dispatch that passes `--strict-mcp-config` with an empty MCP config, on the resumed form too, keeps a review child's tool surface equal to the implementation child's; if the review profile differs only in the sandbox grant (a read-only checkout), no prompt byte changes — a fork would stay prefix-identical.

**No local measurement exists.** Zero runs were made for this doc. A per-request cache ledger — one entry per provider request, `kind = first | append | reuse`, carrying the prefix digest and the served count — was available, but it is written from harness output after the fact and carries no timestamps, so "the first request started at T" is still not observable while a batch is running. Speculation's numbers are external and latency-only (PASTE 43.5% / 1.8×; SPAgent 1.65×, hit rate 73.4% → 11%).

## 5. Risks and failure modes

1. **Speculative work thrown away.** Hit rates collapse as a trajectory lengthens (SPAgent: 73.4% at step 1 → ~11% later). In a coding agent the waste is not just tokens: a speculative `git commit`, migration, or file write is a side effect that the sandbox does not retract.
2. **Cache-write storms.** The un-barriered fan-out is N writes where one would do, and the failure is invisible per run — the same shape as Continue caching the in-flight user turn, "paying a write that is never read" ([02](../../prompt-caching/02-harness-practices.md):17, 56), and as `#27786`'s random UUID in a tool description: ~16,800 writes and 0 reads per turn, ~12× input penalty ([03](../../prompt-caching/03-practitioner-evidence.md):14).
3. **Race on the first write.** Releasing on process spawn does not prove the entry exists ([01](../../prompt-caching/01-provider-mechanics.md):25). A 5 s hold is a delay chosen by a vendor, not a measured optimum; on a slow first token it is too short, and the group writes anyway — paying both the hold and the duplicate write.
4. **Fork children diverge.** One appended child-only system section, one different tool schema, or one model/effort override and the prefix is no longer identical — DSH removes model selection from its fork tools for exactly this reason ([dsh](../agents/dsh.md):70). Divergence is silent: the child simply pays a write.
5. **Non-determinism and per-worktree dispatch.** One dispatch model puts each work item in a fresh worktree, and both `cwd`-in-prefix (codex) and machine+directory cache scope (claude) make two worktrees two prefixes. A barrier keyed on "same plan/agent/route" is insufficient; it needs the workspace too — which `schedule_order` already keys on ([`schedule.rs`](../../../../src/schedule.rs):3) but a plan-level specification may not say.
6. **Resume mistaken for fork.** A resumed sibling is one lineage, not a branch; it can rewrite the prefix (`#40524`: 305,735 write tokens in one turn, [03](../../prompt-caching/03-practitioner-evidence.md):15). Preferring "resume" to "fork" to save the fork surface converts a read into a write.
7. **Budget-paced dispatch interference.** Pacing decides *when* a provider may start and how many slots to fill, and it may only lower concurrency. A barrier adds a second, unmodelled reason for a slot to sit idle, and the hold produces no run, no sample and no token — the pacer's EWMAs fold only completed successful runs, so the hold is invisible to the estimate while consuming window time. A barrier that starts a run the pacer would have held is a gate regression.
8. **Measurement hazards.** `cached > 0` is consistent with a much shorter persisted unit than the prefix ([dsh](../agents/dsh.md):121); a shared `prompt_cache_key` looks like sharing and is not ([codex](../agents/codex.md):61); a vendor "~5%" or ">90%" claim is not a measurement ([perplexity](../agents/perplexity.md):132, [grok](../agents/grok.md):123).

## 6. Verdict for `warmpath`

1. **Implement a first-request barrier, as start ordering, with a narrow specification.** Group by *prefix digest + route + workspace*, require N ≥ 2, and release the group on the harness's own first-request hold where one exists (claude: 5 s) or a small bounded timer otherwise. Never a completed-run barrier. A request path that blind-starts admits in input order when no warm state exists; per-tick admission means "at most one cold-group member per tick" is a filter over the order `schedule_order` already produces, testable against an existing schedule log, and measurable afterwards from the ledger's `first` entries, which record the prefix digest and the served count. The economics also say where: a planning batch over one locked context shares a real prefix — keeping the first task solo — whereas per-work-item worktrees make same-prefix dispatch the exception, so the barrier's value concentrates in planning and in same-item siblings (review, conflict repair, a re-verify), not across a multi-item batch.
2. **Prefer fork-style continuation wherever a harness documents it — and record that a process-per-run mode reaches none of them.** Claude (fork, `/fork` copy), Codex (full-history fork), DSH (fork with the byte-identical seed) all document it; all three are in-process. The crate's harness profiles carry `resume_argv` and no fork surface (`grep fork` over `src`: zero hits). So fork is a design option for a future harness-native dispatch mode, not a pending feature — and it must never be faked by pointing a second process at the parent's session id, which is a resume and can rewrite the prefix.
3. **Do not build generic LLM speculation.** The external results are latency wins with a predictor and an isolation layer; the binding constraint here is billing and window budget, not inter-token latency, and a speculative tool call in a code-modifying run has side effects the verifier cannot un-take. A speculative result also cannot be *reused* without desynchronising transcript from prefix — the exact thing that makes replaying tool results unsafe ([claude](../agents/claude-code.md):174).
4. **Keep the dispatch budget gate authoritative.** Apply the barrier after routing and *after* the tick's pace decision, so a `wait`/`reserve`/`concurrency` outcome wins and the barrier never starts a run pacing would hold, never holds a slot against `allowed_concurrency`, and never enters the pacer's samples. The barrier may delay a start; it may not admit one. Order: route → pace decision → group → hold → admit.
5. **warmpath gains nothing structural.** `schedule_order` (pure, deterministic, warm-first, `schedule.rs:81`) plus `PrefixDigest` plus `reuse_net_saving()` already express the whole rule; the missing signal is "first request started", which is a dispatch-plane event, not a provider fact. Adding a fork type or a barrier state machine to the crate would put a scheduling policy inside a library whose contract is provider accounting. One optional, pure helper is defensible — `expected_fanout_saving(prefix, n, contract)` returning `prefix × (n−1) × (w − r)` under the existing saturating, zero-floor arithmetic — and only if a second consumer appears.
6. **Label the cost.** A first-request barrier's primary risk is wall-clock latency. Report the hold as its own quantity beside cache writes per accepted outcome, so a deployment can tell a barrier that saved writes from one that only delayed starts.

## 7. Confidence and gaps

| Claim class | Confidence | Why |
| --- | --- | --- |
| The three mechanisms are distinct and only (a)/(b) touch a cache | **high** | provider rule + four independent harness docs agree |
| Claude's 5 s fan-out hold; subagent cold start; fork inherits prefix | **high** | vendor page fetched 2026-10-28, quoted verbatim; matches [claude](../agents/claude-code.md) |
| Codex fork preserves prefix; spawn shares key only | **high** | source + prefix-equality test at the pinned tag ([codex](../agents/codex.md):61–62) |
| DSH fork reuses the seed, spawn does not, no barrier exists | **high** | source-verified in the harness checkout ([dsh](../agents/dsh.md):70–75) |
| agy/kimi/grok fork warmth | **unknown** | undocumented; agy has recorded **zero** cache reads ever |
| Perplexity: no subagent primitive | **high** (absence) | no primitive in the Agent API docs |
| Multipliers and `w − r` | **high** | crate contracts + provider docs; one internal tension noted below |
| Barrier economics and break-even | **medium** | arithmetic on documented multipliers; the *prefix size* it multiplies is borrowed (agy's 11,261), not measured for a per-run dispatch |
| Speculation's latency figures | **medium** | primary abstract (PASTE) plus search snippets (SPAgent, SpecHop); no coding-agent deployment measured here |
| Any barrier working in this deployment | **untested** | zero runs; the per-request ledger carries no timestamps, so a hold cannot be reconstructed from recorded state |

Gaps, named: (1) **no live measurement** — not one fan-out was dispatched for this doc, and while the per-request ledger records *that* a first request happened it records no instant, so the barrier's H can only be a policy constant; (2) **prefix size unknown** for a per-run dispatch — the $0.056-per-child figure rests on agy's first turn and claude's reported ~16 K head, both other harnesses; (3) **the read multiplier is disputed in the measured data** — the claude contract says 0.10× while the measured Opus 5.5 invoice arithmetic closes at 0.05× ([claude](../agents/claude-code.md):111); this shifts the saving per token, never its sign or the N ≥ 2 break-even; (4) **`--fork-session` is undocumented and untested** — the vendor's fork statements concern the subagent/session features, not the flag ([claude](../agents/claude-code.md):95, 196), so a CLI-level fork warmer cannot be justified from the docs; (5) **the six agent docs are in draft state** — several carry literal elision markers where a heading or metadata line was compressed (e.g. [codex](../agents/codex.md):9, 21), so only their substantive prose was used; (6) **no vendor states whether a fork child's later turns re-write the inherited prefix** — the reuse claim is about the *first* request only, in every doc read.

## 8. Sources

**Local (2026-10-28).** Agent docs: [claude](../agents/claude-code.md):34, 89–95, 111, 150, 174–176, 196 · [codex](../agents/codex.md):27, 61–63, 93 · [dsh](../agents/dsh.md):28, 70–75, 121 · [agy](../agents/agy.md):28, 66–68, 82 · [kimi](../agents/kimi.md):39, 98–105 · [grok](../agents/grok.md):41, 98–104, 123 · [perplexity](../agents/perplexity.md):60, 109–113, 132. Research: [01](../../prompt-caching/01-provider-mechanics.md):12–14, 25, 86–94 · [02](../../prompt-caching/02-harness-practices.md):17, 56 · [03](../../prompt-caching/03-practitioner-evidence.md):14–15, 62. Code: `src/cache.rs`:137, 214–250 (`reuse_net_saving`, multipliers) · `src/warm.rs`:14–17, 220, 337, 520, 595 (`decide`, prefix gate, counterfactual) · `src/schedule.rs`:3, 81 (key is harness+workspace) · `src/prefix.rs`:1–6, 23 (digest, `PrefixChange`) · [DESIGN](../../../../docs/DESIGN.md):41–45, 56–61.

**Web (accessed 2026-10-28).**

- [How Claude Code uses prompt caching](https://code.claude.com/docs/en/prompt-caching) — Anthropic, undated (≥2.1.290-era), fetched — supports: subagent vs fork first-request behaviour, `/fork` session copy appending at the end, the 5 s workflow fan-out hold, cache scope to one machine and directory, per-bucket TTL.
- [Parallelizing Tool Execution and LLM Generation for Low-Latency Agent Serving (PASTE)](https://arxiv.org/abs/2603.18897) — Sui et al., submitted 2026-03-19, v3 2026-06-16, fetched — supports: −43.5% average task completion time, 1.8× lower tool latency, speculative results isolated until confirmed.
- [Speculate While You Reason: Teaching Agents to Predict Their Next Tool Call](https://arxiv.org/abs/2607.25816) — 2026-07-27, snippet only — supports: the speculator-agent gap, self-speculation; cited as a lead, not measured here.
- [Reducing Latency of LLM Search Agent via Speculation-based Co-Design](https://arxiv.org/abs/2511.20048) — snippet only — supports: up to 1.65× speedup; 73.4% first-step match falling to ~11% in later steps.
- [SpecHop: Continuous Speculation for Multi-Hop Retrieval Agents](https://neurips.cc/virtual/2026/loc/atlanta/poster/150381) — NeurIPS 2026 poster, snippet only — supports: lossless-speculation framing for multi-hop tool use.
