//! Warm-session capture, reuse decisions, and the resume counterfactual.
//!
//! A warm session is a conversation the harness can resume, whose provider-side prefix may still be
//! cached. Reuse is a **decision with evidence**, never an assumption:
//!
//! - the profile must declare a resume argv ([`can_resume`]);
//! - a captured session key must be usable ([`usable_prefix`]);
//! - the candidate's identity (subject, revision, workspace, harness, provider, model, effort,
//!   prefix) must match the request — a mismatch is named, not silently skipped;
//! - the candidate must be younger than the declared TTL. The TTL is a **step**: at the expiry
//!   instant the session is refused. There is no exponential decay in the decision;
//! - the declared-file overlap must meet the caller's floor.
//!
//! The expected saving is a counterfactual: `cached × overlap × (w - r)` — the prefix tokens a warm
//! resume continues, times the share of files that genuinely overlap, times the difference between
//! rewriting the cached prefix and reading it. It is an estimate, named as one, and it saturates at
//! zero rather than inventing a negative saving.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::cache::CacheContract;
use crate::error::Error;
use crate::newtypes::{
    BasisPoints, CachedTokens, EpochSeconds, FreshTokens, HarnessId, ModelId, PrefixDigest,
    ProviderId, RevisionId, SubjectId, Tokens, TtlSecs, WarmAgeSecs, warm_age,
};
use crate::profile::HarnessProfile;
use crate::reported::Reported;

/// The placeholder a resume or assign template uses for the session id.
pub const SESSION_PLACEHOLDER: &str = "{session}";

/// The placeholder a prepare template uses for the prompt.
pub const PROMPT_PLACEHOLDER: &str = "{prompt}";

/// What a run's output says about its session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCapture {
    /// The session key, when the output named a usable one.
    pub session_key: Option<String>,
}

/// Capture the session key a harness printed.
///
/// Scans the output's JSON documents for the profile's session paths and returns the last
/// non-empty trimmed value. `None` is normal: not every harness prints a session id, and a failed
/// run may not reach the point where it would.
#[must_use]
pub fn capture_session(profile: &HarnessProfile, bytes: &[u8]) -> Option<String> {
    crate::usage::session_key(profile, bytes)
}

/// Whether the profile declares a resume form.
#[must_use]
pub fn can_resume(profile: &HarnessProfile) -> bool {
    !profile.resume_argv.is_empty()
}

/// Whether a fresh run may be handed a caller-chosen session id.
#[must_use]
pub fn assignable(profile: &HarnessProfile) -> bool {
    profile.assignable && !profile.assign_argv.is_empty()
}

/// Whether a captured key is worth reusing.
///
/// A too-short key (fewer than eight characters), an all-placeholder key, or one containing
/// whitespace is not a conversation id: reusing it would resume the wrong conversation or fail.
#[must_use]
pub fn usable_prefix(key: &str) -> bool {
    let key = key.trim();
    key.len() >= 8 && !key.chars().any(char::is_whitespace) && key != "unknown"
}

/// The short form of a session key for a log line.
#[must_use]
pub fn short(key: &str) -> &str {
    let end = key
        .char_indices()
        .nth(8)
        .map_or(key.len(), |(index, _)| index);
    &key[..end]
}

/// The inputs a caller supplies for one invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationContext {
    /// The harness binary.
    pub binary: String,
    /// Arguments for a fresh invocation.
    pub args: Vec<String>,
    /// The prompt text, substituted for [`PROMPT_PLACEHOLDER`] when a template uses it.
    pub prompt: Option<String>,
}

impl InvocationContext {
    /// Build a context.
    #[must_use]
    pub fn new(binary: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            binary: binary.into(),
            args,
            prompt: None,
        }
    }

    /// Attach a prompt.
    #[must_use]
    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }
}

/// The argv a run should execute, and the session id this crate assigned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInvocation {
    /// The full argv, binary first.
    pub argv: Vec<String>,
    /// The session id assigned to a fresh assignable run, or `None`.
    pub assigned: Option<String>,
}

/// Build the argv for a fresh or resumed invocation.
///
/// A resumed invocation renders the profile's resume template with the session substituted; a fresh
/// invocation uses the context's args, and an assignable profile receives a caller-chosen session id
/// through its assign template. A template placeholder with no value is [`Error::InvalidTarget`]:
/// running a template with a literal `{session}` would resume nothing.
///
/// # Errors
///
/// [`Error::InvalidName`] when a non-empty `resume`/`assign` value is unusable, or
/// [`Error::InvalidTarget`] when a template needs a value the context does not provide.
pub fn prepare(
    profile: &HarnessProfile,
    context: &InvocationContext,
    resume: Option<&str>,
    assign: Option<&str>,
) -> Result<PreparedInvocation, Error> {
    if let Some(resume) = resume
        && !usable_prefix(resume)
    {
        return Err(Error::InvalidName {
            kind: "session key",
            value: resume.to_owned(),
            detail: "is not a usable session key",
        });
    }
    let mut argv = vec![context.binary.clone()];
    let mut assigned = None;
    match resume {
        Some(session) if can_resume(profile) => {
            for arg in &profile.resume_argv {
                argv.push(substitute(profile, arg, session, context)?);
            }
        }
        _ => {
            argv.extend(context.args.iter().cloned());
            if let Some(session) = assign
                && assignable(profile)
            {
                if !usable_prefix(session) {
                    return Err(Error::InvalidName {
                        kind: "session key",
                        value: session.to_owned(),
                        detail: "is not a usable session key",
                    });
                }
                for arg in &profile.assign_argv {
                    argv.push(substitute(profile, arg, session, context)?);
                }
                assigned = Some(session.to_owned());
            }
        }
    }
    Ok(PreparedInvocation { argv, assigned })
}

/// Substitute a template placeholder, refusing to leave a literal placeholder in an argv.
fn substitute(
    profile: &HarnessProfile,
    template: &str,
    session: &str,
    context: &InvocationContext,
) -> Result<String, Error> {
    if template.contains(PROMPT_PLACEHOLDER) {
        return match context.prompt.as_deref() {
            Some(prompt) => Ok(template.replace(PROMPT_PLACEHOLDER, prompt)),
            None => Err(Error::InvalidTarget {
                detail: format!("the `{}` resume template needs a prompt", profile.harness),
            }),
        };
    }
    Ok(template.replace(SESSION_PLACEHOLDER, session))
}

/// The instant after which a session at `ended_at` is no longer warm.
#[must_use]
pub fn warm_expires_at(ended_at: EpochSeconds, ttl: TtlSecs) -> EpochSeconds {
    ttl.after(ended_at)
}

/// Seconds between `now` and `ended_at`; an ended-at in the future is zero.
#[must_use]
pub fn warm_age_secs(now: EpochSeconds, ended_at: EpochSeconds) -> WarmAgeSecs {
    warm_age(now, ended_at)
}

/// The profile's declared warm TTL, when it has one.
#[must_use]
pub fn default_warm_ttl(profile: &HarnessProfile) -> Option<TtlSecs> {
    profile.warm_ttl
}

/// A recorded warm candidate: the session, the identity it was captured under, and its usage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WarmRecord {
    /// The subject the work belonged to.
    pub subject: SubjectId,
    /// The revision of that subject.
    pub revision: RevisionId,
    /// The workspace (opaque caller-chosen text).
    pub workspace: String,
    /// The harness that produced the session.
    pub harness: HarnessId,
    /// The provider it ran against.
    pub provider: ProviderId,
    /// The model, when named.
    #[serde(default)]
    pub model: Option<ModelId>,
    /// The effort, when the route had one.
    #[serde(default)]
    pub effort: Option<String>,
    /// The canonical prefix digest at the end of the run.
    pub prefix: PrefixDigest,
    /// The captured session key.
    #[serde(default)]
    pub session_key: Option<String>,
    /// When the run ended.
    pub ended_at: EpochSeconds,
    /// Cache-read tokens the run reported.
    pub cached: CachedTokens,
    /// Fresh input tokens the run reported.
    pub fresh: FreshTokens,
    /// The cumulative baseline a resumed cumulative harness should subtract, when recorded.
    #[serde(default)]
    pub cumulative: Option<crate::usage::UsageTotals>,
    /// The declared files the run's work touched, for the overlap score.
    #[serde(default)]
    pub files: Vec<String>,
}

impl WarmRecord {
    /// Build a record.
    #[must_use]
    pub fn new(
        subject: SubjectId,
        revision: RevisionId,
        workspace: impl Into<String>,
        harness: HarnessId,
        provider: ProviderId,
        prefix: PrefixDigest,
        ended_at: EpochSeconds,
    ) -> Self {
        Self {
            subject,
            revision,
            workspace: workspace.into(),
            harness,
            provider,
            model: None,
            effort: None,
            prefix,
            session_key: None,
            ended_at,
            cached: CachedTokens::ZERO,
            fresh: FreshTokens::ZERO,
            cumulative: None,
            files: Vec::new(),
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

    /// Set the session key.
    #[must_use]
    pub fn with_session(mut self, key: impl Into<String>) -> Self {
        self.session_key = Some(key.into());
        self
    }

    /// Set the recorded tokens.
    #[must_use]
    pub fn with_usage(mut self, cached: CachedTokens, fresh: FreshTokens) -> Self {
        self.cached = cached;
        self.fresh = fresh;
        self
    }

    /// Set the cumulative baseline.
    #[must_use]
    pub fn with_cumulative(mut self, cumulative: crate::usage::UsageTotals) -> Self {
        self.cumulative = Some(cumulative);
        self
    }

    /// Set the declared files.
    #[must_use]
    pub fn with_files(mut self, files: Vec<String>) -> Self {
        self.files = files;
        self
    }

    /// The age of this record at `now`.
    #[must_use]
    pub fn age(&self, now: EpochSeconds) -> WarmAgeSecs {
        warm_age(now, self.ended_at)
    }

    /// Whether the record is inside a TTL at `now`.
    #[must_use]
    pub fn is_within(&self, now: EpochSeconds, ttl: TtlSecs) -> bool {
        self.age(now).get() < ttl.get()
    }

    /// The declared-file overlap between this record and a request.
    #[must_use]
    pub fn file_overlap_percent(&self, files: &[String]) -> BasisPoints {
        file_overlap_percent(&self.files, files)
    }

    /// The estimated fresh-token equivalent a warm resume saves.
    ///
    /// `cached × overlap × (write_multiplier - read_multiplier)`, saturating. The record's cached
    /// count is the prefix a resume would read; the overlap share is how much of that prefix the new
    /// request genuinely shares; `(w - r)` is the avoided rewrite cost minus the read actually paid.
    #[must_use]
    pub fn expected_saved_tokens(&self, overlap: BasisPoints, contract: &CacheContract) -> Tokens {
        expected_saved_tokens(self.cached, overlap, contract)
    }
}

/// The request a warm decision is made for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarmRequest {
    /// The subject.
    pub subject: SubjectId,
    /// The revision.
    pub revision: RevisionId,
    /// The workspace.
    pub workspace: String,
    /// The harness.
    pub harness: HarnessId,
    /// The provider.
    pub provider: ProviderId,
    /// The model, when named.
    pub model: Option<ModelId>,
    /// The effort, when any.
    pub effort: Option<String>,
    /// The canonical prefix of the request.
    pub prefix: PrefixDigest,
    /// The declared files the request will touch.
    pub files: Vec<String>,
}

impl WarmRequest {
    /// Build a request for the same identity as a record.
    #[must_use]
    pub fn for_record(record: &WarmRecord, prefix: PrefixDigest, files: Vec<String>) -> Self {
        Self {
            subject: record.subject.clone(),
            revision: record.revision.clone(),
            workspace: record.workspace.clone(),
            harness: record.harness.clone(),
            provider: record.provider.clone(),
            model: record.model.clone(),
            effort: record.effort.clone(),
            prefix,
            files,
        }
    }
}

/// Which identity field a candidate failed to match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarmIdentityMismatch {
    /// A different subject.
    Subject,
    /// A different revision.
    Revision,
    /// A different workspace.
    Workspace,
    /// A different harness.
    Harness,
    /// A different provider.
    Provider,
    /// A different model.
    Model,
    /// A different effort.
    Effort,
    /// A different canonical prefix.
    Prefix,
}

impl WarmIdentityMismatch {
    /// The stable code recorded with a rejection.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            WarmIdentityMismatch::Subject => "subject_mismatch",
            WarmIdentityMismatch::Revision => "revision_mismatch",
            WarmIdentityMismatch::Workspace => "workspace_mismatch",
            WarmIdentityMismatch::Harness => "harness_mismatch",
            WarmIdentityMismatch::Provider => "provider_mismatch",
            WarmIdentityMismatch::Model => "model_mismatch",
            WarmIdentityMismatch::Effort => "effort_mismatch",
            WarmIdentityMismatch::Prefix => "prefix_mismatch",
        }
    }
}

/// Why a candidate was not reused.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WarmRejectReason {
    /// The profile declares no TTL, so a warm window cannot be established.
    NoTtl,
    /// The candidate is at or past the declared TTL (a step, not a decay).
    Expired,
    /// An identity field moved.
    Identity(WarmIdentityMismatch),
    /// The declared-file overlap is below the caller's floor.
    OverlapBelowMinimum(BasisPoints),
    /// The candidate carries no usable session key.
    NotResumable,
}

impl WarmRejectReason {
    /// The stable code recorded with a rejection.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            WarmRejectReason::NoTtl => "no_ttl",
            WarmRejectReason::Expired => "expired",
            WarmRejectReason::Identity(_) => "identity_mismatch",
            WarmRejectReason::OverlapBelowMinimum(_) => "overlap_below_minimum",
            WarmRejectReason::NotResumable => "not_resumable",
        }
    }
}

/// A rejected candidate and the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarmRejection {
    /// The candidate that was not used.
    pub record: WarmRecord,
    /// Why it was rejected.
    pub reason: WarmRejectReason,
}

/// A chosen warm route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarmRoute {
    /// The candidate being reused.
    pub record: WarmRecord,
    /// The declared-file overlap with the request.
    pub overlap: BasisPoints,
    /// The warm lifetime remaining at the decision instant.
    pub remaining: TtlSecs,
    /// The estimated fresh-token equivalent saved.
    pub expected_saved: Tokens,
}

/// The outcome of a warm decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarmDecision {
    /// The chosen route, when one qualified.
    pub route: Option<WarmRoute>,
    /// Every candidate that did not qualify, with its reason.
    pub rejected: Vec<WarmRejection>,
}

impl WarmDecision {
    /// Whether a warm route was chosen.
    #[must_use]
    pub fn is_warm(&self) -> bool {
        self.route.is_some()
    }

    /// The chosen route, when one exists.
    #[must_use]
    pub fn chosen(&self) -> Option<&WarmRoute> {
        self.route.as_ref()
    }
}

/// Decide which candidate to reuse, if any.
///
/// A candidate must match the request identity exactly, be inside `ttl`, have a usable session key,
/// and meet `minimum_overlap`. At or past the TTL it is rejected as expired — no decay credit.
/// Among qualifiers the highest overlap wins, then the most remaining lifetime, then the newest
/// end; the ordering is total, so the decision is deterministic for equal inputs.
#[must_use]
pub fn decide(
    records: &[WarmRecord],
    request: &WarmRequest,
    now: EpochSeconds,
    ttl: Option<TtlSecs>,
    minimum_overlap: BasisPoints,
    contract: &CacheContract,
) -> WarmDecision {
    let mut rejected = Vec::new();
    let mut qualified: Vec<WarmRoute> = Vec::new();
    for record in records {
        let Some(reason) = reject_reason(record, request, now, ttl, minimum_overlap) else {
            let overlap = record.file_overlap_percent(&request.files);
            let remaining = ttl
                .map(|ttl| ttl.remaining(now, record.ended_at))
                .unwrap_or(WarmAgeSecs::ZERO);
            let remaining_ttl = TtlSecs::new(remaining.get().max(1))
                .unwrap_or(TtlSecs::new(1).expect("one is a positive TTL"));
            qualified.push(WarmRoute {
                expected_saved: record.expected_saved_tokens(overlap, contract),
                record: record.clone(),
                overlap,
                remaining: remaining_ttl,
            });
            continue;
        };
        rejected.push(WarmRejection {
            record: record.clone(),
            reason,
        });
    }
    qualified.sort_by(|left, right| {
        right
            .overlap
            .cmp(&left.overlap)
            .then_with(|| right.remaining.get().cmp(&left.remaining.get()))
            .then_with(|| right.record.ended_at.get().cmp(&left.record.ended_at.get()))
    });
    let route = if qualified.is_empty() {
        None
    } else {
        Some(qualified.remove(0))
    };
    WarmDecision { route, rejected }
}

/// The first reason a candidate cannot be reused, or `None` when it qualifies.
fn reject_reason(
    record: &WarmRecord,
    request: &WarmRequest,
    now: EpochSeconds,
    ttl: Option<TtlSecs>,
    minimum_overlap: BasisPoints,
) -> Option<WarmRejectReason> {
    if record.subject != request.subject {
        return Some(WarmRejectReason::Identity(WarmIdentityMismatch::Subject));
    }
    if record.revision != request.revision {
        return Some(WarmRejectReason::Identity(WarmIdentityMismatch::Revision));
    }
    if record.workspace != request.workspace {
        return Some(WarmRejectReason::Identity(WarmIdentityMismatch::Workspace));
    }
    if record.harness != request.harness {
        return Some(WarmRejectReason::Identity(WarmIdentityMismatch::Harness));
    }
    if record.provider != request.provider {
        return Some(WarmRejectReason::Identity(WarmIdentityMismatch::Provider));
    }
    if record.model != request.model {
        return Some(WarmRejectReason::Identity(WarmIdentityMismatch::Model));
    }
    if record.effort != request.effort {
        return Some(WarmRejectReason::Identity(WarmIdentityMismatch::Effort));
    }
    if record.prefix != request.prefix {
        return Some(WarmRejectReason::Identity(WarmIdentityMismatch::Prefix));
    }
    let Some(ttl) = ttl else {
        return Some(WarmRejectReason::NoTtl);
    };
    if !record.is_within(now, ttl) {
        return Some(WarmRejectReason::Expired);
    }
    if !record.session_key.as_deref().is_some_and(usable_prefix) {
        return Some(WarmRejectReason::NotResumable);
    }
    let overlap = record.file_overlap_percent(&request.files);
    if overlap < minimum_overlap {
        return Some(WarmRejectReason::OverlapBelowMinimum(overlap));
    }
    None
}

/// Declared-file Jaccard overlap in basis points (`0..=10_000`).
///
/// Empty sets score 0: work that names no files has no context to match, and a fabricated 100 %
/// would make every unnamed run look reusable.
#[must_use]
pub fn file_overlap_percent(left: &[String], right: &[String]) -> BasisPoints {
    let left: std::collections::BTreeSet<&str> = left.iter().map(String::as_str).collect();
    let right: std::collections::BTreeSet<&str> = right.iter().map(String::as_str).collect();
    if left.is_empty() || right.is_empty() {
        return BasisPoints::ZERO;
    }
    let intersection = left.intersection(&right).count();
    let union = left.union(&right).count();
    BasisPoints::from_ratio(intersection as u128, union as u128).unwrap_or(BasisPoints::ZERO)
}

/// The estimated fresh-token equivalent a warm resume saves.
///
/// `cached × overlap × (w - r)`, computed in `u128` and saturating at [`u64::MAX`].
#[must_use]
pub fn expected_saved_tokens(
    cached: CachedTokens,
    overlap: BasisPoints,
    contract: &CacheContract,
) -> Tokens {
    let saving = contract.reuse_net_saving();
    let numerator = u128::from(cached.get())
        .saturating_mul(u128::from(overlap.get()))
        .saturating_mul(saving.numerator());
    let denominator = 10_000_u128.saturating_mul(saving.denominator()).max(1);
    Tokens::new(u64::try_from(numerator / denominator).unwrap_or(u64::MAX))
}

impl fmt::Display for WarmRoute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} {} (overlap {}, {} remaining, ~{} tokens saved)",
            self.record.harness,
            short(self.record.session_key.as_deref().unwrap_or_default()),
            self.overlap,
            self.remaining,
            self.expected_saved
        )
    }
}

/// The classification of a record's usage for a report line.
#[must_use]
pub fn reported_usage(record: &WarmRecord) -> (Reported<CachedTokens>, Reported<FreshTokens>) {
    (
        Reported::Reported(record.cached),
        Reported::Reported(record.fresh),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::newtypes::{ContractId, ContractVersion};

    fn subject() -> SubjectId {
        SubjectId::new("subject-1").expect("valid")
    }

    fn revision() -> RevisionId {
        RevisionId::new("rev-1").expect("valid")
    }

    fn harness() -> HarnessId {
        HarnessId::new("codex").expect("valid")
    }

    fn provider() -> ProviderId {
        ProviderId::new("openai").expect("valid")
    }

    fn prefix() -> PrefixDigest {
        crate::prefix::digest(&[b"prefix"])
    }

    fn record() -> WarmRecord {
        WarmRecord::new(
            subject(),
            revision(),
            "ws",
            harness(),
            provider(),
            prefix(),
            EpochSeconds::new(1_000),
        )
        .with_session("session-12345678")
        .with_usage(CachedTokens::new(10_000), FreshTokens::new(1_000))
        .with_files(vec!["a.rs".to_owned(), "b.rs".to_owned()])
    }

    fn contract() -> CacheContract {
        crate::cache::semantics(&provider(), Some(&ModelId::new("gpt-5.6").expect("valid")))
    }

    #[test]
    fn the_ttl_is_a_step_at_the_expiry_instant() {
        let ttl = TtlSecs::new(600).expect("non-zero");
        let record = record();
        assert!(record.is_within(EpochSeconds::new(1_599), ttl));
        assert!(
            !record.is_within(EpochSeconds::new(1_600), ttl),
            "exactly at expiry is expired"
        );
        assert_eq!(
            warm_expires_at(EpochSeconds::new(1_000), ttl),
            EpochSeconds::new(1_600)
        );
        assert_eq!(
            warm_age_secs(EpochSeconds::new(1_700), EpochSeconds::new(1_000)).get(),
            700
        );
    }

    #[test]
    fn a_session_is_captured_from_the_profiles_paths() {
        let claude = HarnessProfile::claude();
        let bytes = br#"{"session_id":"abc12345678","usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0}}"#;
        assert_eq!(
            capture_session(&claude, bytes).as_deref(),
            Some("abc12345678")
        );
        let missing = br#"{"usage":{"input_tokens":1}}"#;
        assert_eq!(capture_session(&claude, missing), None);
    }

    #[test]
    fn usability_rejects_short_or_blank_keys_but_keeps_real_ids() {
        assert!(usable_prefix("12345678"));
        assert!(!usable_prefix("1234567"));
        assert!(!usable_prefix("   "));
        assert!(!usable_prefix("has space here"));
        assert_eq!(short("1234567890"), "12345678");
        assert_eq!(short("123"), "123");
    }

    #[test]
    fn prepare_resumes_with_the_template_and_assigns_on_a_fresh_run() {
        let claude = HarnessProfile::claude();
        let context =
            InvocationContext::new("claude", vec!["-p".to_owned(), "do the work".to_owned()]);
        let resumed = prepare(&claude, &context, Some("session-12345678"), None).expect("resumes");
        assert_eq!(resumed.argv, vec!["claude", "--resume", "session-12345678"]);
        assert_eq!(resumed.assigned, None);

        let fresh = prepare(&claude, &context, None, Some("assigned-12345678")).expect("assigns");
        assert_eq!(
            fresh.argv,
            vec![
                "claude",
                "-p",
                "do the work",
                "--session-id",
                "assigned-12345678"
            ]
        );
        assert_eq!(fresh.assigned.as_deref(), Some("assigned-12345678"));

        // A profile without a resume form ignores the resume request and stays fresh.
        let grok = HarnessProfile::grok();
        let fresh = prepare(&grok, &context, Some("session-12345678"), None).expect("fresh");
        assert_eq!(fresh.argv, vec!["claude", "-p", "do the work"]);
        assert!(!can_resume(&grok));
        assert!(!assignable(&grok));
    }

    #[test]
    fn prepare_refuses_an_unusable_session_and_an_unfilled_prompt_placeholder() {
        let claude = HarnessProfile::claude();
        let context = InvocationContext::new("claude", Vec::new());
        assert!(prepare(&claude, &context, Some("short"), None).is_err());
        let mut prompt_placeholder = HarnessProfile::claude();
        prompt_placeholder.resume_argv = vec!["--resume".to_owned(), PROMPT_PLACEHOLDER.to_owned()];
        assert!(matches!(
            prepare(
                &prompt_placeholder,
                &context,
                Some("session-12345678"),
                None
            ),
            Err(Error::InvalidTarget { .. })
        ));
        let with_prompt = context.with_prompt("hello");
        let prepared = prepare(
            &prompt_placeholder,
            &with_prompt,
            Some("session-12345678"),
            None,
        )
        .expect("the prompt fills the template");
        assert_eq!(prepared.argv, vec!["claude", "--resume", "hello"]);
    }

    #[test]
    fn overlap_is_jaccard_and_names_no_files_means_no_reuse() {
        let left = vec!["a.rs".to_owned(), "b.rs".to_owned()];
        let right = vec!["b.rs".to_owned(), "c.rs".to_owned()];
        assert_eq!(
            file_overlap_percent(&left, &right),
            BasisPoints::new(3_333).expect("valid")
        );
        assert_eq!(file_overlap_percent(&left, &left), BasisPoints::MAX);
        assert_eq!(file_overlap_percent(&[], &left), BasisPoints::ZERO);
        assert_eq!(
            record().file_overlap_percent(&right),
            BasisPoints::new(3_333).expect("valid")
        );
    }

    #[test]
    fn the_counterfactual_is_cached_times_overlap_times_write_minus_read() {
        let contract = contract(); // w = 1.25, r = 0.1 -> net 1.15
        let saved = expected_saved_tokens(
            CachedTokens::new(10_000),
            BasisPoints::new(5_000).expect("valid"),
            &contract,
        );
        // 10_000 * 0.5 * 1.15 = 5_750
        assert_eq!(saved, Tokens::new(5_750));
        let strict = crate::cache::strict();
        assert_eq!(
            expected_saved_tokens(CachedTokens::new(10_000), BasisPoints::MAX, &strict),
            Tokens::ZERO,
            "a strict contract claims no saving"
        );
    }

    #[test]
    fn the_decision_prefers_the_highest_overlap_then_the_most_remaining() {
        let request = WarmRequest {
            subject: subject(),
            revision: revision(),
            workspace: "ws".to_owned(),
            harness: harness(),
            provider: provider(),
            model: None,
            effort: None,
            prefix: prefix(),
            files: vec!["a.rs".to_owned(), "b.rs".to_owned()],
        };
        let older = WarmRecord::new(
            subject(),
            revision(),
            "ws",
            harness(),
            provider(),
            prefix(),
            EpochSeconds::new(900),
        )
        .with_session("older-12345678")
        .with_files(vec!["a.rs".to_owned()]);
        let newer = record();
        let decision = decide(
            &[older, newer],
            &request,
            EpochSeconds::new(1_100),
            TtlSecs::new(600),
            BasisPoints::new(1).expect("valid"),
            &contract(),
        );
        let chosen = decision.chosen().expect("a warm route");
        assert_eq!(
            chosen.record.session_key.as_deref(),
            Some("session-12345678")
        );
        assert_eq!(chosen.overlap, BasisPoints::MAX);
        assert_eq!(chosen.expected_saved, Tokens::new(11_500));
    }

    #[test]
    fn a_decision_names_every_rejection_and_refuses_a_moved_identity() {
        let request = WarmRequest {
            subject: subject(),
            revision: revision(),
            workspace: "ws".to_owned(),
            harness: harness(),
            provider: provider(),
            model: Some(ModelId::new("other").expect("valid")),
            effort: None,
            prefix: prefix(),
            files: vec!["a.rs".to_owned(), "b.rs".to_owned()],
        };
        let decision = decide(
            &[record()],
            &request,
            EpochSeconds::new(1_100),
            TtlSecs::new(600),
            BasisPoints::ZERO,
            &contract(),
        );
        assert!(decision.route.is_none());
        assert!(matches!(
            decision.rejected[0].reason,
            WarmRejectReason::Identity(WarmIdentityMismatch::Model)
        ));
        assert_eq!(decision.rejected[0].reason.code(), "identity_mismatch");
        assert_eq!(WarmIdentityMismatch::Model.code(), "model_mismatch");
    }

    #[test]
    fn an_expired_session_is_rejected_even_with_a_perfect_score() {
        let request = WarmRequest::for_record(&record(), prefix(), vec!["a.rs".to_owned()]);
        let decision = decide(
            &[record()],
            &request,
            EpochSeconds::new(1_600),
            TtlSecs::new(600),
            BasisPoints::ZERO,
            &contract(),
        );
        assert!(decision.route.is_none());
        assert_eq!(decision.rejected[0].reason, WarmRejectReason::Expired);
        let no_ttl = decide(
            &[record()],
            &request,
            EpochSeconds::new(1_100),
            None,
            BasisPoints::ZERO,
            &contract(),
        );
        assert_eq!(no_ttl.rejected[0].reason, WarmRejectReason::NoTtl);
    }

    #[test]
    fn a_candidate_without_a_usable_key_is_not_resumable() {
        let request = WarmRequest::for_record(&record(), prefix(), vec!["a.rs".to_owned()]);
        let mut no_key = record();
        no_key.session_key = None;
        let decision = decide(
            &[no_key],
            &request,
            EpochSeconds::new(1_100),
            TtlSecs::new(600),
            BasisPoints::ZERO,
            &contract(),
        );
        assert_eq!(decision.rejected[0].reason, WarmRejectReason::NotResumable);
    }

    #[test]
    fn a_route_renders_stably_and_a_record_round_trips() {
        let route = WarmRoute {
            record: record(),
            overlap: BasisPoints::new(3_300).expect("valid"),
            remaining: TtlSecs::new(120).expect("non-zero"),
            expected_saved: Tokens::new(3_000),
        };
        let rendered = route.to_string();
        assert!(rendered.contains("codex"), "{rendered}");
        assert!(rendered.contains("session-"), "{rendered}");
        let encoded = serde_json::to_string(&record()).expect("serializes");
        let decoded: WarmRecord = serde_json::from_str(&encoded).expect("deserializes");
        assert_eq!(decoded, record());
        // The contract id only exists to keep the test honest about the versioned data shape.
        let _ = ContractId::new("aics").expect("valid");
        let _ = ContractVersion::new(1);
    }
}
