# Coding-agent caching strategies research

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research complete, 2026-10-28
- **Scope:** seven coding agents — `claude`, `codex`, `agy`, `grok`, `dsh`, `kimi`,
  `perplexity` — against seven caching strategies: prefix/prompt caching (context re-use),
  deterministic ordering, tool & scratchpad output caching, semantic caching, speculative /
  subagent execution caching, tool/function cache, and exact-request cache.
- **Decision this supports:** keep the structural investment where it is; build three cheap
  gates; do not build the strategies that have no safe mechanism for a code-modifying
  dispatch plane.
- **Strategy set:** the original strategy list had eight bullets with `prefix / prompt caching` twice; it is one
  strategy and is covered once in [01](strategies/01-prefix-prompt-caching.md). `tool & scratchpad
  output caching` ([03](strategies/03-tool-and-scratchpad-output-caching.md)) and `tool / function
  cache` ([06](strategies/06-tool-function-cache.md)) are kept separate because one reuses bytes in
  the prompt and the other removes a tool execution.
- **Method:** fourteen research subagents in two waves — one per agent (`agents/`), one per
  strategy (`strategies/`) — each local-first, then primary sources. No model call was made and
  no request was captured; every claim below is documentation- or source-level, graded.
- **Companion:** [prompt/context caching research](../prompt-caching/README.md) (provider
  mechanics, 2026-10-26) and the standalone [`warmpath`](../../../docs/DESIGN.md) crate that owns
  the accounting.

## Decision this research supports

1. **Only prefix caching is a cache in the money sense; deterministic ordering is its enabling
   condition.** Everything else on the list is a store with an invalidation problem (tool output,
   semantic, exact-response), an anti-cache (compaction/pruning buys context window by breaking
   the prefix), or a scheduling technique with side effects (speculation, pre-warming).
2. **Invest in the two structural levers and nothing else:** one append-only request assembly
   with canonical bytes, and declared per-route cache semantics with `unknown = strict`. Both are
   already implemented (`warmpath`; `CacheSemantics`; cache identity; AICS/EEPR). The remaining
   work is enforcement, not new mechanism.
3. **Build three cheap gates.** The prefix-invariance primitive and the crate half of the
   canonical-tool-manifest work shipped in `warmpath` 0.2.0 (2026-10-28); wiring them into a
   request path, and the fan-out barrier, are still proposed:
   - **Prefix invariance** — a cross-provider test: request *N* is a byte prefix of *N+1*
     under the declared cache mode; fixtures for timestamp, tool reorder, effort change, and
     compaction. Offline, no provider call. The crate now exposes `check_append_only` and
     `PrefixVariance`; a request path does not yet run them over recorded steps.
   - **Canonical tool manifest** — a *production* producer for the manifest, a tool-set digest
     in `CacheIdentity`, and `ToolSurfaceChanged` attribution. `CacheIdentity.tools` and the
     `ToolSurfaceChanged` attribution now exist in `warmpath` 0.2.0; the manifest producer is
     the remaining half.
   - **Fan-out barrier** — a warm-up barrier grouped by prefix digest + route + workspace with
     N ≥ 2, released by the harness's own first-request hold where one exists. A request path
     that admits in input order with no warm state is the case to replace.
4. **Do not build** a semantic response cache, an exact-request cache, a cross-run
   tool-output cache, generic LLM speculation, or a pre-warmed prefix belonging to another
   process. None of the seven agents ships any of these; each has a silent wrong-answer or
   stale-evidence failure mode that the verifier cannot see, and each would corrupt the
   usage-denominated accounting the cost gates depend on.
5. **Keep undocumented routes strict, but stop treating "strict" as the end state.** Kimi is now
   documented well enough for a provider family (automatic, 5 m/1 h, read 0.1×, write 1.0×/2.0×,
   minimum unpublished → `None`); xAI is documented on mechanism and read multiplier but not TTL
   or minimum; Perplexity's cache page contradicts itself and is in preview. Separately, two
   built-in profiles carry field defects and one deployment has one configuration gap, each
   of which would make a measured run misreport (below).

## Corpus

| File | Covers | Strongest evidence |
| --- | --- | --- |
| [agents/claude-code.md](agents/claude-code.md) | Claude Code (closed source; Anthropic API cache) | vendor page + one deployment's measured runs (99.998 % of input cached on one three-run batch) |
| [agents/codex.md](agents/codex.md) | Codex CLI 0.164.0 (open source, read at `rust-v0.164.0`) | request struct, ordering test, compaction builder |
| [agents/agy.md](agents/agy.md) | Antigravity CLI 1.4.8 (harness + gateway) | first-party changelog; measured runs predate the telemetry fix |
| [agents/grok.md](agents/grok.md) | xAI Grok (provider documented; Grok Build source unread) | xAI cache/pricing pages; harness = unknown |
| [agents/dsh.md](agents/dsh.md) | DeepSeek Harness (source at the DSH checkout) | `file:line` request assembly, compaction, fork, usage parsing |
| [agents/kimi.md](agents/kimi.md) | Kimi Code CLI 0.36.0 + Moonshot API | Moonshot caching/pricing guide; CLI assembly undocumented |
| [agents/perplexity.md](agents/perplexity.md) | Perplexity APIs (not a repo-editing harness) | gateway schema, presets; no Agent-API usage capture |
| [strategies/01-prefix-prompt-caching.md](strategies/01-prefix-prompt-caching.md) | the cache unit, economics, cross-agent mechanisms | provider docs + agent docs |
| [strategies/02-deterministic-ordering.md](strategies/02-deterministic-ordering.md) | canonical byte order as the enabling condition | codex ordering test, DSH ordering code, agy changelog |
| [strategies/03-tool-and-scratchpad-output-caching.md](strategies/03-tool-and-scratchpad-output-caching.md) | tool results, scratchpads, artifacts, pruning | DSH pruner/summarizer, Codex truncation, Anthropic context editing |
| [strategies/04-semantic-caching.md](strategies/04-semantic-caching.md) | near-match response caching and why not | GPTCache-class work; missing from all seven agents |
| [strategies/05-speculative-subagent-execution-caching.md](strategies/05-speculative-subagent-execution-caching.md) | fan-out barrier, fork inheritance, speculation | claude 5 s hold, codex child key, DSH fork seed |
| [strategies/06-tool-function-cache.md](strategies/06-tool-function-cache.md) | invocation memoization + tool-definition caching | codex tool-search memo, canonical manifest gaps |
| [strategies/07-exact-request-cache.md](strategies/07-exact-request-cache.md) | byte-identical replay and its accounting damage | DSH/Codex request paths, provider feature absence |

## Findings that matter

1. **The provider is not where the variance is.** The same provider through different harnesses
   produces different reuse: Anthropic's cache is marker-based and Claude Code places the markers
   (and holds a fan-out), while an Anthropic model behind a gateway may never receive a marker at
   all; Gemini's implicit cache is automatic, but agy documents no pass-through surface. A
   "provider cache facts" table alone predicts neither hit rate nor contract — the harness layer
   is the second half of the contract, which is why `HarnessProfile` separates `provider` from
   `harness`.
2. **Every harness that caches well does the same three things:** append-only assembly with a
   byte-stable head, compaction as a same-prefix side call, and a session-scoped identity that
   pins model/effort/tool surface. Codex states it as an ordering test; DSH states it as
   "prefix-cache stability is corollary #1"; Claude Code states it as "static content first,
   dynamic content last"; agy ships ordered rules/paths since 1.1.6 for exactly this reason.
3. **The failure mode is always silent and always before the first byte a provider sees:** a
   timestamp, a per-thread id, a reordered tool list, a compaction that omits tools, a resume that
   rewrites the system layer. The measured cases in the sibling research (tool-description UUID:
   ~16,800 writes / 0 reads per turn; Codex explicit breakpoint: 0 % → 98.6 %) are all of this
   class. The countermeasure is structural (canonical bytes at one assembly point + an invariance
   test), not operational vigilance.
4. **Cross-agent reuse is fork-shaped, not cache-shaped.** The only documented way one agent's
   work warms another is a full-history fork that keeps the head byte-identical (Claude Code,
   Codex, DSH), or a short same-prefix hold on a fan-out (Claude Code's 5 s). A process-per-run
   dispatch model with only `resume` support has neither today; a fan-out barrier is the only
   version it can implement without a harness-native fork mode, and it must never be faked by
   pointing a second process at the parent's session id.
5. **Compaction is where the accounting and the cache disagree.** A compaction request that
   replays the prefix is cheap; the turn after it rewrites the retained tail and reads almost
   nothing. Recording that as "a miss" misattributes a deliberate policy as a provider failure;
   recording it as "warm" overstates reuse. The report has to be a pair (compaction request vs
   next turn), which is why `CacheBreakReason::Compaction` exists and why cold starts and
   compaction turns are excluded from EEPR rather than zeroed.

## Corrections and contradictions this pass found

| Prior claim | New evidence (2026-10-28) | Disposition |
| --- | --- | --- |
| Codex local compaction omitting tools was "fixed upstream" ([prompt-caching/03](../prompt-caching/03-practitioner-evidence.md)) | Still `tools: []`, `parallel_tool_calls: false` at `rust-v0.164.0` (`compact.rs`); #37305 open | Status corrected in [prompt-caching/03](../prompt-caching/03-practitioner-evidence.md); keep compaction turns out of expected-prefix |
| agy's recorded 0 % cache hit shows agy does not cache | The cached-token path is fixed in `HarnessProfile::agy`; every zero row predates it, and the calibration rows carry synthetic ids and 2024 epochs | Not evidence; re-measure on 1.4.8 before ranking agy |
| Grok's cache share is 96.73 % | That number is a borrowed Codex shape from a local cost model; xAI publish no cache-share figure | Treat as unmeasured; the vendor ">90 %" claim is marketing over unnamed workloads |
| Codex compaction re-injects base instructions/context, so it is cache-safe ([prompt-caching/02](../prompt-caching/02-harness-practices.md)) | True only on the remote v2 path (appends a `CompactionTrigger`, sends `model_visible_specs()`); Codex's local path sends `tools: []` and cannot hit at 0.164.0. DSH's summarize call is the same-prefix design done properly (`summarizer.ts`) | Report a compaction as a declared break and as a pair (request vs next turn); exclude local-compaction turns from EEPR |
| Anthropic caching is explicit-`cache_control`-only; the client always places the breakpoint | The vendor now documents a top-level automatic `cache_control` mode where the API places and advances the breakpoint (medium: region-gated page, snippets only) | Record as an open question, not a contract change; `CacheMode::Marker` still describes the CLI path |
| OpenAI's long cache TTL is 12 h (encoded as `long_ttl = 43,200 s` in `warmpath`) | The current API-reference snippet says `ttl` defaults to 30 m and is "currently the only supported value" (medium) | Verify before quoting a 12 h saving; if confirmed, a `SEMANTICS_VERSION` bump drops the long option |
| Perplexity is a coding agent to compare with Claude Code / Codex | It is an API and a hosted agent loop with no filesystem/diff primitive; its caching lives on a preview Orchestrator API in the OpenAI dialect | Record the surface mismatch; do not add a Perplexity family |

**Naming hazard found in this pass.** The Claude Code doc was first written as `agents/claude.md`.
On a case-insensitive filesystem (macOS APFS), the harness's instruction-file discovery resolved it
as `agents/CLAUDE.md` and loaded the whole research doc as agent instructions; it was renamed to
`agents/claude-code.md`. Any future file named `claude.md` in a directory an agent works in will be
auto-loaded the same way — name research artifacts so they cannot collide with `CLAUDE.md`,
`AGENTS.md`, or `GEMINI.md`.

Two profile defects and one configuration gap were found while checking the claims against the
crate. They are recorded, not fixed, because a research task must not change a public compatibility
surface:

- `src/profile.rs` `grok()` declares `usage.cached_input_tokens` /
  `usage.cache_creation_tokens`, but xAI documents nested
  `usage.prompt_tokens_details.cached_tokens` and no write field, and cached tokens are a subset of
  prompt tokens (`InputShape::Inclusive`, not `Exclusive`).
- `agy()` reads `usage.reasoning_tokens` where agy emits `usage.thinking_tokens`; `resume_argv`
  is `--resume` while agy's flag is `--conversation` (one deployment overrides it, a library consumer
  would not); `InputShape::Exclusive` is unverified against Gemini's inclusive reporting.
- One deployment's profile loader exposes a per-agent override for `cache_read` but not
  `cache_write`, so a Kimi-style provider whose writes are billed separately cannot be pointed at
  its write field; write spend is invisible to `cache_pct` and `cache-gate`.

## Confidence and gaps

- **High:** the seven agent docs' source-level facts (Codex request struct and ordering test, DSH
  `file:line` assembly, agy changelog statements, Moonshot/xAI/Anthropic/OpenAI provider pages).
- **Medium:** harness behaviour where the vendor publishes prose but no source (Claude Code, Kimi
  CLI assembly, agy's request body), and all cross-agent comparisons built from those claims.
- **Low/unknown:** whether any of the "none documented" absences is actually "none exists"; Grok
  Build's request assembly (source unread); Perplexity's Agent API usage object (reference not
  fetched); every provider TTL/eviction where no number is published.
- **Irreducible gap for this pass:** no live model call and no wire capture. The cheapest
  resolution for each gap is written into the agent and strategy docs rather than left implicit.

## Regenerating

Each agent doc names the files, tags, and versions it was built from: `claude` 2.1.311, `codex`
0.164.0 / `rust-v0.164.0`, `agy` 1.4.8 (bundled changelog head 1.4.9), `kimi` 0.36.0, DSH
checkout at commit `22ddee3`. Refresh an agent doc when its harness ships a release that touches
request assembly, compaction, or usage parsing; refresh a strategy doc when a new harness claims
a mechanism. A change to a provider's documented numbers is a `warmpath` `SEMANTICS_VERSION` bump,
not a doc edit alone.

## Follow-up (2026-10-28)

The findings' capability matrix is encoded in this crate: `topology::capabilities(harness)`
(`TOPOLOGY_VERSION` 1) is the versioned table, `NodeKind`/`PrefixInheritance`/`ChildUsage` keep the
layers apart, and `CohortKey` names the identity facts a sibling fan-out groups on. A change to an
existing harness row is a `TOPOLOGY_VERSION` bump, like the provider table. The product-side record
that consumes it is `wirepuller`'s run cache record (ADR 0038 in the landline repository); the
crate itself stays domain-neutral and only describes the harness.
