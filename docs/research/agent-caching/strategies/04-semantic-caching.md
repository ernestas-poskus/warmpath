# Semantic caching

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Status:** research, 2026-10-28
- **Scope:** caching a **model response** and returning it for a *different but similar* request, matched by embedding or
  normalized-text similarity. Not the provider prefix cache ([01](../../prompt-caching/01-provider-mechanics.md)), not an
  exact-request cache ([07](07-exact-request-cache.md)), not decision memoization (§1, §6).
- **Method:** 17 local files (seven agent docs, prompt-caching 00–03, one deployment's cost model, cost ledger, and
  routing-cache source); 8 web sources (3 fetched, 5 snippet-only; one fetched arXiv id was a mis-citation);
  access date 2026-10-28.
- **Companion:** [index](../README.md) · [matrix](../matrix.md)

## 1. What the strategy is (and is not)

A semantic cache sits **in front of the model**: a new request is reduced to a vector (or normalized token sequence), compared
against stored `(key, response)` entries, and — if the nearest entry is close enough — the stored response is returned with **no
model call**. Three properties define it, not the word "semantic": an *approximate* match predicate, a *model-output* payload,
and a *skipped* call. Four neighbours differ on at least one:

| Neighbour | Match predicate | Model called? | Payload | Failure mode |
| --- | --- | --- | --- | --- |
| Provider prefix cache ([01](../../prompt-caching/01-provider-mechanics.md)) | **exact bytes** over a prefix | yes | tokens, discounted | silent miss; higher bill |
| Exact-request cache ([07](07-exact-request-cache.md)) | byte/key identity | no | response | stale answer for an identical request |
| **Semantic response cache** (this doc) | similarity ≥ τ, or learned per-entry τ | **no** | response | **confident wrong answer** |
| Route / decision memoization | normalized key or similarity over a short task string | yes, on a chosen model | a *label* | suboptimal tier for one run |
| Embedding retrieval (RAG) | similarity ≥ k | yes | *context bytes* | degraded answer; model can reject them |

**Normalization is not semantics.** Lowercasing, stripping punctuation, or bucketing a quota figure yields a *canonical form* that
is then hashed **exactly** — nothing is approximate. A routing cache built this way is **normalized-text memoization**, not
semantic matching: it lowercases an alphanumeric word sequence of the task and hashes it together with the route's decision
inputs (provider, band, risk class, model candidates, quota buckets). There is no embedding, vector index, cosine metric, or
nearest-neighbour search in that design. Calling it "semantic" overstates the mechanism.

**A prefix cache is the inverse problem.** The provider matches the *history* and re-prices the *tail*; a semantic cache must
ignore the history and match the tail (§4).

## 2. How it works

| Decision | What implementations do | What it costs / breaks |
| --- | --- | --- |
| **Embedding model** | Pluggable encoder over the request text (GPTCache); vCache is "agnostic to the underlying embedding model" ([2502.03771](https://arxiv.org/abs/2502.03771)) | The encoder must separate requests whose *answers* differ. General-purpose sentence embeddings are trained for topical similarity, so "fix the failing test in `parser.rs`" and "…in `lexer.rs`" land close — inference, medium. The embedding, not τ, sets the ceiling. |
| **Similarity, threshold** | Static cosine τ is the default. vCache: static thresholds "do not give formal correctness guarantees, result in unexpected error rates, and lead to suboptimal cache hit rates"; it learns a **per-cached-prompt threshold online** under a user-set max error rate (ICLR 2026) | τ is a precision/recall *policy*. Published treatments bound error probabilistically over a distribution, never per request. |
| **Key normalization** | Strip boilerplate, canonicalize case/whitespace, hash a projected subset, exclude volatile fields | Over-normalize and two questions share a key; under-normalize and nothing hits. The choice silently *is* the semantic model. |
| **Storage** | ANN index beside the response blob, or a plain map for normalized-exact keys | The store holds user content → isolation (org/tenant/run) becomes a security property. |
| **Eviction** | TTL + LRU is the naive default; a 2026 blog claims frequency-biased, cluster-aware eviction (SphereLFU) beats LRU on topic-skewed traffic ([MLflow](https://mlflow.org/articles/tags/semantic-cache-llm/), secondary) | Near-identical entries evict together, so the *worst* answers persist longest. Unverified; low grade. |
| **False-positive policy** | Serve-and-hope, log-and-shadow, or a verified bound (vCache). Apigee ships a **Semantic caching policy** — "serving cached responses for queries that are semantically similar" — with no threshold semantics on the page read | The only honest policies are "measure precision on labelled traffic" or "do not serve". |

**What a hit means.** A response produced for request *A* is returned to request *B* with no model call, no tool call, no usage
field, and no transport-level signal. **What failure looks like:** a fluent, on-topic, confidently wrong answer — Perplexity's doc
records third-party advice that a false hit serves "a real, fluent, on-topic answer — to the neighbouring question"
([perplexity.md:106](../agents/perplexity.md)). Nothing errors; the verifier judges the artifact, not its provenance.

## 3. Cross-agent evidence

Verified: all seven report no semantic response cache — with the grades the docs assign, which are *not* uniform.

| Agent | Reported mechanism | Grade as written | Actually inspected |
| --- | --- | --- | --- |
| Claude Code | "**none documented**"; prefix match is exact | medium (absence) | vendor pages + 3 issues; closed source ([claude-code.md:33](../agents/claude-code.md)) |
| Codex CLI | none; "no embedding index over history, no response dedupe, no near-match shortcut" | high (absence) | source at `rust-v0.164.0` ([codex.md:26](../agents/codex.md), [:55](../agents/codex.md)) |
| agy | none in docs, changelog, `/config`, flags: "two byte-identical requests are two provider calls" | high | ~1,000 changelog lines, 3 searches ([agy.md:27](../agents/agy.md), [:61](../agents/agy.md)) |
| dsh | "none found"; `grep -r semantic_cache` over `packages/`+`apps/` → 0 files | high (absence) | DSH checkout at `22ddee3` ([dsh.md:27](../agents/dsh.md), [:62](../agents/dsh.md)) |
| Grok | "nothing at any layer" | none documented | xAI docs; harness layer **not inspected** ([grok.md:40](../agents/grok.md), [:20](../agents/grok.md)) |
| Kimi | none; no embedding or normalized-task reuse in API or CLI docs | high that it is *undocumented* | guide, pricing, Messages ref, CLI docs ([kimi.md:38](../agents/kimi.md)) |
| Perplexity | absent; third-party posts build one *in front of* the API | absent, medium-high | 5 searches, 3 fetches ([perplexity.md:59](../agents/perplexity.md), [:102](../agents/perplexity.md)) |

1. **"None documented", not "none exists".** Claude Code is closed source, and grok.md grades layer (b) *Unknown* on the line above
   its "nothing at any layer" row — the claim is broader than the inspection. dsh's grep covered the DSH checkout only.
2. **No vendor is incentivised to ship one.** Providers sell the prefix discount; a harness-side response cache removes billable
   tokens instead of discounting them, and the harness would own the correctness claim.
3. **The near-miss proves the distinction.** All seven cache *decisions* or *bytes*; none caches *answers*. Absence across five
   languages and four dialects outweighs any single doc's grade.

**Contradiction found.** [claude-code.md](../agents/claude-code.md) places the deployment's routing cache in the routing
configuration; in that deployment the key lives in the model-selection configuration instead, and the routing section has no
such field. [grok.md](../agents/grok.md) states it correctly. The substantive claim in both — routing decisions, not model
output — is right; claude-code.md's section name is stale.

## 4. Measured evidence and economics

| Workload | fresh / cached / output | cached share of input | effective $/M input vs list |
| --- | --- | --- | --- |
| claude, one run | 28 / 744,078 / 6,713 | **99.996%** | ≈$0.20 vs $4 → **5%** |
| codex, 3 work items | 51,301 / 1,045,504 / 6,716 | 95.3% | ≈$0.25 vs $1.75 → 14% |
| codex, 7 runs | 685,887 / 20,281,216 / 59,228 | **96.73%** | ≈$0.23 vs $1.75 → **13%** |
| DSH orchestration | 1,353,561 / 177,797,000 / 262,709 | 99.24% | ≈$0.004 vs $0.15 → 2.7% |
| agy, one run | 176,182 / **0** / 18,256 | 0 reported | pre-`cache_read_tokens`-fix — not evidence agy never caches |

A token in the stream this pipeline sends already costs **2.7–13% of list**: the prefix cache took the first 87–97% of the value.

**The structural argument is stronger than the arithmetic.** A coding request is `P + t` — `P` the prior conversation, `t` one new
message or tool result. Consecutive requests share `P` almost exactly (the 95.3–99.996% above). What a similarity metric sees:

- Over the **whole request**, any two turns in a session are ≥0.95 similar by token count regardless of the question. τ fires on
  every pair, and every hit answers a *different question*.
- Over the **tail only**, similarity is informative — but `t` ("now fix the second failure") has no self-contained meaning; it is a
  function of `P`, which the matcher just discarded.

The inversion: **the prefix cache matches the part that is identical and prices the part that differs; a semantic cache must match
the part that differs while ignoring what makes it interpretable.** A hit is either trivially true and wrong (whole-request
matching) or underdetermined (tail matching); no threshold repairs the conditioning.

**How much of the stream is near-duplicate?** *Within a run*, 95.3–99.996% of input bytes are already-sent prefix: measured,
already monetised, and byte duplication rather than semantic duplication (structural, "95–99.9%",
[README:57](../../prompt-caching/README.md)). *Across runs* it is **never measured here** — no experiment computed embedding
similarity between requests or trailing messages. The pipeline is built to make it low: each dispatch is a different work item, plan,
worktree path and tree state, and both agent docs name worktree path / `cwd` as prefix differentiators
([claude-code.md:68](../agents/claude-code.md), [codex.md:93](../agents/codex.md)). The population a semantic cache would serve is one this
design deliberately never repeats. That missing number decides the case, and it is absent.

**Output is what is left over.** Codex's 59,228 output tokens are 0.28% of its input but 15% of its bill ($0.829 of $5.579): a hit
eliminates the tokens that differ every time by design, and whose substitution is most visible wrong.

## 5. Risks and failure modes

- **Non-determinism makes "the stored answer" a sample, not a fact.** Caching one draw makes it canonical for every later near-match.
- **Tool side effects.** A coding response is often a tool call or an edit plan; returning a stored one skips the implied calls and
  desynchronises transcript from tree. The verifier judges what was *produced*; a cached response produces nothing.
- **Stale patches.** The answer encodes tree state at *t*; a hit applies it to a moved file, or a tree where the bug is fixed.
- **Prompt-injection persistence.** The cache is a write path into future model context that bypasses request assembly; one poisoned entry
  serves every future near-match ("a poisoned or manipulated cache entry can serve the same wrong answer to every future matching
  query", [MLflow](https://mlflow.org/articles/tags/semantic-cache-llm/), low grade). Provider caches are org/workspace-isolated
  ([01](../../prompt-caching/01-provider-mechanics.md)); a self-built one only if built so.
- **Evaluation is circular.** *Hit precision* — of hits served, how many were correct — needs the call the cache exists to avoid.
  vCache makes the error rate a **policy to defend**, not a property to assume. No source read publishes measured hit precision for
  coding traffic; Apigee's page states the capability, not its precision.
- **Accounting integrity.** A hit emits no usage fields, and every number in a measured deployment's cost model, cache ledger,
  and per-agent gate comes from provider usage. A semantic layer removes requests from the denominator: hit rate and EEPR would
  read *better* while less work happened. One deployment already needed a separate `cache_hit` fact so *result* reuse was not
  averaged with *prompt* reuse; this needs the same.

## 6. Verdict for `warmpath`

**Do not build model-output semantic caching for coding dispatch.** Ranked:

1. The addressable base is the residual of a 95–99.99% prefix hit rate priced at 2.7–13% of list — a small ceiling the provider
   already gave away.
2. The informative similarity is over the tail, whose meaning depends on the prefix: the match is either trivially true (§4) or
   unsound. A mechanism problem, not a tuning problem.
3. A wrong hit is silent, lands in a code-modifying run, and the verifier sees the artifact, not its provenance.
4. It would corrupt usage-denominated accounting the cost gates depend on, unless a separate fact is added first. None of the seven
   ships one, and the two that cache hardest use a mechanism the provider prices.

**What is legitimate, and already done — keep it, name it correctly.**

- A normalized-text **decision memoization** cache is the legitimate form: a `DefaultHasher` over a lowercased word sequence
  plus config fields and quota buckets, a short default TTL, and a bounded map with **oldest-insert eviction** — FIFO, not LRU,
  because a read never refreshes the timestamp. Safe *because the payload is a label*: the answer is one of a pre-validated
  candidate set, so a wrong label costs one run at a suboptimal tier, not a wrong answer. A 16 KiB task cap collapses long tasks
  onto a shared key.
- `resume_hints` persists a *decision* against a session prefix — again a label, again recoverable.
- **Search/index caching** is a different object: Perplexity's `web_search`/`fetch_url` use its own index/fetch layer, "internal,
  not billed as cache reads" ([perplexity.md:61](../agents/perplexity.md)). Caching *retrieval over documents* is defensible
  because documents are near-immutable and the answer is still generated from them.
- **Embedding-based retrieval, not caching**, is the sound use of the machinery: a bad retrieval degrades an answer; a bad cache
  replaces it.

**If it is ever considered anyway, the evidence threshold.** All of these, in order:

1. A measured **cross-run near-duplicate rate** on real traffic — ≥200 requests across ≥20 work items, tail identified per request.
2. An offline replay reporting **hit precision and recall with labels** on the tail, embedding cost included; precision ≥0.99.
3. A **shadow mode** logging proposed hits and serving none, long enough to bound error on traffic the system never trained on.
4. A hard class restriction: **never** for a response that may contain a tool call or patch; only read-only, context-independent
   sub-questions ("what does this error mean").
5. Per-tenant keys plus explicit invalidation on repo, plan, tool-surface, model or effort change.
6. A `semantic_cache_hit` fact kept separate from `cached_input_tokens`, so hit-rate and EEPR denominators stay honest.
7. Head-to-head against the **96.73%** measured baseline, not against an uncached strawman.

Point 4 decides it: the questions that survive it are not the questions dispatch asks.

## 7. Confidence and gaps

| Claim class | Confidence | Why |
| --- | --- | --- |
| Definition and layering (§1) | **high** | mechanism-level; prefix exactness is vendor-stated ([README:53-55](../../prompt-caching/README.md)) |
| The deployment's `semantic_cache` is a normalized hash, not embeddings | **high** | source read; no embedding/vector/cosine symbol in the crate |
| All seven ship no semantic response cache | **medium-high** | uniform across 7 docs, 2 closed-source; "none documented" ≠ "none exists" |
| Measured cached shares and derived effective prices | **high** | measured rows, recomputed here from the recorded tokens |
| The prefix/semantic inversion (§4) | **medium-high** | follows from the request shape both agent docs describe; no isolating experiment |
| Cross-run near-duplicate rate is low | **unknown** | never measured; inferred from dispatch structure only |
| vCache's per-entry threshold result | **medium** | abstract read, ICLR 2026 noted; effect size truncated in the fetched text, so none quoted |
| Apigee policy semantics, production hit precision | **low / absent** | one-line purpose only; no precision source found |

Gaps, named: (1) **the GPTCache citation in the original research framing is wrong** — arXiv 2308.02490 resolves to *MM-Vet*, not GPTCache; the
GPTCache paper is the NLP-OSS 2023 workshop paper (pp. 212–218, per the ACL Anthology record and vCache's references) and its full
text was not fetched, so §2's encoder/storage detail rests on the published description; (2) no production write-up with measured
hit precision was found — the gateway material ([Vercel](https://vercel.com/i/semantic-caching-llm-apps)) states the condition
("users independently phrase the same factual, context-independent question") without a measurement; (3) no live run today, so
every token figure is second-hand; (4) agy's 0-cached row is an accounting artefact, not a measurement; (5) no embedding model was
benchmarked, so "general-purpose embeddings are topical" is inference; (6) the 2026 "Prompt Caching ROI"-style material the original research
warned about was not used — nothing here rests on it.

## 8. Sources

- [vCache: Verified Semantic Prompt Caching](https://arxiv.org/abs/2502.03771) — arXiv:2502.03771v5 (rev. 2026-02-21), ICLR 2026,
  **fetched** 2026-10-28 — static thresholds give no formal guarantee; online per-prompt threshold under a user-set error bound.
- [arXiv:2308.02490](https://arxiv.org/abs/2308.02490) — **fetched** 2026-10-28 — *is* MM-Vet (Yu et al., ICML 2024), **not**
  GPTCache; recorded as a mis-citation.
- [GPTCache, NLP-OSS 2023, pp. 212–218](https://aclanthology.org/people/fu-bang/unverified/) — Fu Bang, snippet only — stores LLM
  responses and answers from the cache before calling the model.
- [Apigee AI gateway capabilities](https://docs.cloud.google.com/apigee/docs/api-platform/get-started/ai-capabilities?hl=en) —
  Google Cloud, updated 2026-10-26, **fetched** 2026-10-28 — a "Semantic caching policy" exists ("serving cached responses for
  queries that are semantically similar"); no threshold semantics published. Public preview 2025-05/06, GA 2025-10, per the
  [June](https://discuss.google.dev/t/apigee-release-roundup-june-2025/192441/2) and
  [September](https://discuss.google.dev/t/apigee-releases-september-2025/272519) release posts (snippet only).
- [Semantic caching for LLM apps](https://vercel.com/i/semantic-caching-llm-apps) — Vercel, 2026-08-02, snippet only — the
  qualifying condition is repeated *context-independent* questions; vendor page, not a measurement.
- [MLflow, "semantic cache llm"](https://mlflow.org/articles/tags/semantic-cache-llm/) — 2026-09-12, snippet only — cluster-aware
  eviction (SphereLFU) claimed to beat LRU; poisoning persists across matches. Secondary; no numbers used.

**Local (2026-10-28):** [claude](../agents/claude-code.md) (`:33`, `:68`, `:83`) · [codex](../agents/codex.md) (`:26`, `:55`, `:77`,
`:93`) · [agy](../agents/agy.md) (`:27`, `:61`) · [dsh](../agents/dsh.md) (`:27`, `:62`) · [grok](../agents/grok.md) (`:20`,
`:40`, `:94`) · [kimi](../agents/kimi.md) (`:38`, `:91`) · [perplexity](../agents/perplexity.md) (`:59`, `:61`, `:102-107`);
[prompt-caching README](../../prompt-caching/README.md) (`:53-58`, `:75-76`), [01](../../prompt-caching/01-provider-mechanics.md)
(`:5-15`, `:26-27`), [02](../../prompt-caching/02-harness-practices.md) (`:38-58`).
