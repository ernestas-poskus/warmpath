//! Provider cache semantics, cache identity, break reasons, and cold-start accounting.
//!
//! Two rules shape this module:
//!
//! 1. **Unknown is strict.** A `(provider, model)` route the table does not declare gets
//!    [`CacheMode::Strict`]: no marker, no declared TTL, and unit cost multipliers — the analysis
//!    claims nothing it cannot back with a documented contract. A gateway that may rewrite the
//!    request is therefore strict by construction, not by a caller remembering to say so.
//! 2. **A cache share is token-weighted and pooled.** Cold and warm requests are summed as tokens,
//!    never averaged as percentages, and a run whose usage was not reported enters neither ratio.
//!
//! The contract table follows the behavior each provider documents and the per-contract notes
//! below; a change to an existing route's numbers is a [`SEMANTICS_VERSION`] bump, because recorded
//! runs name the version they were interpreted against.

use serde::{Deserialize, Serialize};

use crate::newtypes::{
    BasisPoints, CachedTokens, EpochSeconds, FreshTokens, HarnessId, ModelId, PrefixDigest,
    ProviderId, Ratio, Tokens, TtlSecs,
};
use crate::prefix::{PrefixChange, digest};
use crate::reported::Reported;

/// The semantics-contract version this build records.
pub const SEMANTICS_VERSION: u32 = 1;

/// The provider's cache mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheMode {
    /// The client places an explicit cache marker.
    Marker,
    /// The provider caches repeated prefixes automatically.
    Automatic,
    /// The client chooses explicit breakpoints.
    ExplicitBreakpoint,
    /// No declared contract; claim nothing.
    Strict,
}

impl CacheMode {
    /// The stable spelling.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            CacheMode::Marker => "marker",
            CacheMode::Automatic => "automatic",
            CacheMode::ExplicitBreakpoint => "explicit_breakpoint",
            CacheMode::Strict => "strict",
        }
    }
}

/// A cache lifetime, as the provider documents it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheTtl {
    /// A declared lifetime in seconds.
    Seconds(TtlSecs),
    /// The provider documents a default it does not quantify.
    ProviderDefault,
    /// No lifetime is documented, or the contract is strict.
    Unknown,
}

impl CacheTtl {
    /// The lifetime in seconds, when one is declared.
    #[must_use]
    pub const fn seconds(self) -> Option<TtlSecs> {
        match self {
            CacheTtl::Seconds(ttl) => Some(ttl),
            CacheTtl::ProviderDefault | CacheTtl::Unknown => None,
        }
    }

    /// The stable spelling.
    #[must_use]
    pub fn code(self) -> String {
        match self {
            CacheTtl::Seconds(ttl) => format!("{}s", ttl.get()),
            CacheTtl::ProviderDefault => "provider_default".to_owned(),
            CacheTtl::Unknown => "unknown".to_owned(),
        }
    }
}

/// The declared cache contract for one provider route.
///
/// The multipliers are **cost ratios against fresh input**: a read at 0.1 costs a tenth of a fresh
/// prompt token, and a write at 1.25 costs a quarter more. They are [`Ratio`] values, never `f64`,
/// so money math downstream stays exact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheContract {
    /// The mechanism.
    pub mode: CacheMode,
    /// The default lifetime.
    pub ttl: CacheTtl,
    /// Cost of a cache read relative to fresh input.
    pub read_multiplier: Ratio,
    /// Cost of writing to the cache relative to fresh input.
    pub write_multiplier: Ratio,
    /// Cost of a write with the provider's long lifetime, where one is offered.
    pub long_write_multiplier: Option<Ratio>,
    /// The provider's long lifetime, where one is offered.
    pub long_ttl: Option<TtlSecs>,
    /// The minimum cacheable prompt length this build documents, where model-dependent.
    pub minimum_tokens: Option<Tokens>,
    /// Whether explicit caching is legacy-only for this provider generation.
    pub explicit_legacy_only: bool,
    /// A one-line note naming the verified behaviour this contract encodes.
    pub note: &'static str,
}

impl CacheContract {
    /// Whether the contract is strict (no cache claimed).
    #[must_use]
    pub const fn is_strict(&self) -> bool {
        matches!(self.mode, CacheMode::Strict)
    }

    /// The cost saved per cached token, relative to fresh input: `1 - read_multiplier`.
    #[must_use]
    pub fn read_saving(&self) -> Ratio {
        Ratio::new(1, 1)
            .expect("one over one")
            .saturating_sub(self.read_multiplier)
    }

    /// The net cost of reusing a cached prefix instead of rewriting it: `write - read`.
    ///
    /// A warm resume reads tokens it would otherwise have to write again; the per-token
    /// counterfactual is the avoided write minus the read actually paid. Negative savings
    /// saturate at zero. This is the `(w - r)` factor the warm-resume estimate multiplies by
    /// overlap and cached tokens.
    #[must_use]
    pub fn reuse_net_saving(&self) -> Ratio {
        self.write_multiplier.saturating_sub(self.read_multiplier)
    }

    /// The minimum prompt length this contract can cache, when declared.
    #[must_use]
    pub const fn minimum_tokens(&self) -> Option<Tokens> {
        self.minimum_tokens
    }
}

/// A provider family, classified from its wire spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderFamily {
    /// Anthropic's Messages API (explicit markers, model-dependent minimums).
    Anthropic,
    /// OpenAI's automatic prefix cache.
    OpenAi,
    /// DeepSeek's automatic, best-effort prefix cache.
    DeepSeek,
    /// Google Gemini's implicit cache (explicit caching is legacy-only).
    Gemini,
    /// Anything else: strict.
    Unknown,
}

impl ProviderFamily {
    /// Classify a provider name, case-insensitively and trimmed.
    #[must_use]
    pub fn of(provider: &ProviderId) -> Self {
        match provider.as_str().trim().to_ascii_lowercase().as_str() {
            "claude" | "anthropic" => ProviderFamily::Anthropic,
            "codex" | "openai" => ProviderFamily::OpenAi,
            "deepseek" => ProviderFamily::DeepSeek,
            "gemini" | "google" => ProviderFamily::Gemini,
            _ => ProviderFamily::Unknown,
        }
    }
}

fn ratio(numerator: u128, denominator: u128) -> Ratio {
    Ratio::new(numerator, denominator).expect("non-zero denominator")
}

/// The Anthropic minimum cacheable prompt length, model-dependent.
///
/// The documented minimums are generation-dependent (roughly 512–4 096 tokens); this table
/// encodes the documented buckets rather than a single global number.
fn anthropic_minimum(model: &str) -> Option<Tokens> {
    let model = model.to_ascii_lowercase();
    if model.contains("opus-4") || model.contains("opus-3") {
        Some(Tokens::new(4_096))
    } else if model.contains("sonnet-4") || model.contains("sonnet-3-7") {
        Some(Tokens::new(1_024))
    } else if model.contains("haiku") {
        Some(Tokens::new(2_048))
    } else {
        Some(Tokens::new(1_024))
    }
}

/// Resolve a route's cache contract.
///
/// `model` selects model-dependent Anthropic facts; an empty model on a known provider is strict
/// (the contract may change by generation, so an unaddressed route claims nothing).
#[must_use]
pub fn semantics(provider: &ProviderId, model: Option<&ModelId>) -> CacheContract {
    let family = ProviderFamily::of(provider);
    let model_text = model
        .map(ModelId::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned();
    if family != ProviderFamily::Unknown && model_text.is_empty() {
        return strict();
    }
    match family {
        ProviderFamily::Anthropic => CacheContract {
            mode: CacheMode::Marker,
            ttl: CacheTtl::Seconds(TtlSecs::new(300).expect("non-zero")),
            read_multiplier: ratio(1, 10),
            write_multiplier: ratio(125, 100),
            long_write_multiplier: Some(ratio(2, 1)),
            long_ttl: TtlSecs::new(3_600),
            minimum_tokens: anthropic_minimum(&model_text),
            explicit_legacy_only: false,
            note: "explicit cache_control; 5m default at 1.25x write, 1h at 2x; read ~0.1x; minimum is model-dependent",
        },
        ProviderFamily::OpenAi => CacheContract {
            mode: CacheMode::Automatic,
            ttl: CacheTtl::Seconds(TtlSecs::new(1_800).expect("non-zero")),
            read_multiplier: ratio(1, 10),
            write_multiplier: ratio(125, 100),
            long_write_multiplier: Some(ratio(25, 10)),
            long_ttl: TtlSecs::new(43_200),
            minimum_tokens: Some(Tokens::new(1_024)),
            explicit_legacy_only: false,
            note: "automatic prefix cache; reads ~0.1x; newer models charge 1.25x/2.5x writes, older models none; prompt_cache_breakpoint is model-gated",
        },
        ProviderFamily::DeepSeek => CacheContract {
            mode: CacheMode::Automatic,
            ttl: CacheTtl::Unknown,
            read_multiplier: ratio(1, 10),
            write_multiplier: ratio(1, 1),
            long_write_multiplier: None,
            long_ttl: None,
            minimum_tokens: None,
            explicit_legacy_only: false,
            note: "automatic best-effort disk cache; no declared TTL; writes are not separately charged",
        },
        ProviderFamily::Gemini => CacheContract {
            mode: CacheMode::Automatic,
            ttl: CacheTtl::Unknown,
            read_multiplier: ratio(25, 100),
            write_multiplier: ratio(1, 1),
            long_write_multiplier: None,
            long_ttl: None,
            minimum_tokens: Some(Tokens::new(2_050)),
            explicit_legacy_only: true,
            note: "implicit caching is automatic and opportunistic with a >=2050-token rule; explicit context caching is legacy-only",
        },
        ProviderFamily::Unknown => strict(),
    }
}

/// The contract for a route this build cannot back with a documented source.
#[must_use]
pub fn strict() -> CacheContract {
    CacheContract {
        mode: CacheMode::Strict,
        ttl: CacheTtl::Unknown,
        read_multiplier: ratio(1, 1),
        write_multiplier: ratio(1, 1),
        long_write_multiplier: None,
        long_ttl: None,
        minimum_tokens: None,
        explicit_legacy_only: false,
        note: "no declared contract: no marker, no TTL, no claimed savings",
    }
}

/// Why the first request of a run could not reuse a cached prefix, when attributable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheBreakReason {
    /// The prefix was rewritten in place (an instruction edit, a random byte).
    PrefixRewritten,
    /// The provider/model/effort route changed.
    RouteChanged,
    /// The tool surface changed.
    ToolSurfaceChanged,
    /// Compaction rewrote the conversation.
    Compaction,
    /// A cold start: no prior warm session.
    ColdStart,
    /// The provider missed despite a matching prefix.
    ProviderMiss,
    /// A policy change moved the request shape.
    PolicyChange,
    /// The cause is not attributable.
    Unknown,
}

impl CacheBreakReason {
    /// The stable code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            CacheBreakReason::PrefixRewritten => "prefix_rewritten",
            CacheBreakReason::RouteChanged => "route_changed",
            CacheBreakReason::ToolSurfaceChanged => "tool_surface_changed",
            CacheBreakReason::Compaction => "compaction",
            CacheBreakReason::ColdStart => "cold_start",
            CacheBreakReason::ProviderMiss => "provider_miss",
            CacheBreakReason::PolicyChange => "policy_change",
            CacheBreakReason::Unknown => "unknown",
        }
    }
}

/// The cache identity of one request: the prefix, the tool surface, and the route that serves it.
///
/// The tool surface is part of the identity because tool definitions are prompt bytes: two runs
/// with the same prefix and route but a different tool manifest do not share a provider cache
/// entry. `None` means the surface is unknown, which never claims a match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheIdentity {
    /// The canonical prefix digest.
    pub prefix: PrefixDigest,
    /// The digest of the canonical tool surface, when the caller declares one.
    pub tools: Option<PrefixDigest>,
    /// The harness issuing the request.
    pub harness: HarnessId,
    /// The provider serving it.
    pub provider: ProviderId,
    /// The model, when named.
    pub model: Option<ModelId>,
    /// The reasoning effort, when the route has one.
    pub effort: Option<String>,
    /// The session key the request continues, when it continues one.
    pub session_key: Option<String>,
}

impl CacheIdentity {
    /// Build an identity.
    #[must_use]
    pub fn new(prefix: PrefixDigest, harness: HarnessId, provider: ProviderId) -> Self {
        Self {
            prefix,
            tools: None,
            harness,
            provider,
            model: None,
            effort: None,
            session_key: None,
        }
    }

    /// Set the model.
    #[must_use]
    pub fn with_model(mut self, model: ModelId) -> Self {
        self.model = Some(model);
        self
    }

    /// Set the effort.
    #[must_use]
    pub fn with_effort(mut self, effort: impl Into<String>) -> Self {
        self.effort = Some(effort.into());
        self
    }

    /// Set the continued session key.
    #[must_use]
    pub fn with_session(mut self, session_key: impl Into<String>) -> Self {
        self.session_key = Some(session_key.into());
        self
    }

    /// Set the tool-surface digest.
    #[must_use]
    pub fn with_tools(mut self, tools: PrefixDigest) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Set the tool-surface digest from a canonical manifest.
    #[must_use]
    pub fn with_tool_manifest(mut self, manifest: &crate::prefix::ToolManifest) -> Self {
        self.tools = Some(manifest.digest().clone());
        self
    }

    /// The identity as a stable digest over length-delimited fields.
    ///
    /// An absent tool surface and a declared one cannot collide: a declared digest is 64 hex
    /// characters and the absent field is empty.
    #[must_use]
    pub fn digest(&self) -> PrefixDigest {
        let normalize = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or_default()
                .to_owned()
        };
        let model = normalize(self.model.as_ref().map(ModelId::as_str));
        let effort = normalize(self.effort.as_deref());
        let session = normalize(self.session_key.as_deref());
        let tools = self
            .tools
            .as_ref()
            .map(PrefixDigest::as_str)
            .unwrap_or_default();
        digest(&[
            self.prefix.as_str().as_bytes(),
            tools.as_bytes(),
            self.harness.as_str().as_bytes(),
            self.provider.as_str().as_bytes(),
            model.as_bytes(),
            effort.as_bytes(),
            session.as_bytes(),
        ])
    }
}

/// The first request's break reason, when one can be attributed.
///
/// A changed tool surface is named first: it is the one break a caller can usually fix, and it
/// also moves the prefix. A moved prefix is `PrefixRewritten`; a run with no warm predecessor is
/// `ColdStart`; a warm request with an unchanged surface and prefix has no attributable break and
/// `None` is the honest answer.
#[must_use]
pub fn first_request_break(
    prefix_change: Option<PrefixChange>,
    tool_change: Option<PrefixChange>,
    cold_start: bool,
) -> Option<CacheBreakReason> {
    if tool_change == Some(PrefixChange::Changed) {
        Some(CacheBreakReason::ToolSurfaceChanged)
    } else if prefix_change == Some(PrefixChange::Changed) {
        Some(CacheBreakReason::PrefixRewritten)
    } else if cold_start {
        Some(CacheBreakReason::ColdStart)
    } else {
        None
    }
}

/// The recorded cold-start classification of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdStart {
    /// A captured session was resumed.
    Warm,
    /// No completed prior run exists for the same harness and workspace.
    NoPriorRun,
    /// The newest prior run is at or past the TTL: a **step**, not a decay curve.
    TtlExpired,
    /// A prior run was inside the window but no session was resumed.
    SessionNotReused,
}

impl ColdStart {
    /// Whether this classification records a cold run.
    #[must_use]
    pub const fn is_cold(self) -> bool {
        !matches!(self, ColdStart::Warm)
    }

    /// The stable spelling.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            ColdStart::Warm => "warm",
            ColdStart::NoPriorRun => "no_prior_run",
            ColdStart::TtlExpired => "ttl_expired",
            ColdStart::SessionNotReused => "session_not_reused",
        }
    }
}

/// Classify one run's warmth from the prior run's end and the declared TTL.
///
/// TTL is a **step**: at or past the expiry instant the session is refused, with no exponential
/// decay and no partial credit.
#[must_use]
pub fn classify_cold(
    now: EpochSeconds,
    prior_ended_at: Option<EpochSeconds>,
    ttl: Option<TtlSecs>,
    resumed: bool,
) -> ColdStart {
    if resumed {
        return ColdStart::Warm;
    }
    match (prior_ended_at, ttl) {
        (None, _) => ColdStart::NoPriorRun,
        (Some(ended), Some(ttl)) => {
            if now.get() >= ttl.after(ended).get() {
                ColdStart::TtlExpired
            } else {
                ColdStart::SessionNotReused
            }
        }
        (Some(_), None) => ColdStart::SessionNotReused,
    }
}

/// One classified run's cache observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColdObservation {
    /// The harness.
    pub harness: HarnessId,
    /// The classification, when known.
    pub class: Reported<ColdStart>,
    /// Cache-read tokens, when reported.
    pub cached: Reported<CachedTokens>,
    /// Fresh input tokens, when reported.
    pub fresh: Reported<FreshTokens>,
    /// First-turn fresh input, when reported.
    pub first_turn_fresh: Reported<FreshTokens>,
}

/// Token-weighted rollup for one harness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCacheRollup {
    /// The harness.
    pub harness: HarnessId,
    /// Classified cold runs.
    pub cold_runs: u64,
    /// Classified warm runs.
    pub warm_runs: u64,
    /// Runs without a classification; they enter neither ratio.
    pub unknown_runs: u64,
    /// Fresh input across cold runs, when any cold run reported usage.
    pub cold_fresh: Reported<FreshTokens>,
    /// Recorded first-turn fresh input, summed, when reported.
    pub first_turn_fresh: Reported<FreshTokens>,
    /// Token-weighted cache share across every classified run.
    pub cache_bps_including_cold: Option<BasisPoints>,
    /// Token-weighted cache share across warm runs only.
    pub cache_bps_excluding_cold: Option<BasisPoints>,
}

/// Sum tokens rather than averaging per-run percentages; unknown runs enter neither ratio.
#[must_use]
pub fn rollup(harness: HarnessId, observations: &[ColdObservation]) -> AgentCacheRollup {
    let mut row = AgentCacheRollup {
        harness,
        cold_runs: 0,
        warm_runs: 0,
        unknown_runs: 0,
        cold_fresh: Reported::NotReported,
        first_turn_fresh: Reported::NotReported,
        cache_bps_including_cold: None,
        cache_bps_excluding_cold: None,
    };
    let (mut fresh, mut cached, mut warm_fresh, mut warm_cached) = (0_u64, 0_u64, 0_u64, 0_u64);
    let (mut fresh_available, mut warm_available) = (false, false);
    for observation in observations {
        let Some(class) = observation.class.as_reported().copied() else {
            row.unknown_runs += 1;
            continue;
        };
        if class.is_cold() {
            row.cold_runs += 1;
        } else {
            row.warm_runs += 1;
        }
        if let Some(first) = observation.first_turn_fresh.as_reported().copied() {
            let total = row
                .first_turn_fresh
                .as_reported()
                .copied()
                .unwrap_or(FreshTokens::ZERO);
            row.first_turn_fresh =
                Reported::Reported(FreshTokens::new(total.saturating_add(first).get()));
        }
        let (Some(fee), Some(read)) = (
            observation.fresh.as_reported().copied(),
            observation.cached.as_reported().copied(),
        ) else {
            continue;
        };
        fresh = fresh.saturating_add(fee.get());
        cached = cached.saturating_add(read.get());
        fresh_available = true;
        if class.is_cold() {
            let cold = row
                .cold_fresh
                .as_reported()
                .copied()
                .unwrap_or(FreshTokens::ZERO);
            row.cold_fresh = Reported::Reported(FreshTokens::new(cold.saturating_add(fee).get()));
        } else {
            warm_fresh = warm_fresh.saturating_add(fee.get());
            warm_cached = warm_cached.saturating_add(read.get());
            warm_available = true;
        }
    }
    if fresh_available {
        row.cache_bps_including_cold =
            BasisPoints::from_ratio(u128::from(cached), u128::from(fresh) + u128::from(cached));
    }
    if warm_available {
        row.cache_bps_excluding_cold = BasisPoints::from_ratio(
            u128::from(warm_cached),
            u128::from(warm_fresh) + u128::from(warm_cached),
        );
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(name: &str) -> ProviderId {
        ProviderId::new(name).expect("valid provider")
    }

    fn model(name: &str) -> ModelId {
        ModelId::new(name).expect("valid model")
    }

    fn harness(name: &str) -> HarnessId {
        HarnessId::new(name).expect("valid harness")
    }

    #[test]
    fn a_declared_provider_gets_its_documented_numbers() {
        let claude = semantics(&provider("claude"), Some(&model("claude-sonnet-4")));
        assert_eq!(claude.mode, CacheMode::Marker);
        assert_eq!(
            claude.ttl,
            CacheTtl::Seconds(TtlSecs::new(300).expect("non-zero"))
        );
        assert_eq!(claude.read_multiplier, ratio(1, 10));
        assert_eq!(claude.write_multiplier, ratio(125, 100));
        assert_eq!(claude.minimum_tokens, Some(Tokens::new(1_024)));
        assert!(!claude.is_strict());

        let opus = semantics(&provider("anthropic"), Some(&model("claude-opus-4-1")));
        assert_eq!(opus.minimum_tokens, Some(Tokens::new(4_096)));

        let openai = semantics(&provider("openai"), Some(&model("gpt-5.6")));
        assert_eq!(openai.mode, CacheMode::Automatic);
        assert_eq!(
            openai.ttl,
            CacheTtl::Seconds(TtlSecs::new(1_800).expect("non-zero"))
        );
        assert_eq!(openai.long_ttl, TtlSecs::new(43_200));

        let gemini = semantics(&provider("gemini"), Some(&model("gemini-2.5-pro")));
        assert_eq!(gemini.mode, CacheMode::Automatic);
        assert_eq!(gemini.minimum_tokens, Some(Tokens::new(2_050)));
        assert!(gemini.explicit_legacy_only);
        assert_eq!(gemini.ttl, CacheTtl::Unknown);
    }

    #[test]
    fn unknown_routes_are_strict_and_the_provider_is_not_inferred_from_the_harness() {
        let unknown = semantics(&provider("agy"), Some(&model("whatever")));
        assert!(unknown.is_strict());
        assert_eq!(unknown.read_multiplier, ratio(1, 1));
        assert_eq!(
            unknown.reuse_net_saving(),
            Ratio::new(0, 1).expect("zero ratio")
        );
        let no_model = semantics(&provider("claude"), None);
        assert!(no_model.is_strict(), "an unaddressed route claims nothing");
    }

    #[test]
    fn the_reuse_counterfactual_is_write_minus_read() {
        let claude = semantics(&provider("claude"), Some(&model("claude-sonnet-4")));
        let net = claude.reuse_net_saving();
        assert_eq!(net, Ratio::new(115, 100).expect("non-zero denominator"));
        assert!((net.as_f64() - 1.15).abs() < f64::EPSILON);
        assert_eq!(
            claude.read_saving(),
            Ratio::new(9, 10).expect("non-zero denominator")
        );
    }

    #[test]
    fn an_identity_digest_changes_with_every_field() {
        let prefix = crate::prefix::digest(&[b"prefix"]);
        let base = CacheIdentity::new(prefix.clone(), harness("codex"), provider("openai"));
        let digest = base.digest();
        assert_eq!(base.clone().digest(), digest, "stable");
        assert_ne!(base.clone().with_model(model("m")).digest(), digest);
        assert_ne!(base.clone().with_effort("high").digest(), digest);
        assert_ne!(base.clone().with_session("s").digest(), digest);
        assert_ne!(
            base.with_tools(crate::prefix::digest(&[b"tools"])).digest(),
            digest,
            "the tool surface is part of the cache identity"
        );
    }

    #[test]
    fn a_declared_tool_surface_matches_its_manifest_digest() {
        let prefix = crate::prefix::digest(&[b"prefix"]);
        let manifest = crate::prefix::ToolManifest::new(&[
            crate::prefix::ToolEntry::new("read", "read a file"),
            crate::prefix::ToolEntry::new("edit", "edit a file"),
        ])
        .expect("distinct names");
        let identity = CacheIdentity::new(prefix.clone(), harness("codex"), provider("openai"))
            .with_tool_manifest(&manifest);
        assert_eq!(identity.tools.as_ref(), Some(manifest.digest()));
        assert_eq!(
            identity.digest(),
            CacheIdentity::new(prefix, harness("codex"), provider("openai"))
                .with_tools(manifest.digest().clone())
                .digest()
        );
    }

    #[test]
    fn a_first_request_break_is_attributed_only_when_it_can_be() {
        assert_eq!(
            first_request_break(Some(PrefixChange::Changed), None, false),
            Some(CacheBreakReason::PrefixRewritten)
        );
        assert_eq!(
            first_request_break(Some(PrefixChange::Unchanged), None, true),
            Some(CacheBreakReason::ColdStart)
        );
        assert_eq!(
            first_request_break(Some(PrefixChange::Unchanged), None, false),
            None
        );
        assert_eq!(
            first_request_break(Some(PrefixChange::FirstRun), None, true),
            Some(CacheBreakReason::ColdStart)
        );
    }

    #[test]
    fn a_tool_surface_change_outranks_a_prefix_rewrite_and_a_cold_start() {
        assert_eq!(
            first_request_break(
                Some(PrefixChange::Changed),
                Some(PrefixChange::Changed),
                true
            ),
            Some(CacheBreakReason::ToolSurfaceChanged),
            "a tool change is the one break a caller can fix"
        );
        assert_eq!(
            first_request_break(None, Some(PrefixChange::Changed), false),
            Some(CacheBreakReason::ToolSurfaceChanged)
        );
        assert_eq!(
            first_request_break(
                Some(PrefixChange::Unchanged),
                Some(PrefixChange::Unchanged),
                false
            ),
            None
        );
    }

    #[test]
    fn a_cold_classification_treats_the_ttl_as_a_step() {
        let ttl = TtlSecs::new(600).expect("non-zero");
        let ended = EpochSeconds::new(1_000);
        assert_eq!(
            classify_cold(EpochSeconds::new(1_100), Some(ended), Some(ttl), false),
            ColdStart::SessionNotReused
        );
        assert_eq!(
            classify_cold(EpochSeconds::new(1_600), Some(ended), Some(ttl), false),
            ColdStart::TtlExpired,
            "exactly at expiry is expired: a step, not a decay curve"
        );
        assert_eq!(
            classify_cold(EpochSeconds::new(1_700), Some(ended), Some(ttl), true),
            ColdStart::Warm,
            "a resumed session is warm whatever the age"
        );
        assert_eq!(
            classify_cold(EpochSeconds::new(1_700), None, Some(ttl), false),
            ColdStart::NoPriorRun
        );
    }

    #[test]
    fn a_rollup_sums_tokens_and_never_averages_percentages() {
        let warm = ColdObservation {
            harness: harness("codex"),
            class: Reported::Reported(ColdStart::Warm),
            cached: Reported::Reported(CachedTokens::new(9_000)),
            fresh: Reported::Reported(FreshTokens::new(1_000)),
            first_turn_fresh: Reported::Reported(FreshTokens::new(900)),
        };
        let cold = ColdObservation {
            harness: harness("codex"),
            class: Reported::Reported(ColdStart::NoPriorRun),
            cached: Reported::Reported(CachedTokens::new(0)),
            fresh: Reported::Reported(FreshTokens::new(1_000)),
            first_turn_fresh: Reported::Reported(FreshTokens::new(1_000)),
        };
        let unknown = ColdObservation {
            harness: harness("codex"),
            class: Reported::NotReported,
            cached: Reported::NotReported,
            fresh: Reported::NotReported,
            first_turn_fresh: Reported::NotReported,
        };
        let row = rollup(harness("codex"), &[warm, cold, unknown]);
        assert_eq!(row.warm_runs, 1);
        assert_eq!(row.cold_runs, 1);
        assert_eq!(row.unknown_runs, 1);
        assert_eq!(
            row.cache_bps_including_cold,
            BasisPoints::new(8_182),
            "pooled 9000/11000, not the mean of 90% and 0%"
        );
        assert_eq!(row.cache_bps_excluding_cold, BasisPoints::new(9_000));
        assert_eq!(
            row.cold_fresh.as_reported().copied(),
            Some(FreshTokens::new(1_000))
        );
        assert_eq!(
            row.first_turn_fresh.as_reported().copied(),
            Some(FreshTokens::new(1_900))
        );
    }
}
