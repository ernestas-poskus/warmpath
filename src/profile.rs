//! Harness profiles: the per-CLI facts a cache analysis needs, declared once.
//!
//! A profile says **where** a harness prints its usage, **what shape** the input count has,
//! **which** cache contract its provider has, **how** a session is captured and resumed, and
//! **how long** a warm session is expected to last. Nothing is inferred from the harness name at
//! runtime: the provider is an explicit field, because a harness may front several providers and
//! a gateway may rewrite requests. A profile that does not declare a fact leaves it absent rather
//! than guessing.
//!
//! The built-ins ([`HarnessProfile::codex`], [`HarnessProfile::claude`],
//! [`HarnessProfile::agy`], [`HarnessProfile::dsh`], [`HarnessProfile::grok`]) are versioned data:
//! changing one is a [`ProfileVersion`] bump, because runs recorded against the old facts must stay
//! interpretable.

use crate::newtypes::{
    HarnessId, HarnessVersion, ModelId, ProfileId, ProfileVersion, ProviderId, TtlSecs,
};

/// The schema version of the built-in profiles in this build.
pub const PROFILE_SCHEMA_VERSION: ProfileVersion = ProfileVersion::new(1);

/// How a harness writes its output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputShape {
    /// Plain text with no machine-readable usage.
    Text,
    /// One JSON document for the whole run.
    Json,
    /// One JSON document per line.
    Jsonl,
}

/// What a provider's reported input count includes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputShape {
    /// The input count includes cache reads (the read subset is hidden inside it).
    Inclusive,
    /// The input count is fresh input; cache reads are reported separately.
    Exclusive,
    /// Fresh, read, and write counts are separate partitions of prompt work.
    Partitioned,
}

/// The provider's cache mechanism class, as the provider documents it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CacheClass {
    /// The provider caches repeated prefixes automatically.
    Automatic,
    /// The client must place an explicit cache marker.
    Marker,
    /// The client selects explicit breakpoints, and the provider caches up to them.
    ExplicitBreakpoint,
    /// No declared contract: claim nothing and behave append-only.
    Strict,
}

/// Ordered usage paths per reported field.
///
/// Each vector is tried in order; the first path that resolves in the last document that carries
/// any usage wins. An empty vector means "this harness does not report it".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsagePaths {
    /// Paths to the input-token count.
    pub input: Vec<String>,
    /// Paths to the output-token count.
    pub output: Vec<String>,
    /// Paths to the cache-read count.
    pub cache_read: Vec<String>,
    /// Paths to the cache-write (cache creation) count.
    pub cache_write: Vec<String>,
    /// Paths to the reasoning-token count.
    pub reasoning: Vec<String>,
}

impl UsagePaths {
    /// Build a set of paths from ordered candidates.
    #[must_use]
    pub fn new(
        input: &[&str],
        output: &[&str],
        cache_read: &[&str],
        cache_write: &[&str],
        reasoning: &[&str],
    ) -> Self {
        Self {
            input: input.iter().map(|path| (*path).to_owned()).collect(),
            output: output.iter().map(|path| (*path).to_owned()).collect(),
            cache_read: cache_read.iter().map(|path| (*path).to_owned()).collect(),
            cache_write: cache_write.iter().map(|path| (*path).to_owned()).collect(),
            reasoning: reasoning.iter().map(|path| (*path).to_owned()).collect(),
        }
    }

    /// No paths at all.
    #[must_use]
    pub fn none() -> Self {
        Self {
            input: Vec::new(),
            output: Vec::new(),
            cache_read: Vec::new(),
            cache_write: Vec::new(),
            reasoning: Vec::new(),
        }
    }
}

/// The event shape of a harness that reports usage per step rather than once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepEventMatch {
    /// Path to the event's type field.
    pub type_path: String,
    /// The type value that marks a step boundary.
    pub type_value: String,
    /// Path to the event's phase field.
    pub phase_path: String,
    /// The phase value that marks the end of a step.
    pub phase_value: String,
}

/// A harness's declared version range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRange {
    /// The oldest version this profile was written for.
    pub min: Option<HarnessVersion>,
    /// The newest version it was written for, when bounded.
    pub max: Option<HarnessVersion>,
}

impl VersionRange {
    /// Every version.
    #[must_use]
    pub const fn any() -> Self {
        Self {
            min: None,
            max: None,
        }
    }

    /// A range from `min` to an unbounded maximum.
    #[must_use]
    pub const fn at_least(min: HarnessVersion) -> Self {
        Self {
            min: Some(min),
            max: None,
        }
    }

    /// Whether `version` is inside the range, comparing dotted numeric components when possible.
    ///
    /// A non-numeric version is compared as opaque text; an unknown version is *not* a reason to
    /// guess, so callers should report [`crate::error::Error::Unsupported`] when they need to.
    #[must_use]
    pub fn supports(&self, version: &HarnessVersion) -> bool {
        if let Some(min) = &self.min
            && compare_versions(version.as_str(), min.as_str()) == std::cmp::Ordering::Less
        {
            return false;
        }
        if let Some(max) = &self.max
            && compare_versions(version.as_str(), max.as_str()) == std::cmp::Ordering::Greater
        {
            return false;
        }
        true
    }
}

/// Compare two dotted versions component by component, numerically when both are numbers.
fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    let left: Vec<&str> = left.split('.').collect();
    let right: Vec<&str> = right.split('.').collect();
    for (a, b) in left.iter().zip(right.iter()) {
        let ordering = match (a.parse::<u64>(), b.parse::<u64>()) {
            (Ok(a), Ok(b)) => a.cmp(&b),
            _ => a.cmp(b),
        };
        if ordering != std::cmp::Ordering::Equal {
            return ordering;
        }
    }
    left.len().cmp(&right.len())
}

/// Everything the cache analysis knows about one harness build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessProfile {
    /// The profile's identity.
    pub profile_id: ProfileId,
    /// The profile's schema version.
    pub profile_version: ProfileVersion,
    /// The harness this profile describes.
    pub harness: HarnessId,
    /// The provider this profile is written for. **Never** inferred from the harness name.
    pub provider: ProviderId,
    /// The provider's cache mechanism class.
    pub cache_class: CacheClass,
    /// How the harness writes its output.
    pub output_shape: OutputShape,
    /// What the reported input count includes.
    pub input_shape: InputShape,
    /// Ordered usage paths.
    pub usage_paths: UsagePaths,
    /// Per-step event matching, when the harness reports usage incrementally.
    pub step_event: Option<StepEventMatch>,
    /// The harness versions this profile was written for.
    pub version_range: VersionRange,
    /// Whether the harness reports a conversation's **running total** rather than this attempt.
    ///
    /// Codex's `turn.completed.usage` accumulates across a resumed thread, so its parsed number is
    /// a baseline-subtractable total; every other profile reports per-attempt numbers.
    pub cumulative_usage: bool,
    /// Paths a session identifier may appear at in the output.
    pub session_paths: Vec<String>,
    /// Argv template that resumes a captured session; `{session}` is substituted.
    pub resume_argv: Vec<String>,
    /// Argv template that assigns a session id to a fresh run; `{session}` is substituted.
    pub assign_argv: Vec<String>,
    /// Whether a fresh run may be handed a caller-chosen conversation id.
    pub assignable: bool,
    /// The provider cache lifetime this profile expects, when known.
    pub warm_ttl: Option<TtlSecs>,
}

impl HarnessProfile {
    /// The Codex profile: OpenAI automatic prefix caching, cumulative JSONL usage.
    #[must_use]
    pub fn codex() -> Self {
        Self {
            profile_id: ProfileId::new("codex-v1").expect("static profile id"),
            profile_version: PROFILE_SCHEMA_VERSION,
            harness: HarnessId::new("codex").expect("static harness id"),
            provider: ProviderId::new("openai").expect("static provider id"),
            cache_class: CacheClass::Automatic,
            output_shape: OutputShape::Jsonl,
            input_shape: InputShape::Inclusive,
            usage_paths: UsagePaths::new(
                &["info.last_token_usage.input_tokens", "usage.input_tokens"],
                &["info.last_token_usage.output_tokens", "usage.output_tokens"],
                &[
                    "info.last_token_usage.cached_input_tokens",
                    "usage.cached_input_tokens",
                ],
                &["usage.cache_write_tokens"],
                &[
                    "usage.output_tokens_details.reasoning_tokens",
                    "info.last_token_usage.reasoning_output_tokens",
                ],
            ),
            step_event: None,
            version_range: VersionRange::at_least(
                HarnessVersion::new("0.20.0").expect("static version"),
            ),
            cumulative_usage: true,
            session_paths: vec!["thread_id".to_owned(), "threadId".to_owned()],
            resume_argv: vec![
                "exec".to_owned(),
                "resume".to_owned(),
                "{session}".to_owned(),
            ],
            assign_argv: Vec::new(),
            assignable: false,
            warm_ttl: TtlSecs::new(600),
        }
    }

    /// The Claude Code profile: Anthropic's explicit marker cache, JSON usage.
    #[must_use]
    pub fn claude() -> Self {
        Self {
            profile_id: ProfileId::new("claude-v1").expect("static profile id"),
            profile_version: PROFILE_SCHEMA_VERSION,
            harness: HarnessId::new("claude").expect("static harness id"),
            provider: ProviderId::new("claude").expect("static provider id"),
            cache_class: CacheClass::Marker,
            output_shape: OutputShape::Json,
            input_shape: InputShape::Exclusive,
            usage_paths: UsagePaths::new(
                &["usage.input_tokens"],
                &["usage.output_tokens"],
                &["usage.cache_read_input_tokens", "usage.cached_input_tokens"],
                &[
                    "usage.cache_creation_input_tokens",
                    "usage.cache_creation_tokens",
                ],
                &[
                    "usage.output_tokens_details.reasoning_tokens",
                    "usage.reasoning_tokens",
                ],
            ),
            step_event: None,
            version_range: VersionRange::at_least(
                HarnessVersion::new("1.0.0").expect("static version"),
            ),
            cumulative_usage: false,
            session_paths: vec![
                "session_id".to_owned(),
                "sessionId".to_owned(),
                "session.id".to_owned(),
            ],
            resume_argv: vec!["--resume".to_owned(), "{session}".to_owned()],
            assign_argv: vec!["--session-id".to_owned(), "{session}".to_owned()],
            assignable: true,
            warm_ttl: TtlSecs::new(300),
        }
    }

    /// The Antigravity (`agy`) profile: a gateway in front of several model families.
    ///
    /// The provider is the gateway's own name, which resolves to the strict cache contract: a
    /// gateway may rewrite the request, so no prefix reuse is claimed.
    #[must_use]
    pub fn agy() -> Self {
        Self {
            profile_id: ProfileId::new("agy-v2").expect("static profile id"),
            profile_version: PROFILE_SCHEMA_VERSION,
            harness: HarnessId::new("agy").expect("static harness id"),
            provider: ProviderId::new("agy").expect("static provider id"),
            cache_class: CacheClass::Strict,
            output_shape: OutputShape::Json,
            input_shape: InputShape::Exclusive,
            usage_paths: UsagePaths::new(
                &["usage.input_tokens"],
                &["usage.output_tokens"],
                &[
                    "usage.cache_read_tokens",
                    "usage.cached_input_tokens",
                    "usage.cache_read_input_tokens",
                ],
                &["usage.cache_creation_tokens"],
                &["usage.thinking_tokens", "usage.reasoning_tokens"],
            ),
            step_event: None,
            version_range: VersionRange::any(),
            cumulative_usage: false,
            session_paths: vec!["conversation_id".to_owned(), "conversationId".to_owned()],
            resume_argv: vec!["--conversation".to_owned(), "{session}".to_owned()],
            assign_argv: Vec::new(),
            assignable: false,
            warm_ttl: TtlSecs::new(300),
        }
    }

    /// The DSH profile: per-step JSONL accounting.
    #[must_use]
    pub fn dsh() -> Self {
        Self {
            profile_id: ProfileId::new("dsh-v1").expect("static profile id"),
            profile_version: PROFILE_SCHEMA_VERSION,
            harness: HarnessId::new("dsh").expect("static harness id"),
            provider: ProviderId::new("dsh").expect("static provider id"),
            cache_class: CacheClass::Strict,
            output_shape: OutputShape::Jsonl,
            input_shape: InputShape::Exclusive,
            usage_paths: UsagePaths::new(
                &["usage.inputTokens"],
                &["usage.outputTokens"],
                &["usage.cacheReadTokens"],
                &["usage.cacheWriteTokens"],
                &["usage.reasoningTokens"],
            ),
            step_event: Some(StepEventMatch {
                type_path: "type".to_owned(),
                type_value: "status".to_owned(),
                phase_path: "phase".to_owned(),
                phase_value: "step_end".to_owned(),
            }),
            version_range: VersionRange::any(),
            cumulative_usage: false,
            session_paths: vec!["session_id".to_owned(), "sessionId".to_owned()],
            resume_argv: vec!["--resume".to_owned(), "{session}".to_owned()],
            assign_argv: Vec::new(),
            assignable: false,
            warm_ttl: TtlSecs::new(300),
        }
    }

    /// The Grok profile: no resume form and no declared cache lifetime.
    #[must_use]
    pub fn grok() -> Self {
        Self {
            profile_id: ProfileId::new("grok-v2").expect("static profile id"),
            profile_version: PROFILE_SCHEMA_VERSION,
            harness: HarnessId::new("grok").expect("static harness id"),
            provider: ProviderId::new("grok").expect("static provider id"),
            cache_class: CacheClass::Automatic,
            output_shape: OutputShape::Json,
            input_shape: InputShape::Inclusive,
            usage_paths: UsagePaths::new(
                &["usage.prompt_tokens", "usage.input_tokens"],
                &["usage.completion_tokens", "usage.output_tokens"],
                &[
                    "usage.prompt_tokens_details.cached_tokens",
                    "usage.input_tokens_details.cached_tokens",
                ],
                &[],
                &[
                    "usage.completion_tokens_details.reasoning_tokens",
                    "usage.output_tokens_details.reasoning_tokens",
                ],
            ),
            step_event: None,
            version_range: VersionRange::any(),
            cumulative_usage: false,
            session_paths: vec!["session_id".to_owned()],
            resume_argv: Vec::new(),
            assign_argv: Vec::new(),
            assignable: false,
            warm_ttl: None,
        }
    }

    /// Every built-in profile, in a stable order.
    #[must_use]
    pub fn builtin() -> Vec<HarnessProfile> {
        vec![
            Self::codex(),
            Self::claude(),
            Self::agy(),
            Self::dsh(),
            Self::grok(),
        ]
    }

    /// The built-in profile for a harness, when this build declares one.
    #[must_use]
    pub fn for_harness(harness: &HarnessId) -> Option<HarnessProfile> {
        Self::builtin()
            .into_iter()
            .find(|profile| &profile.harness == harness)
    }

    /// Start a custom profile for a caller-declared harness.
    pub fn custom(
        profile_id: &str,
        harness: &str,
        provider: &str,
    ) -> Result<HarnessProfileBuilder, crate::error::Error> {
        Ok(HarnessProfileBuilder {
            profile: Self {
                profile_id: ProfileId::new(profile_id)?,
                profile_version: PROFILE_SCHEMA_VERSION,
                harness: HarnessId::new(harness)?,
                provider: ProviderId::new(provider)?,
                cache_class: CacheClass::Strict,
                output_shape: OutputShape::Json,
                input_shape: InputShape::Exclusive,
                usage_paths: UsagePaths::none(),
                step_event: None,
                version_range: VersionRange::any(),
                cumulative_usage: false,
                session_paths: Vec::new(),
                resume_argv: Vec::new(),
                assign_argv: Vec::new(),
                assignable: false,
                warm_ttl: None,
            },
        })
    }

    /// Whether this profile's declared range accepts `version`.
    #[must_use]
    pub fn supports_version(&self, version: &HarnessVersion) -> bool {
        self.version_range.supports(version)
    }

    /// The declared provider/model route as `provider/model`, when a model is named.
    #[must_use]
    pub fn route(&self, model: &ModelId) -> String {
        format!("{}/{}", self.provider, model)
    }
}

/// A builder for a caller-declared profile.
#[derive(Debug, Clone)]
pub struct HarnessProfileBuilder {
    profile: HarnessProfile,
}

impl HarnessProfileBuilder {
    /// Set the cache class.
    #[must_use]
    pub fn cache_class(mut self, class: CacheClass) -> Self {
        self.profile.cache_class = class;
        self
    }

    /// Set the output shape.
    #[must_use]
    pub fn output_shape(mut self, shape: OutputShape) -> Self {
        self.profile.output_shape = shape;
        self
    }

    /// Set the input shape.
    #[must_use]
    pub fn input_shape(mut self, shape: InputShape) -> Self {
        self.profile.input_shape = shape;
        self
    }

    /// Replace the usage paths.
    #[must_use]
    pub fn usage_paths(mut self, paths: UsagePaths) -> Self {
        self.profile.usage_paths = paths;
        self
    }

    /// Set session-identifier paths.
    #[must_use]
    pub fn session_paths(mut self, paths: &[&str]) -> Self {
        self.profile.session_paths = paths.iter().map(|path| (*path).to_owned()).collect();
        self
    }

    /// Set the resume argv template.
    #[must_use]
    pub fn resume_argv(mut self, argv: &[&str]) -> Self {
        self.profile.resume_argv = argv.iter().map(|arg| (*arg).to_owned()).collect();
        self
    }

    /// Set the assign argv template and mark the profile assignable.
    #[must_use]
    pub fn assign_argv(mut self, argv: &[&str]) -> Self {
        self.profile.assign_argv = argv.iter().map(|arg| (*arg).to_owned()).collect();
        self.profile.assignable = true;
        self
    }

    /// Set the version range.
    #[must_use]
    pub fn version_range(mut self, range: VersionRange) -> Self {
        self.profile.version_range = range;
        self
    }

    /// Declare that the harness reports a running total.
    #[must_use]
    pub fn cumulative_usage(mut self, cumulative: bool) -> Self {
        self.profile.cumulative_usage = cumulative;
        self
    }

    /// Set the expected warm lifetime.
    #[must_use]
    pub fn warm_ttl(mut self, ttl: TtlSecs) -> Self {
        self.profile.warm_ttl = Some(ttl);
        self
    }

    /// Finish the profile.
    #[must_use]
    pub fn build(self) -> HarnessProfile {
        self.profile
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_is_self_consistent() {
        for profile in HarnessProfile::builtin() {
            assert!(!profile.harness.as_str().is_empty());
            assert!(!profile.provider.as_str().is_empty());
            assert!(
                !profile.usage_paths.input.is_empty(),
                "{} must say where input tokens are",
                profile.harness
            );
            assert!(
                !profile.usage_paths.output.is_empty(),
                "{} must say where output tokens are",
                profile.harness
            );
            assert_eq!(
                HarnessProfile::for_harness(&profile.harness)
                    .expect("builtin lookup")
                    .profile_id,
                profile.profile_id
            );
        }
    }

    #[test]
    fn a_provider_is_data_not_a_guess_from_the_harness() {
        let codex = HarnessProfile::codex();
        assert_eq!(codex.provider.as_str(), "openai");
        let custom = HarnessProfile::custom("custom-v1", "codex", "some-gateway")
            .expect("valid ids")
            .build();
        assert_eq!(custom.provider.as_str(), "some-gateway");
    }

    #[test]
    fn a_version_range_compares_dotted_numbers() {
        let range = VersionRange::at_least(HarnessVersion::new("1.10.0").expect("valid"));
        assert!(!range.supports(&HarnessVersion::new("1.9.9").expect("valid")));
        assert!(range.supports(&HarnessVersion::new("1.10.0").expect("valid")));
        assert!(range.supports(&HarnessVersion::new("2.0.0").expect("valid")));
        assert!(VersionRange::any().supports(&HarnessVersion::new("weird").expect("valid")));
    }

    #[test]
    fn a_grok_profile_matches_the_documented_xai_usage_fields() {
        let grok = HarnessProfile::grok();
        assert_eq!(grok.profile_id.as_str(), "grok-v2");
        assert_eq!(grok.input_shape, InputShape::Inclusive);
        assert_eq!(grok.cache_class, CacheClass::Automatic);
        assert!(
            grok.usage_paths
                .cache_read
                .iter()
                .any(|path| path == "usage.prompt_tokens_details.cached_tokens"),
            "xAI nests the cached count under prompt_tokens_details"
        );
        assert!(
            grok.usage_paths.cache_write.is_empty(),
            "xAI documents no cache-write field"
        );
        assert_eq!(grok.warm_ttl, None, "no cache TTL is documented");
    }

    #[test]
    fn an_agy_profile_resumes_with_its_own_dialect() {
        let agy = HarnessProfile::agy();
        assert_eq!(agy.profile_id.as_str(), "agy-v2");
        assert_eq!(agy.resume_argv, vec!["--conversation", "{session}"]);
        assert_eq!(
            agy.usage_paths.reasoning.first().map(String::as_str),
            Some("usage.thinking_tokens")
        );
        assert_eq!(agy.provider.as_str(), "agy", "the gateway stays strict");
    }

    #[test]
    fn the_builtin_profiles_declare_the_documented_lifetimes() {
        assert_eq!(HarnessProfile::claude().warm_ttl, TtlSecs::new(300));
        assert_eq!(HarnessProfile::codex().warm_ttl, TtlSecs::new(600));
        assert_eq!(HarnessProfile::grok().warm_ttl, None);
    }
}
