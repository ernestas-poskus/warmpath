# Deterministic ordering

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research, 2026-10-28
- **Scope:** the ordering half of a cache-safe request — what makes one request a canonical byte
  sequence, why an exact byte prefix is the only thing every provider's cache matches, how seven
  harnesses establish it or do not, and what a dispatch plane must enforce before the first request is
  sent. Breakpoints, compaction, warm selection and fan-out appear only where they move bytes.
- **Method:** local files (seven [agent docs](../agents/claude-code.md); four
  [prompt-caching docs](../../prompt-caching/README.md); [warmpath DESIGN](../../../../docs/DESIGN.md)
  and `src/{prefix,cache,prompt}.rs`; one deployment's request, prefix, and prefix-log modules);
  5 web sources (2 fetched, 3 snippet-only); access date 2026-10-28.
- **Companion:** [index](../README.md) · [matrix](../matrix.md)

## 1. What the strategy is (and is not)

**Deterministic ordering is the property that one request's bytes are a pure function of declared
inputs.** Five clauses: **totality** (every byte from a named input — no ambient clock, RNG,
environment, filesystem order or host identity); **purity** (same inputs, same bytes, another process
or host or day); **layer order** (stable first, volatile last: harness system prompt and tool definitions
→ project context → task payload → history → per-turn tail); **canonical serialization** (one
representation per value — recursive key order, one whitespace convention, tools sorted by a fixed
comparator, no locale collation); **identity** (a digest over *the bytes actually sent*). Clause 5 makes
the property checkable, clause 3 makes it cheap.

| Adjacent strategy | Adds | Why it is not this |
| --- | --- | --- |
| Append-only assembly | history only ever extends | Says nothing about the *first* request, where layer order and serialization are decided — and a renderer that puts a per-attempt budget block *before* the plan does exactly that (§6). |
| Route / tool pinning | the same identity is reused | Pinning decides whether to continue; ordering decides what the bytes are. A pinned route with unsorted tools still misses. |
| Semantic caching | a lookup keyed on meaning | Every provider matches bytes; "same content, different order" is a full miss. |

**The strong claim:** ordering is the *enabling* condition for prefix caching, not a practice beside it.
Reuse is `bytes[0..k]` equality; every clause above is a way of not perturbing `k`.

## 2. How it works

A provider's cache matches an exact prefix: Anthropic walks back from `cache_control` with a 20-block
lookback, OpenAI and DeepSeek match automatic prefix units, Gemini an implicit prefix
([01](../../prompt-caching/01-provider-mechanics.md):10-15). The reusable region ends at the first
differing byte, so the cost of a variance is not "some overhead" but **the whole suffix after the
divergence point** — for a layer-0 change, the conversation.

A dispatch plane controls L1 (project context), L2 (task payload) and L4 (per-attempt tail); L0
(harness system prompt and tool definitions) and L3 (history) belong to the CLI — the largest layer is
outside the caller's assembly, so the levers are not letting the caller's layers drift ahead of it, and
pinning what makes the harness render L0 differently: workspace, MCP config, model, effort.

**Serialization rules that matter**, each a regression class in §5: sort object keys recursively and tool
definitions by name, refusing duplicates with different bodies
([`canonical_tool_surface`](../../../../src/prefix.rs):174-191); emit no insignificant
whitespace ([`canonical_text`](../../../../src/prefix.rs):225-246); hash with **length
delimiters** so `["ab","c"] ≠ ["a","bc"]` ([`digest`](../../../../src/prefix.rs):70-85); never
sort with a locale collator, never trust map or directory iteration order; **canonicalize at assembly,
not at hash time** — if the digest is over canonical bytes while the wire carries raw ones, the identity
lies. Canonicalization has to happen at one assembly point: the renderer should start from the canonical
string rather than re-derive it from the raw inputs.

**Identity** is prefix + harness + provider + model + effort + session
([cache.rs](../../../../src/cache.rs):319-391); it carries neither the tool manifest nor
anything after the static head. RFC 8785 canonicalises keys and numbers
([RFC 8785](https://www.rfc-editor.org/rfc/rfc8785.html), fetched 2026-10-28); our `canonical_json` sorts
UTF-8 keys and leaves numbers alone ([prefix.rs](../../../../src/prefix.rs):193-223) —
identical for ASCII, divergent for non-ASCII, so it is self-consistent but is not "RFC 8785".

## 3. Cross-agent evidence

| Agent | Ordering mechanism, with evidence | Grade | Residual variance it does not control |
| --- | --- | --- | --- |
| [claude](../agents/claude-code.md):54-68 | Documented layer order: system prompt (instructions + tools) → project context → conversation; the vendor blog states the rule prescriptively ("static content first, dynamic content last") and names three self-inflicted breaks — a timestamp in the static prompt, non-deterministic tool shuffling, editing the Agent tool's parameters. Five stabilisers: prompt-snapshot replay, `--exclude-dynamic-system-prompt-sections` (cwd/env/memory paths/git status into the first user message), plan/skill/plugin text as *messages*, plan mode as tools, `defer_loading` stubs in fixed order | **high** | cwd/platform/shell/OS in the opening; git-status snapshot; auto-memory paths; a per-session console URL inside a tool description ([#82229](https://github.com/anthropics/claude-code/issues/82229)); breakpoint count/placement **unknown** |
| [codex](../agents/codex.md):43-47 | Front fixed and asserted by test: `[0]` base instructions, `[1]` permissions, `[2]` bundled context (including `cwd`), `[3]` user turn; tools a top-level array (or a prepended `AdditionalTools` item). Equivalence is structural — `tools_changed`, `input_shortened`, `input_mismatch` are named non-incremental reasons ([client.rs](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/src/client.rs#L347-L465)) | **high** | Item ids are `uuidv5` from `thread_id`: identical bytes in two fresh threads are not identical requests ([#21796](https://github.com/openai/codex/issues/21796)); `cwd` sits inside the cached contextual message |
| [agy](../agents/agy.md):38-51 | Deliberate and first-party: 1.1.6 "sorting rules and discovered paths deterministically, preventing unstable prompt ordering and needless prompt-cache misses"; 1.4.4 keeps tool declarations on the summarisation request so the prefix is reused. Order is fixed by the build, not per run | **high** (that it is deliberate) | Membership drifts: cumulative rules (workspace + `~/.gemini` + plugin), host MCP/plugin surface, the 20 k-token demotion budget. Ordering is deterministic; the input set is host state |
| [dsh](../agents/dsh.md):50-54 | Enforced by construction: prompt sections sort by ascending `order` then code-unit `name`; the final request order sorts by `order`; tool schemas sort by name or an explicit `toolOrder` with a name-sorted remainder. Header equality is a structural comparison of the canonicalised envelope, so "no header event" proves the head did not move | **high** | Surface replacement: node-0 rewrite, compaction, tool-result pruning, image offload all rewrite earlier bytes (logged, with a reason) |
| [kimi](../agents/kimi.md):36,77-82 | **Advice only.** The provider guide says front-load system prompt, tool definitions and reference material and tail-load turns and tool results; no vendor statement of an append-only invariant, single assembly point, or "old bytes never rewritten". A third-party deep-dive describes system → injections → history with appends at segment end — secondary, low | **medium** (advice) / **low** (harness) | Everything: assembly undocumented; volatile front bytes are warned against, not shown prevented |
| [grok](../agents/grok.md):38,72-82 | Provider advice only, and generic: front-load static content, never edit/remove/reorder earlier messages, append only. **What the CLI does is `unknown`** — layer order, whether per-turn state sits before or after the stable prefix, whether compaction rewrites in place | **medium** (provider) / **unknown** (harness) | The whole harness half. A session id surviving `--resume` is an inference, not evidence, for `x-grok-conv-id` |
| [perplexity](../agents/perplexity.md):57,88-94 | An effort level bundles model, search config, reasoning steps, system prompt and tools, so the head is Perplexity's bytes in Perplexity's order; a tool's position in the serialised array is **undocumented**; caller `instructions` then `input` follow — stable in examples, inferred | **medium** (head per version) / **unknown** (order) | Order, whitespace, and any server-side assembly |

Three readings. (1) Ordering is a named concern only where the harness owns its request path — Codex
asserts it in a test, DSH enforces it in a function, agy ships a changelog line. (2) The three `unknown`
cells share one shape — a closed or unread client — which is why §5 states hazards as classes, not
per-agent bugs. (3) **Ordering is necessary, not sufficient:** Claude Code and Codex both order correctly
and still fail cross-session reuse, because a per-session identifier lives *inside* an early layer
([#82229](https://github.com/anthropics/claude-code/issues/82229),
[#21796](https://github.com/openai/codex/issues/21796)).

**Contradiction found in the local docs.** On Codex local compaction,
[03](../../prompt-caching/03-practitioner-evidence.md):79 lists "local compaction omitting tools" as
**fixed upstream**, and [02](../../prompt-caching/02-harness-practices.md):14 calls "compaction re-injects
base instructions/context" cache-safe, while the tag-pinned source read in
[codex.md](../agents/codex.md):91 records it as "still present at 0.164.0; #37305 open since 2026-08-05".
The agent doc is the most specific and most recent, so it wins. Softer: 02 treats Claude Code compaction as
cache-safe because the summarisation request repeats the prefix, while
[claude-code.md](../agents/claude-code.md):73 counts compaction as a conversation-layer rewrite — both true of
different requests (side call vs rebuilt history), which the table's compression loses.

## 4. Measured evidence and economics

For a request of `L` tokens whose prefix matches for `p`, cost in units of fresh input is `p·r + (L − p)`,
where `r` is the read multiplier: 0.1 for Anthropic, OpenAI, DeepSeek and Kimi, 0.25 for Grok 4.7
([grok](../agents/grok.md):48-52). So `p = L` costs `0.1L` and `p = 0` costs `L` — a layer-0 change is a
**4×–10×** multiplier, and on the measured Opus run (744,078 cached at $0.20/M against $4/M fresh) the
same tokens are **20×** dearer as fresh input. The multiplier
is step-shaped in `p`: which layer moves decides the bill.

| Public regression | Change | Measured effect | Grade |
| --- | --- | --- | --- |
| [#27786](https://github.com/anthropics/claude-code/issues/27786) | random UUID in the Bash tool description, per `query()` | ~16,800 writes, 0 reads per turn; ~12× input penalty | **high** (matches [03](../../prompt-caching/03-practitioner-evidence.md):14; snippet only here) |
| [#82229](https://github.com/anthropics/claude-code/issues/82229) | per-session console URL in a tool description, CC 2.1.220, verified by diffing captured `/v1/messages` bodies | no cross-session warm start; every session writes its ~20 k-token head | **medium** (one reporter, method stated) |
| HN 47754795 via [03](../../prompt-caching/03-practitioner-evidence.md):16 | git-status snapshot; an empty commit | warm 18 k read / 0 write → 11 k read / 6 k write | **medium-high** |
| Goose [#11019](../../prompt-caching/03-practitioner-evidence.md):24 | volatile turn context moved out of the prefix | reuse 77% → 91%, input cost −42% | **high** (vendor before/after) |
| [#21796](https://github.com/openai/codex/issues/21796) | identical 11,749-token prefix, key from thread id | only ~55% cached | **medium** |
| [#35300](https://github.com/openai/codex/issues/35300) | explicit breakpoint on a stable prefix | 0% → 98.6% hits | **high** — a boundary fix, not ordering; it prices what a stable prefix was worth |

One deployment's own runs are the strongest evidence that ordering already holds: 93.05% of input
cached over the seven-run claude batch, 96.73% over the Codex batch, 99.24% over the DSH session. Those
shares are *within-run, turn-to-turn prefix growth* — appended history with an unchanged head, which is
what clause 3 buys — and **all seven claude runs are `warm=false`** ([claude-code.md](../agents/claude-code.md):114),
so cross-run reuse contributes nothing and ordering is the only mechanism currently paying. A regression
here is not a cache-feature bug; it multiplies the dominant cost line.

## 5. Risks and failure modes

| # | Hazard | How it enters | Detection |
| --- | --- | --- | --- |
| 1 | **Worktree path** | the harness embeds cwd/workspace in its own L0 or an early message; a fresh worktree per work item is a new prefix ([claude-code.md](../agents/claude-code.md):67-68, [codex.md](../agents/codex.md):93) | `workspace` / `workspace_changed` in warm identity; per-turn `cache_read` |
| 2 | **Git state** | branch, base sha, dirty snapshot, `git status` output in the head | `prefix_changed`; `prefix.log` `first_diff` |
| 3 | **Timestamps** | "generated at", budget window reset, any `now()` in a stable block | same; a fixture that injects a clock |
| 4 | **Generated ids** | session UUID or console URL in a tool description; `uuidv5(thread_id, …)` item ids; billing-hash substitution in stored tool results ([claude-code.md](../agents/claude-code.md):148-151) | identity digest moves with no config change |
| 5 | **Unordered collections** | tool registry is a `HashMap`; paginated MCP discovery; instruction files read from a directory | tool-manifest digest (§6); a two-order fixture |
| 6 | **Per-host paths** | caller artifact path, `$HOME`, memory dir, sandbox stanza rendered verbatim | two-host fixture; digest over the rendered head |
| 7 | **MCP / plugin surface** | a server connects, a plugin enables, a compressing proxy's MCP server toggles, `ENABLE_TOOL_SEARCH` differs per host | tool-manifest digest; the harness's reported tool set where available |
| 8 | **Digest over the head only** | the prefix digest covers header + poll guidance + sandbox stanza + instructions and stops | a second digest for the rendered first message (§6) |
| 9 | **Serialization drift** | a dependency upgrade changes float/key/escape rendering; `serde_json` map order is not a contract | golden *bytes* fixture, not digest-only |
| 10 | **Canonicalisation that hides a change** | canonical form hashed, raw bytes wired (the inverse case) | assert the canonical bytes are the wire bytes |

**The volatile tail must be last and small:** a per-attempt budget block before the plan is worse than
useless, because the plan, research and acceptance bytes move with it — a request assembler can do exactly that (§6).
**Per-work-item worktrees and per-run budgets are hazards a dispatch plane creates:** neither exists in a
single-user CLI session, and both sit inside the prefix.

## 6. Verdict for `warmpath`

**What exists, and is good.** `warmpath::prefix` is a real toolkit — length-delimited SHA-256,
`canonical_text`, `canonical_json`, `canonical_tool_surface`, `compare_prefixes` with `first_differing_byte`
and `PrefixDifference::is_append_only` ([prefix.rs](../../../../src/prefix.rs):70-255).

**What is missing.** (1) **`is_append_only()` has no production consumer** — a grep finds its own unit test
and nothing else. (2) **`CacheIdentity` has no tool-manifest member**
([cache.rs](../../../../src/cache.rs):319-391), though [codex.md](../agents/codex.md):100 recommends
one: same request head, different MCP surface, same identity today. (3) **`canonical_tool_surface` has no
production caller** — the primitive exists, the wiring does not. (4) **Volatile bytes can sit in the
middle of an assembled request:** a per-attempt budget block before the plan re-writes the plan on
every budget change, and a digest that stops before that block cannot see it.

**What to build, cheapest first.**

1. **The tool manifest:** a `ToolManifest` — sorted `(name, definition)` entries plus a `PrefixDigest`,
   built once per session from the surface the caller can see (the MCP config, a proxy's server set when
   enabled, the harness's reported set) and carried as `tools: Option<PrefixDigest>` on `CacheIdentity`,
   absent meaning "unknown, do not claim a match". It turns an invisible invalidator into a comparable
   field; additive on a compatibility surface, so it needs the `0.x` bump and CHANGELOG entry that
   [AGENTS.md](../../../../AGENTS.md) requires.
2. **Prefix invariance as an assertion, not a subsystem:** `compare_prefixes(N, N+1)` already returns
   `is_append_only()`. A scripted step sequence asserting every adjacent pair is append-only, plus two
   negative fixtures (a timestamp in the head, tools reordered), is the whole CI gate — no crate, no
   provider call, no clock.
3. **A second digest for the rendered first message,** so hazard 8 is visible before it bills: keep
   `prefix_digest` as the workspace-independent head digest warm matching uses, add `request_head_digest`
   over the full rendered message, and never merge them — merging makes the warm record
   workspace-dependent and breaks cross-worktree reuse. If a measurement then shows fresh attempts
   re-reading the plan, move the budget block after plan and workspace and drop the window-reset epoch;
   that changes what the agent reads, so it belongs behind a default-off discipline.

**What not to build.** No JCS dependency or RFC 8785 conformance claim (the canonical form never leaves the
process). No normalisation of model-visible content — `canonical_text` is for framing, and trimming
whitespace inside a code block changes the prompt. No reordering of the harness's own L0, which we do not own.

**The cheapest single test.** `tool_manifest_digest_is_order_insensitive_but_name_sensitive`: identical
entries in different orders give one digest, a renamed tool another, a duplicate name with a different
definition a construction error — offline, ~30 lines, and it fails the day a `HashMap` returns.

## 7. Confidence and gaps

| Claim class | Confidence | Why |
| --- | --- | --- |
| Providers match an exact byte prefix; reuse ends at the first differing byte | **high** | four vendor families in [01](../../prompt-caching/01-provider-mechanics.md), re-read through three agent docs |
| Layer order and canonical serialization are what make it cheap | **high** | vendor statements (claude, agy, dsh) plus the measured shares in §4 |
| The seven agents' mechanisms and grades | **high** for claude/codex/agy/dsh; **medium/low** for kimi; **unknown** for grok's and perplexity's harness order | §3 |
| The 4×–20× multiplier | **high** as arithmetic on published multipliers; **medium** as a forecast for one workload | §4 |
| That one deployment already enjoys steady-state ordering | **high** | 93–99% measured shares, all within-run |
| That the budget block re-writes the plan bytes | **high** on code shape in one deployment; **unmeasured** as a billed cost | no dispatch run made |

Gaps: (1) **no live capture** — nothing here is a request-body diff for one deployment, so every claim
about its request is source-level; (2) **cross-attempt request variance is unmeasured** — no two
dispatches were compared; (3) **the harness's L0 is invisible**, so `CacheIdentity.prefix` is a digest of
the caller's head plus a separate workspace field, not of the request; (4) kimi, grok and perplexity have
provider advice only; (5) **#82229 is one reporter on 2.1.220**, credible but not vendor-confirmed.

## 8. Sources

Web, accessed 2026-10-28:

- [RFC 8785 — JSON Canonicalization Scheme](https://www.rfc-editor.org/rfc/rfc8785.html) — IETF, June 2020, fetched — recursive property sorting by UTF-16 code units, no insignificant whitespace, ECMAScript number serialisation, UTF-8 output; and the note that sorting UTF-8 keys can differ.
- [anthropics/claude-code#27786](https://github.com/anthropics/claude-code/issues/27786) — reported 2026-02-22, snippet only — a generated id inside a tool definition breaks the prefix; ~12× input penalty; the tool block is ~17 k tokens.
- [anthropics/claude-code#82229](https://github.com/anthropics/claude-code/issues/82229) — CC 2.1.220, fetched — a per-session identifier in `tools[]` defeats cross-session reuse, verified by diffing captured bodies; names #27786 as the SDK sibling.
- [zeroclaw-labs/zeroclaw#10959](https://github.com/zeroclaw-labs/zeroclaw/pull/10959) — Sep 2026, snippet only — a `HashMap`-backed MCP registry emitting tools in a different order per process; the Anthropic breakpoint sits on the last tool definition. **UGC: hypothesis-grade only.**
- [codewhale-hq/Codewhale@2576408](https://github.com/codewhale-hq/Codewhale/commit/2576408484e467e9dc651094bf64b156f537e093) — commit, snippet only — sorting discovered tools by name at three points, with a paginated-discovery regression test. **UGC: practice corroboration only.**

Local: [claude](../agents/claude-code.md):31,54-68,73-78,114,148-151 · [codex](../agents/codex.md):24,38,43-47,91-93,100-101 · [agy](../agents/agy.md):25,38-51,97-99 · [dsh](../agents/dsh.md):25,38-54,96-99,113 · [kimi](../agents/kimi.md):36,77-82,135-139 · [grok](../agents/grok.md):38,48-52,72-82 · [perplexity](../agents/perplexity.md):57,88-94 · [01](../../prompt-caching/01-provider-mechanics.md):8-15,61-72 · [02](../../prompt-caching/02-harness-practices.md):10-21,38-57 · [03](../../prompt-caching/03-practitioner-evidence.md):14-28,74-79 · [warmpath DESIGN](../../../../docs/DESIGN.md):39-45,54-61 · `src/prefix.rs`:70-85,142-148,174-191,193-223,225-246 · `src/cache.rs`:280-303,319-391 · `src/prompt.rs`:1-20.
