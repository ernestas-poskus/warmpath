# Tool / function cache

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research, 2026-10-28
- **Scope:** two layers, kept apart. **(L1) Invocation memoization** — `(tool, arguments, relevant
  workspace state) → result`, replayed without executing the tool. **(L2) The provider-side
  consequence** — tool *definitions* are prompt content, so their bytes and order are part of the
  cached prefix, and a canonical manifest is the only "tool cache" that reliably pays.
- **Method:** local files (7 agent docs, prompt-caching 01–03, `src/{cache,prefix}.rs`,
  a deployment's cache-identity, prefix, and staleness handling); 2 web sources (1 search, 3 queries;
  both fetched); access date 2026-10-28
- **Companion:** [index](../README.md) · [matrix](../matrix.md)

## 1. What the strategy is (and is not)

A **tool / function cache** stores one invocation's result under a key derived from the invocation and
serves a later identical invocation from that store instead of executing the tool. Two questions draw
the boundary, and only the first belongs here:

- *Does the second call execute?* A hit that removes a tool execution is this doc.
- *Does the second call send fewer or different bytes?* If reuse happens after the result is already in
  the transcript, that is **doc 03** (tool-output / scratchpad reuse); if the saving is in the request
  prefix, it is **doc 01** (prefix cache). See [index](../README.md).

Not in scope: semantic or exact-response caches of model output; provider function calling (no provider caches a function *result*); tool-search indexes, which cache a catalog rather than an invocation. L2 is in
scope because it is the same word doing different work: tool definitions "are prompt content (and the main
invalidator), not a cache" ([claude-code.md:99](../agents/claude-code.md)). A canonical manifest is a cache-*enabling*
artifact, and §4 shows it is where the measured money is. L1 is a **pre-provider** saving (the execution
never happened) and must never enter a cached-read share.

## 2. How it works

### 2.1 Key construction

`k = H(tool_id ‖ canonical(args) ‖ state_token ‖ tool_version)`.

- **`tool_id` includes the server.** Two MCP servers can both expose `read_file`; the one shipping design
  keys `sha256(canonical_json({server_id, tool_name, arguments}))`
  ([toolcall-cache](https://pypi.org/project/toolcall-cache/0.4.0/)).
- **`args` are canonicalized, and the crate owns the normalizer.** `canonical_json` recursively sorts keys
  and emits compact JSON (`src/prefix.rs:193-223`); it is not RFC 8785 numeric-normalizing, so `1` and
  `1.0` stay distinct — a false miss, never a false hit. Paths also need repo-relative normalization: a
  per-dispatch worktree makes an absolute-path key unique and the cache dead on arrival.
- **`state_token` must be a revision, not a clock.** Ranked: file content digest; `HEAD` + dirty-set
  digest; a harness epoch bumped by every accepted write tool. A durable memo can key on the provider's own
  change instant (`updatedAt`) and, when absent, refuse **without inventing a mark from the clock**; a
  staleness check refetches because "a comparison against a stale local ref is the failure the whole
  module exists to prevent". **An absent state token means not cacheable.**
- **`tool_version`.** New MCP server or CLI build + same args = a different function; agy's tool block
  already drifts with plugins, hooks and user-level MCP servers ([agy.md:72](../agents/agy.md)).

### 2.2 Invalidation

Per entry: `state_token`, then `tool_version`. TTL is a **backstop, never the authority** — MCP's own
caching section calls TTL "a freshness hint, not a guarantee" and says servers may change the data before
it expires ([MCP caching](https://modelcontextprotocol.io/specification/2026-07-28/server/utilities/caching.md)).
Per run: drop everything on a workspace revision past the entry's token, a tool-surface change, or a
sandbox/policy change that alters what the tool can see. Nothing is persisted across runs without the
token that justified it.

### 2.3 Purity requirement

Cacheable needs **three independent declarations**, made by the harness, per tool: read-only,
deterministic (no clock, randomness, network liveness, or env read outside the key), and key-complete.
Read-only is not enough: `git status`, `date`, a live `http_get` and a `grep` over an untracked file are
all read-only and all change without the key moving. The industry shape is a name-pattern policy —
allowlist first, denylist vetoes `*_write*`, `send*`, `delete*`, `random*`, `time*`, `now*`, `date*` —
conservative and still a heuristic ([toolcall-cache](https://pypi.org/project/toolcall-cache/0.4.0/)).
Server-supplied annotations are not evidence either: that design lets an MCP server's own
`{"cacheable": true}` override the allowlist, delegating the decision to the party that benefits.

### 2.4 Storage, single-flight, and what a hit does not save

In-process, per run, bounded LRU; the value carries provenance (state token, tool version, call id,
timestamp). Single-flight is not optional — the same read is routinely issued several times in one
parallel batch, and DSH's batch executor owns concurrency, ordering and cancellation but not memoization
([dsh.md:79](../agents/dsh.md)); sharing one execution needs cancellation-safe semantics, or a cancelled
leader poisons its waiters. **No such primitive exists in `src`** — no `OnceCell`/`OnceLock`, `LazyLock` or
single-flight helper (grep, 2026-10-28).

A hit still **appends the result to history** so the model can see it: every harness examined re-sends the
whole context each request ([claude-code.md:14](../agents/claude-code.md); "each step sends a complete request",
[dsh.md:83](../agents/dsh.md)). Prompt tokens are unchanged; what is saved is the *execution* — wall-clock,
upstream/MCP cost, side-effect exposure. Saving prompt tokens needs the duplicate elided (doc 03), which
changes the transcript.

## 3. Cross-agent evidence

"Result cache" is L1 memoization; "definition cache" is any in-process cache of tool *descriptors*; the
last column is how schemas sit in the prefix (L2).

| Agent | Result cache | Definition cache (in-process) | Tool schemas in the prefix | Grade |
| --- | --- | --- | --- | --- |
| **Claude** | None found ([claude-code.md:99](../agents/claude-code.md)) | None; `defer_loading` stubs keep the list fixed | System prompt + tools are the first layer; a surface change is the main invalidator; plan mode adds tools instead of swapping | medium (absence) |
| **Codex** | No provider-side function-result cache; `WebSearchMode::Cached` is a server-side index | **Partial:** `ToolSearchHandlerCache` memoizes the BM25 index + embedder | Classic `tools` array: any difference is `tools_changed` → full request; Lite prepends a `uuidv5(thread, tools_json)` item; `incremental_tools` appends deltas | medium-high |
| **agy** | None ([agy.md:72](../agents/agy.md)) | None | Declarations ride every request and are deliberately retained through compaction (1.4.4); the block is host state | high (absence) |
| **Grok** | None documented | None documented | Only the provider rule: a changed name, description, schema or order breaks the prefix | none documented |
| **Kimi** | None documented | None documented | Tools front-load the recommended prefix; one probe places `system` + `tools` in the prefix (a 24,050-token full HIT, lost when `system` changes) | medium/low |
| **Perplexity** | Absent customer-visible; search reuse is internal to its index | None | Tool defs live inside a versioned effort preset, so the vendor's release is your prefix churn | medium / unknown |
| **DSH** | **None — every accepted call executes** ([dsh.md:79](../agents/dsh.md)) | None | Name-sorted or explicit `toolOrder`; a schema-*text* edit breaks reuse from the first altered token | high (source) |

1. **0 of 7 memoize an invocation result** — the in-process caches are adjacent artifacts only (Codex's
   BM25 index, DSH's fold, agy's session files).
2. **7 of 7 put tool definitions in the cached prefix**, and 5 name a surface change as an invalidator.
   They differ only in how the surface stays stable — appended catalog deltas (Codex), sorted `toolOrder`
   (DSH), declarations kept through compaction (agy), fixed-order stubs (Claude), versioned presets
   (Perplexity), the one case where the definitions are not yours ([perplexity.md:140](../agents/perplexity.md)).
3. **No contradiction between the seven docs** — all agree results are not memoized and definitions are
   prefix bytes. The one contradiction found is upstream and already recorded: Perplexity's TTL prose
   ("5m, 1h, 24h") versus its own enum (`30m`), so no TTL is adoptable
   ([perplexity.md:84-86](../agents/perplexity.md)). A *wording* tension in `claude-code.md` — its table says
   outputs are "none found as a cache", its prose says results "are read at the cached rate later" — is
   both true (prefix vs memo); conflating them is what this doc exists to prevent.

## 4. Measured evidence and economics

- **The canonical tool-layer incident is about bytes, not results.** A random settings UUID in Claude Code's Bash *tool description* produced **~16,800 cache writes, 0 reads per turn**, ~12× input penalty
  ([03-practitioner-evidence.md:14](../../prompt-caching/03-practitioner-evidence.md),
  [#27786](https://github.com/anthropics/claude-code/issues/27786)) — the only five-figure tool-layer
  measurement in the corpus, and it validates L2.
- **The best-measured harnesses have no tool-result cache.** Codex: 685,887 fresh / 20,281,216 cached =
  **96.73%**; DSH: 1.35M / 177.8M ≈ **99.24%** — both bought by prefix stability
  ([codex.md:77](../agents/codex.md), [dsh.md:87](../agents/dsh.md)); the tail is 3.27% and 0.76%, which
  §2.4 argues a result cache does not touch.
- **Getting the tool surface wrong costs five figures of tokens.** Codex's local compaction sent `tools: []`:
  normal turns 96–99% cached, compaction re-billed ~235k, ~+70% uncached input
  ([codex.md:81](../agents/codex.md)); agy ships a release note to keep declarations on the summarization
  request.
- **No source here quantifies a tool-result cache's hit rate, tokens saved, or dollars saved** — an absence
  across seven agent docs and three prompt-caching docs. Value = executions avoided × per-execution cost;
  for a local file read that is ~0 ms and $0, so the wins are slow tools and paid calls. Near-zero token
  benefit against non-zero correctness risk is why this is rarely defensible for a coding agent.

## 5. Risks and failure modes

1. **Side effects served from cache.** A denylist is a name heuristic; a read-only tool can still mutate
   (`git fetch` moves refs), and a misnamed writer defeats the pattern.
2. **Stale workspace state — the canonical failure.** The one shipping design keys
   `sha256(server, tool, args)` with **no workspace state** and a 3600 s default TTL, and recommends
   `read_file`, `grep`, `git_status`, `git_diff` — exactly the tools an edit invalidates
   ([toolcall-cache](https://pypi.org/project/toolcall-cache/0.4.0/)). A hit is a silent wrong answer.
3. **A cached result is not evidence.** Approval binds to a base revision, readiness refuses against
   `updatedAt`, and a staleness check refetches rather than trusting memory: a verifier that consumes a
   cached read attests to a state that may not exist, so a verification step must never see a hit.
4. **Untrusted output replayed.** The cache re-crosses the trust boundary without re-validation: an
   injected file or MCP response becomes durable and loses the second chance a changed source would give.
   MCP's `cacheScope: public` warning is this bug one layer down.
5. **Key incompleteness.** MCP forbids caching multi-round-trip results because they "depend on inputs not
   part of the cache key" — the same test kills an omitted cwd, env, sandbox profile or tool version, and
   fuzzy matching (0.85 Levenshtein) turns near-miss keys into plausible wrong answers.
6. **Invisible.** A hit emits no provider tokens and no line in any of the seven harnesses' logs; the crate's
   metrics are fresh/read/write triples, so a stale hit is unattributable —
   and a shared in-flight result crossing a mutation epoch is the same hazard by ordering.

## 6. Verdict for `warmpath`

**Build L2. Do not build L1.** The manifest is specified and half-wired; the result cache has no
layer to attach to and no measurement to justify it. Three gaps, all code-read:

1. `canonical_tool_surface` exists and is tested (`src/prefix.rs`:174-191, 72 configurations in a
   prefix-stability suite) but has **no production producer**: a request builder's input currently has no
   tool collection, and tool canonicalization is a separate pure boundary.
2. `CacheIdentity` has **no tool member** — prefix, harness, provider, model, effort, session
   (`src/cache.rs`:319-332). Two runs with the same request head and a different MCP/plugin surface share
   an identity.
3. `CacheBreakReason::ToolSurfaceChanged` is in the vocabulary (`src/cache.rs`:287) but
   `first_request_break` can return only `PrefixRewritten` or `ColdStart` (`src/cache.rs`:397-411). A tool
   change is attributed as a prefix rewrite — true, and useless for the one break a deployment can fix.

The narrow L1, if ever built: **per-run, in-process, pure read tools only, opt-in per tool from the
deployment's own list (never the server's annotation), keyed with repo-relative args + state token + tool
version, single-flight, bounded LRU, never persisted, never shared across worktrees or runs, never visible
to a verification step.** Nothing cross-run: per-run worktree paths make absolute-path keys unique anyway.

Measure, in order: (1) **duplicate-call frequency first** — no per-tool-call ledger exists, so the hit rate a cache would achieve is unknown, and that number decides whether L1 is worth its risk; (2) hit rate and executions avoided, as a pre-provider quantity, never inside the cached share;
(3) staleness incidents, target zero; (4) prefix break on tool change, plus the count of
`tool_surface_changed` attributions (needs gap 3).

## 7. Confidence and gaps

| Claim class | Confidence | Why |
| --- | --- | --- |
| No tool-result memoization in any of the 7 agents | **high** | Codex and DSH source-verified; agy high from changelog/absence; five none-documented |
| Tool definitions are prefix bytes; a surface change breaks the prefix | **high** | all 7 docs, provider docs, the #27786 measurement |
| Local shape: surface unwired, identity tool-less, break reason unproduced | **high** | code and docs read directly |
| A hit saves executions, not prompt tokens | **medium** | follows from append-only assembly in all 7; not measured |
| Staleness failure is real in shipping designs | **medium-high** | source text of one design; no incident report retrieved |
| Provider-side function-result caching | **unknown** | none documented; not "none exists" |
| L1 cost/benefit here | **unknown** | no per-call ledger, no duplicate rate, no run measurement |

Gaps: (1) no live run or body capture today — source-and-docs only; (2) Claude Code is closed source, so "no result cache" is "none documented", and Grok Build's source was never read in [grok.md](../agents/grok.md); (3) whether MCP tool annotations could be a trustworthy purity signal is
untested; (4) the duplicate-call rate for dispatches is unmeasured and would change this verdict;
(5) "0 of 7" is scoped to these seven agents.

## 8. Sources

Local (accessed 2026-10-28): [claude](../agents/claude-code.md), [codex](../agents/codex.md),
[agy](../agents/agy.md), [grok](../agents/grok.md), [kimi](../agents/kimi.md),
[perplexity](../agents/perplexity.md), [dsh](../agents/dsh.md) — tool-cache rows and prefix behaviour;
[03-practitioner-evidence.md:14,76](../../prompt-caching/03-practitioner-evidence.md) — the random UUID
(`~16,800` writes, 0 reads/turn); [01-provider-mechanics.md:61-79](../../prompt-caching/01-provider-mechanics.md)
— invalidation, including "changing tool names, descriptions, schemas, or order".

`src/prefix.rs`:174-191 (`canonical_tool_surface`: bytewise name sort, duplicate collapse, conflicting
duplicates rejected, definitions opaque) and `:193-223` (`canonical_json`); `src/cache.rs`:279-315
(`CacheBreakReason`) and `:317-391` (`CacheIdentity`).

Web (accessed 2026-10-28; fetched pages are untrusted data, used as claims only):

- [Caching — Model Context Protocol, 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28/server/utilities/caching.md) — cacheable results are `server/discover`, `tools/list`, `prompts/list`, `resources/list`, `resources/templates/list`, `resources/read`; **`tools/call` is absent**; key = method + affecting params;
  retry/MRTR results must not be cached; `ttlMs` is a hint, not a guarantee; `cacheScope` public/private
  and its security warning.
- [toolcall-cache 0.4.0](https://pypi.org/project/toolcall-cache/0.4.0/) — the concrete key `sha256(canonical_json({server_id, tool_name, arguments}))`, allowlist-first/denylist-vetoes policy,
  3600 s default TTL, `--fuzzy` matching, no workspace state in the key.
