# Sources and confidence

> **Provenance.** Copied from the project's caching research (dated in the original) and de-branded
> for this crate. This copy is maintained here.

- **Accessed:** 2026-10-26
- **Rule:** vendor docs/source are primary; GitHub issues and HN are evidence of observed behavior;
  engineering blogs are secondary; Reddit was not directly fetchable and is not primary here.

## Primary: provider documentation

- Anthropic prompt caching — <https://platform.claude.com/docs/en/build-with-claude/prompt-caching>
- OpenAI prompt caching — <https://developers.openai.com/api/docs/guides/prompt-caching>
- DeepSeek context caching — <https://api-docs.deepseek.com/guides/kv_cache>
- Gemini caching — <https://ai.google.dev/gemini-api/docs/caching>
- Gemini CLI token caching — <https://gemini-cli-docs.pages.dev/en/cli/token-caching.html>
- Claude Code prompt caching — <https://code.claude.com/docs/en/prompt-caching>
- Claude Code costs — <https://code.claude.com/docs/en/costs>
- Aider prompt caching — <https://aider.chat/docs/usage/caching.html>

## Primary: public harness engineering and source

- Claude Code: “Prompt caching is everything” — <https://claude.dev/blog/lessons-from-building-claude-code-prompt-caching-is-everything/>
- Cursor: “Improved token efficiency for longer agent runs” — <https://cursor.com/blog/improved-token-efficiency>
- OpenHands: context condensation — <https://www.openhands.dev/blog/openhands-context-condensensation-for-more-efficient-ai-agents>
- Cline: prompt-caching PR — <https://github.com/cline/cline/pull/2627>
- Continue: turn-level caching — <https://github.com/continuedev/continue/pull/10801>
- Goose: cache-safe request assembly — <https://github.com/aaif-goose/goose/issues/11019>
- OpenHands: cache-preserving condenser — <https://github.com/OpenHands/OpenHands/pull/7893>

## Evidence: issues, PRs, and forum posts

- [anthropics/claude-code#46829](https://github.com/anthropics/claude-code/issues/46829) — TTL/cost analysis
- [anthropics/claude-code#40524](https://github.com/anthropics/claude-code/issues/40524) — conversation cache rewrite
- [anthropics/claude-code#27786](https://github.com/anthropics/claude-code/issues/27786) — random UUID in tool description
- [openai/codex#35300](https://github.com/openai/codex/issues/35300) — explicit breakpoint gap
- [openai/codex#37305](https://github.com/openai/codex/issues/37305) — compaction omits tools
- [openai/codex#21796](https://github.com/openai/codex/issues/21796) — per-thread cache key
- [openai/codex#30425](https://github.com/openai/codex/issues/30425) — intermittent zero hits
- [cline/cline#15307](https://github.com/cline/cline/issues/15307) — breakpoints stop advancing
- [cline/cline#12913](https://github.com/cline/cline/issues/12913) — Bedrock dialect
- [cline/cline#10596](https://github.com/cline/cline/issues/10596) — OpenRouter provider pinning
- [Aider-AI/aider#4372](https://github.com/Aider-AI/aider/issues/4372) — keepalive pings
- [HN $38k Bedrock bill](https://news.ycombinator.com/item?id=47933355)
- [HN git-status cache break](https://news.ycombinator.com/item?id=47754795)

## Local measurement and implementation

- `docs/subsystems/system-prompt.md`, `docs/subsystems/token-meter.md`,
  `docs/subsystems/compaction.md` (DSH checkout)
- `packages/core/agent-loop/README.md`, `packages/compaction/compaction-basic/README.md`,
  `packages/compaction/compaction-tool-result-pruner/README.md` (DSH checkout)
- one deployment's request assembly, cache eligibility and gates, cold-start and session
  handling, and usage parsing
- that deployment's cost model, warm-resume calibration report, and cost ledger

## Confidence and gaps

| Claim class | Confidence | Why |
| --- | --- | --- |
| Provider cache units, usage fields, TTL/pricing | high | vendor primary docs |
| Claude Code public behavior | high | official docs, source-adjacent issue evidence |
| Codex/Cline/Continue/Goose behavior | high/medium | source, maintainer issues, before/after measurements; `main` moves |
| Cursor/Amp internals | medium | official engineering prose; no source access |
| Practitioner cost multipliers | medium/low | self-reported; arithmetic often consistent but not audited |
| Reddit sentiment | not used | direct fetch blocked; secondhand claims excluded from numbers |

Provider behavior changes quickly. Re-verify minimums, TTLs, and model-generation-specific fields
against the pinned provider version before implementing a breakpoint or TTL policy.

## Where these corrections now live (2026-10-27)

The verified corrections in `01`–`03` are encoded in the standalone `warmpath` crate:
`cache::semantics` carries the model-dependent Anthropic minimums/read multipliers and the
5m/1h write costs, Gemini's implicit `>=2050`-token rule with explicit caching legacy-only, OpenAI's
automatic cache with model-gated writes, and strict semantics for unknown routes. `warmpath` also
records the evidence status of each practitioner finding in [03](03-practitioner-evidence.md) so a
measured-but-unfixed upstream issue is not read as settled behaviour, and it models the TTL as a
step with a `(w − r)` resume counterfactual.

The sources themselves did not change; what changed is that the numbers now have one reviewed home
and a version (`SEMANTICS_VERSION`) that recorded runs can name.
