//! Parsing a harness's output into reported token usage, without inventing numbers.
//!
//! The parser reads the **last document that carries any usage** (or, for a step event harness,
//! every matching step event summed), resolves each field through the profile's ordered paths, and
//! keeps the difference between:
//!
//! - **measured zero** — the field was present and its value was `0`;
//! - **not reported** — no configured path resolved, so no number exists;
//! - **inapplicable** — the profile declares no path for the field at all.
//!
//! All three are distinguishable in the result: [`Reported::NotReported`] covers the last two, and
//! the warnings name a field whose configured path resolved to something that is not a valid count
//! (a negative or fractional value) rather than silently clamping it.

// Provider stdout is untrusted, untyped JSON; this module is the boundary that parses it and
// constructs typed values from it.
#![allow(clippy::disallowed_types)]

use crate::error::Error;
use crate::newtypes::{
    BasisPoints, CacheWriteTokens, CachedTokens, FreshTokens, HarnessId, OutputTokens,
    ReasoningTokens, Tokens,
};
use crate::profile::{HarnessProfile, OutputShape, StepEventMatch};
use crate::prompt::PromptTokens;
use crate::reported::Reported;

/// Raw reported totals, used as the cumulative baseline of a resumed conversation.
///
/// All three are raw provider numbers: `input` is the reported input count (whose meaning depends
/// on the profile's [`crate::profile::InputShape`]), not a normalized fresh count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UsageTotals {
    /// Reported input tokens.
    pub input: Tokens,
    /// Reported output tokens.
    pub output: OutputTokens,
    /// Reported cache-read tokens.
    pub cached: CachedTokens,
}

impl UsageTotals {
    /// Build the totals.
    #[must_use]
    pub const fn new(input: Tokens, output: OutputTokens, cached: CachedTokens) -> Self {
        Self {
            input,
            output,
            cached,
        }
    }
}

/// How a parsed number relates to the attempt that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UsageShape {
    /// The number is this attempt's usage.
    PerRun,
    /// The number was a conversation running total; the reported usage is the difference from the
    /// baseline.
    Delta,
}

impl UsageShape {
    /// The stable spelling.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            UsageShape::PerRun => "per_run",
            UsageShape::Delta => "delta",
        }
    }
}

/// A parse-time observation that is not a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UsageWarning {
    /// A field's configured path resolved to a value that is not a count (negative, fractional,
    /// or a non-number).
    InvalidField {
        /// The field name (`input`, `output`, `cache_read`, `cache_write`, `reasoning`).
        field: &'static str,
        /// The path that resolved.
        path: String,
        /// The offending JSON value, rendered compactly.
        value: String,
    },
    /// Output bytes were not valid UTF-8, so no document could be parsed.
    NotUtf8,
    /// A JSON document or JSONL line did not parse.
    MalformedDocument {
        /// The decoder's message.
        detail: String,
    },
    /// The profile declares a harness version this build does not know.
    UnsupportedHarnessVersion {
        /// The harness.
        harness: HarnessId,
        /// The version reported by the caller.
        version: String,
    },
}

impl std::fmt::Display for UsageWarning {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UsageWarning::InvalidField { field, path, value } => {
                write!(formatter, "`{field}` at `{path}` is not a count: {value}")
            }
            UsageWarning::NotUtf8 => write!(formatter, "output is not valid UTF-8"),
            UsageWarning::MalformedDocument { detail } => {
                write!(formatter, "output document did not parse: {detail}")
            }
            UsageWarning::UnsupportedHarnessVersion { harness, version } => write!(
                formatter,
                "harness `{harness}` version `{version}` is outside the profile's declared range"
            ),
        }
    }
}

/// The parsed usage of one attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageReport {
    /// The normalized prompt-token fields.
    pub prompt: PromptTokens,
    /// Output tokens, when reported.
    pub output: Reported<OutputTokens>,
    /// Reasoning tokens, when reported separately.
    pub reasoning: Reported<ReasoningTokens>,
}

impl UsageReport {
    /// Fresh input tokens of this attempt, normalized through the shape.
    #[must_use]
    pub fn fresh(&self) -> Reported<FreshTokens> {
        self.prompt.fresh()
    }

    /// Expected prompt work (the cache-share denominator), normalized through the shape.
    #[must_use]
    pub fn expected(&self) -> Reported<Tokens> {
        self.prompt.expected()
    }

    /// The cache-read share of prompt work, when both halves were reported.
    #[must_use]
    pub fn cache_hit_bps(&self) -> Option<BasisPoints> {
        self.prompt.cache_hit_bps()
    }
}

/// The result of parsing one attempt's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseOutcome {
    /// The parsed usage. Every field may be [`Reported::NotReported`].
    pub usage: UsageReport,
    /// Raw totals, when input, output, and cache reads were all reported.
    ///
    /// This is the number a resumed cumulative harness should remember as its next baseline.
    pub totals: Option<UsageTotals>,
    /// Parse observations that are not failures.
    pub warnings: Vec<UsageWarning>,
}

/// One request's token counts, when the harness output separates requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestUsage {
    /// The reported input count for the request.
    pub input: Tokens,
    /// Cache-read tokens for the request, when reported.
    pub cached: Reported<CachedTokens>,
    /// Cache-write tokens for the request, when reported.
    pub cache_write: Reported<CacheWriteTokens>,
}

/// The last session identifier the output printed, using the profile's session paths.
///
/// Session capture lives beside the parser because it needs the same document splitting; a caller
/// that had to parse JSON itself would be a second implementation of the output shape.
#[must_use]
pub fn session_key(profile: &HarnessProfile, bytes: &[u8]) -> Option<String> {
    let documents = json_documents(profile, bytes);
    documents.iter().rev().find_map(|document| {
        profile.session_paths.iter().find_map(|path| {
            document
                .pointer(&pointer(path))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| crate::warm::usable_prefix(value))
                .map(ToOwned::to_owned)
        })
    })
}

/// Parse a harness's output bytes.
///
/// Never panics and never fails: malformed bytes produce an empty usage report plus warnings. The
/// caller decides whether an all-[`Reported::NotReported`] report is a problem for its workflow.
#[must_use]
pub fn parse(profile: &HarnessProfile, bytes: &[u8]) -> ParseOutcome {
    let mut warnings = Vec::new();
    let documents = documents(profile.output_shape, bytes, &mut warnings);
    let usage = if let Some(step) = &profile.step_event {
        sum_step_events(profile, &documents, step, &mut warnings)
    } else {
        parse_one(profile, &documents, &mut warnings)
    };
    let totals = totals(profile, &usage);
    ParseOutcome {
        usage,
        totals,
        warnings,
    }
}

/// Like [`parse`], but refuses a harness version the profile does not declare.
///
/// # Errors
///
/// [`Error::Unsupported`] when `version` is outside the profile's declared range.
pub fn parse_checked(
    profile: &HarnessProfile,
    version: &crate::newtypes::HarnessVersion,
    bytes: &[u8],
) -> Result<ParseOutcome, Error> {
    if !profile.supports_version(version) {
        return Err(Error::Unsupported {
            harness: "profile",
            version: format!("{} {}", profile.harness, version),
        });
    }
    Ok(parse(profile, bytes))
}

/// Split output bytes into JSON documents according to the profile's output shape.
fn documents(
    shape: OutputShape,
    bytes: &[u8],
    warnings: &mut Vec<UsageWarning>,
) -> Vec<serde_json::Value> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        warnings.push(UsageWarning::NotUtf8);
        return Vec::new();
    };
    match shape {
        OutputShape::Text => Vec::new(),
        OutputShape::Json => match serde_json::from_str::<serde_json::Value>(text) {
            Ok(value) => vec![value],
            Err(error) => {
                warnings.push(UsageWarning::MalformedDocument {
                    detail: error.to_string(),
                });
                Vec::new()
            }
        },
        OutputShape::Jsonl => {
            let mut out = Vec::new();
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                match serde_json::from_str::<serde_json::Value>(line) {
                    Ok(value) => out.push(value),
                    Err(error) => warnings.push(UsageWarning::MalformedDocument {
                        detail: error.to_string(),
                    }),
                }
            }
            out
        }
    }
}

/// The JSON documents a profile's output shape yields, with malformed documents skipped.
///
/// Exposed for consumers that need the same document splitting the parser uses (session capture,
/// per-step accounting) without duplicating the shape rules.
#[must_use]
pub fn json_documents(profile: &HarnessProfile, bytes: &[u8]) -> Vec<serde_json::Value> {
    let mut warnings = Vec::new();
    documents(profile.output_shape, bytes, &mut warnings)
}

/// Resolve the last document that carries any usage field, then read every field from it.
fn parse_one(
    profile: &HarnessProfile,
    documents: &[serde_json::Value],
    warnings: &mut Vec<UsageWarning>,
) -> UsageReport {
    let paths = &profile.usage_paths;
    let chosen = documents.iter().rev().find(|document| {
        has_any(
            document,
            &[
                &paths.input,
                &paths.output,
                &paths.cache_read,
                &paths.cache_write,
                &paths.reasoning,
            ],
        )
    });
    match chosen {
        Some(document) => report_from(profile, document, warnings),
        None => empty_report(profile),
    }
}

/// Sum usage across the profile's step-end events.
fn sum_step_events(
    profile: &HarnessProfile,
    documents: &[serde_json::Value],
    step: &StepEventMatch,
    warnings: &mut Vec<UsageWarning>,
) -> UsageReport {
    let mut input = Reported::NotReported;
    let mut output = Reported::NotReported;
    let mut read = Reported::NotReported;
    let mut write = Reported::NotReported;
    let mut reasoning = Reported::NotReported;
    for document in documents
        .iter()
        .filter(|document| step_matches(document, step))
    {
        input = add_field(
            input,
            field(
                profile,
                document,
                &profile.usage_paths.input,
                "input",
                warnings,
            ),
        );
        output = add_field(
            output,
            field(
                profile,
                document,
                &profile.usage_paths.output,
                "output",
                warnings,
            ),
        );
        read = add_field(
            read,
            field(
                profile,
                document,
                &profile.usage_paths.cache_read,
                "cache_read",
                warnings,
            ),
        );
        write = add_field(
            write,
            field(
                profile,
                document,
                &profile.usage_paths.cache_write,
                "cache_write",
                warnings,
            ),
        );
        reasoning = add_field(
            reasoning,
            field(
                profile,
                document,
                &profile.usage_paths.reasoning,
                "reasoning",
                warnings,
            ),
        );
    }
    build_report(profile, input, output, read, write, reasoning)
}

/// Whether a step-event document is a completed step.
fn step_matches(document: &serde_json::Value, step: &StepEventMatch) -> bool {
    document
        .pointer(&pointer(&step.type_path))
        .and_then(serde_json::Value::as_str)
        == Some(step.type_value.as_str())
        && document
            .pointer(&pointer(&step.phase_path))
            .and_then(serde_json::Value::as_str)
            == Some(step.phase_value.as_str())
}

/// A dotted path as a JSON pointer.
fn pointer(path: &str) -> String {
    format!("/{}", path.replace('.', "/"))
}

/// Whether any of the path groups resolves in a document.
fn has_any(document: &serde_json::Value, groups: &[&Vec<String>]) -> bool {
    groups
        .iter()
        .any(|paths| paths.iter().any(|path| resolve(document, path).is_some()))
}

/// Resolve the first path that exists in the document.
fn resolve<'a>(document: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    document.pointer(&pointer(path))
}

/// Read one field through its ordered paths.
fn field(
    _profile: &HarnessProfile,
    document: &serde_json::Value,
    paths: &[String],
    name: &'static str,
    warnings: &mut Vec<UsageWarning>,
) -> Reported<u64> {
    for path in paths {
        let Some(value) = resolve(document, path) else {
            continue;
        };
        if let Some(count) = value.as_u64() {
            return Reported::Reported(count);
        }
        // A negative or fractional provider number is an observation we refuse to clamp: it is
        // reported as a warning and the field stays absent.
        warnings.push(UsageWarning::InvalidField {
            field: name,
            path: path.clone(),
            value: value.to_string(),
        });
        return Reported::NotReported;
    }
    Reported::NotReported
}

/// Sum two reported counts, preserving absence when neither side reported.
fn add_field(total: Reported<u64>, next: Reported<u64>) -> Reported<u64> {
    match (total, next) {
        (Reported::NotReported, Reported::NotReported) => Reported::NotReported,
        (Reported::NotReported, Reported::Reported(value))
        | (Reported::Reported(value), Reported::NotReported) => Reported::Reported(value),
        (Reported::Reported(left), Reported::Reported(right)) => {
            Reported::Reported(left.saturating_add(right))
        }
    }
}

/// Build a report from reported raw numbers.
fn build_report(
    profile: &HarnessProfile,
    input: Reported<u64>,
    output: Reported<u64>,
    read: Reported<u64>,
    write: Reported<u64>,
    reasoning: Reported<u64>,
) -> UsageReport {
    let into_tokens = |value: Reported<u64>| -> Reported<Tokens> { value.map(Tokens::new) };
    UsageReport {
        prompt: PromptTokens::new(
            into_tokens(input),
            read.map(CachedTokens::new),
            write.map(CacheWriteTokens::new),
            profile.input_shape,
        ),
        output: output.map(OutputTokens::new),
        reasoning: reasoning.map(ReasoningTokens::new),
    }
}

/// Read every field from one document.
fn report_from(
    profile: &HarnessProfile,
    document: &serde_json::Value,
    warnings: &mut Vec<UsageWarning>,
) -> UsageReport {
    let paths = &profile.usage_paths;
    build_report(
        profile,
        field(profile, document, &paths.input, "input", warnings),
        field(profile, document, &paths.output, "output", warnings),
        field(profile, document, &paths.cache_read, "cache_read", warnings),
        field(
            profile,
            document,
            &paths.cache_write,
            "cache_write",
            warnings,
        ),
        field(profile, document, &paths.reasoning, "reasoning", warnings),
    )
}

/// A report with every field absent.
fn empty_report(profile: &HarnessProfile) -> UsageReport {
    build_report(
        profile,
        Reported::NotReported,
        Reported::NotReported,
        Reported::NotReported,
        Reported::NotReported,
        Reported::NotReported,
    )
}

/// Raw totals from a report, when input, output, and cache reads were all measured.
fn totals(profile: &HarnessProfile, usage: &UsageReport) -> Option<UsageTotals> {
    match (
        usage.prompt.reported_input.as_reported().copied(),
        usage.output.as_reported().copied(),
        usage.prompt.cache_read.as_reported().copied(),
    ) {
        (Some(input), Some(output), Some(cached)) => {
            let _ = profile;
            Some(UsageTotals {
                input,
                output: OutputTokens::new(output.get()),
                cached: CachedTokens::new(cached.get()),
            })
        }
        _ => None,
    }
}

/// Turn a cumulative harness's raw number into this attempt's usage.
///
/// Only a profile that declares cumulative accumulation (`codex`) is affected; every other
/// profile reports per-run numbers unchanged. The subtraction requires a baseline for the **same
/// conversation** and a monotonic raw count in every component; a CLI that switched back to
/// per-run reporting (raw below baseline) falls back to the raw number instead of clamping an
/// invented delta.
#[must_use]
pub fn effective_usage(
    profile: &HarnessProfile,
    raw: UsageTotals,
    baseline: Option<&UsageTotals>,
) -> (UsageTotals, UsageShape) {
    if !profile.cumulative_usage {
        return (raw, UsageShape::PerRun);
    }
    let Some(baseline) = baseline else {
        return (raw, UsageShape::PerRun);
    };
    if raw.input < baseline.input || raw.output < baseline.output || raw.cached < baseline.cached {
        return (raw, UsageShape::PerRun);
    }
    (
        UsageTotals {
            input: raw.input.saturating_sub(baseline.input),
            output: OutputTokens::new(raw.output.saturating_sub(baseline.output).get()),
            cached: CachedTokens::new(raw.cached.saturating_sub(baseline.cached).get()),
        },
        UsageShape::Delta,
    )
}

/// The total to remember after an attempt, so the next resume can subtract it.
///
/// Nothing reported keeps the previous total; a cumulative profile that fell back to a per-run
/// number forgets its baseline (storing it would subtract the wrong total next time); otherwise
/// the raw number is the new total.
#[must_use]
pub fn next_cumulative(
    profile: &HarnessProfile,
    raw: UsageTotals,
    shape: UsageShape,
    previous: Option<UsageTotals>,
) -> Option<UsageTotals> {
    if raw.input.is_zero() && raw.output.is_zero() && raw.cached.is_zero() {
        return previous;
    }
    if profile.cumulative_usage && shape == UsageShape::PerRun && previous.is_some() {
        return None;
    }
    Some(raw)
}

/// Per-request usage from a run's output, in request order.
///
/// Only what the harness exposes: a single JSON envelope is one request; JSONL request events are
/// one each (a cumulative harness's value is the delta from the previous event); a step-event
/// harness contributes one entry per completed step; text output yields an empty vector.
#[must_use]
pub fn per_request_usage(profile: &HarnessProfile, bytes: &[u8]) -> Vec<RequestUsage> {
    let mut warnings = Vec::new();
    let documents = documents(profile.output_shape, bytes, &mut warnings);
    if let Some(step) = &profile.step_event {
        return documents
            .iter()
            .filter(|document| step_matches(document, step))
            .filter_map(|document| request_from(profile, document, &mut warnings))
            .collect();
    }
    if profile.output_shape == OutputShape::Json {
        return documents
            .last()
            .and_then(|document| request_from(profile, document, &mut warnings))
            .into_iter()
            .collect();
    }
    let mut requests = Vec::new();
    let mut previous: Option<(u64, u64)> = None;
    for document in &documents {
        let Some(request) = request_from(profile, document, &mut warnings) else {
            continue;
        };
        if profile.cumulative_usage {
            let current = (
                request.input.get(),
                request.cached.unwrap_or(CachedTokens::ZERO).get(),
            );
            let request = match previous {
                Some((prior_input, prior_cached))
                    if current.0 >= prior_input && current.1 >= prior_cached =>
                {
                    RequestUsage {
                        input: Tokens::new(current.0 - prior_input),
                        cached: request.cached.map(|cached| {
                            CachedTokens::new(cached.get().saturating_sub(prior_cached))
                        }),
                        cache_write: request.cache_write,
                    }
                }
                _ => request,
            };
            previous = Some(current);
            requests.push(request);
        } else {
            requests.push(request);
        }
    }
    requests
}

/// Read one document as one request's usage.
fn request_from(
    profile: &HarnessProfile,
    document: &serde_json::Value,
    warnings: &mut Vec<UsageWarning>,
) -> Option<RequestUsage> {
    let paths = &profile.usage_paths;
    let input = field(profile, document, &paths.input, "input", warnings);
    let cached = field(profile, document, &paths.cache_read, "cache_read", warnings);
    let write = field(
        profile,
        document,
        &paths.cache_write,
        "cache_write",
        warnings,
    );
    match input {
        Reported::NotReported => None,
        Reported::Reported(input) => Some(RequestUsage {
            input: Tokens::new(input),
            cached: cached.map(CachedTokens::new),
            cache_write: write.map(CacheWriteTokens::new),
        }),
    }
}

/// Fresh input of the first request that reported usage.
///
/// This is the "cold turn" number: a resumed session's first request normally shows no cache reads,
/// and the first request of a fresh session is the comparison the EEPR contract uses.
#[must_use]
pub fn first_turn_fresh_input(profile: &HarnessProfile, bytes: &[u8]) -> Reported<FreshTokens> {
    let requests = per_request_usage(profile, bytes);
    let Some(first) = requests.first() else {
        return Reported::NotReported;
    };
    match profile.input_shape {
        crate::profile::InputShape::Inclusive => match first.cached {
            Reported::NotReported => Reported::NotReported,
            Reported::Reported(cached) => Reported::Reported(FreshTokens::new(
                first.input.saturating_sub(Tokens::new(cached.get())).get(),
            )),
        },
        _ => Reported::Reported(FreshTokens::new(first.input.get())),
    }
}

/// Cached / (cached + fresh) in basis points, rounded half up; `None` for a zero denominator.
#[must_use]
pub fn cache_hit_bps(cached: CachedTokens, fresh: FreshTokens) -> Option<BasisPoints> {
    BasisPoints::from_ratio(
        u128::from(cached.get()),
        u128::from(cached.get()) + u128::from(fresh.get()),
    )
}

/// Cached / (cached + fresh) as whole basis points, rounded half up; zero for no input.
#[must_use]
pub fn cache_percent(cached: CachedTokens, fresh: FreshTokens) -> BasisPoints {
    cache_hit_bps(cached, fresh).unwrap_or(BasisPoints::ZERO)
}

/// The cached share in the provider's own accounting.
///
/// Inclusive shapes report the cached count inside input, so the denominator is input; exclusive
/// and partitioned shapes report it beside fresh input, so the denominator is both. This is the
/// "provider cache share" a status line quotes, not the gate's normalized metric.
#[must_use]
pub fn provider_cache_percent(
    profile: &HarnessProfile,
    cached: CachedTokens,
    reported_input: Tokens,
) -> BasisPoints {
    let denominator = match profile.input_shape {
        crate::profile::InputShape::Inclusive => reported_input.get(),
        _ => reported_input.get().saturating_add(cached.get()),
    };
    BasisPoints::from_ratio(u128::from(cached.get()), u128::from(denominator))
        .unwrap_or(BasisPoints::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(bytes: &str) -> ParseOutcome {
        parse(&HarnessProfile::claude(), bytes.as_bytes())
    }

    #[test]
    fn an_exclusive_shape_reads_fresh_input_and_separate_reads() {
        let outcome = parsed(
            r#"{"usage":{"input_tokens":1000,"output_tokens":200,"cache_read_input_tokens":3000,"cache_creation_input_tokens":400}}"#,
        );
        assert_eq!(
            outcome.usage.prompt.fresh(),
            Reported::Reported(FreshTokens::new(1000))
        );
        assert_eq!(
            outcome.usage.prompt.expected(),
            Reported::Reported(Tokens::new(4000))
        );
        assert_eq!(
            outcome.usage.prompt.cache_hit_bps(),
            BasisPoints::new(7_500)
        );
        assert_eq!(
            outcome.usage.output,
            Reported::Reported(OutputTokens::new(200))
        );
        assert_eq!(
            outcome.usage.prompt.cache_write,
            Reported::Reported(CacheWriteTokens::new(400))
        );
        assert_eq!(
            outcome.totals,
            Some(UsageTotals::new(
                Tokens::new(1000),
                OutputTokens::new(200),
                CachedTokens::new(3000)
            ))
        );
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn an_inclusive_shape_subtracts_the_read_from_input() {
        let outcome = parse(
            &HarnessProfile::codex(),
            br#"{"type":"turn.completed","usage":{"input_tokens":1200,"cached_input_tokens":900,"output_tokens":40}}"#,
        );
        assert_eq!(
            outcome.usage.prompt.fresh(),
            Reported::Reported(FreshTokens::new(300))
        );
        assert_eq!(
            outcome.usage.prompt.expected(),
            Reported::Reported(Tokens::new(1200))
        );
        assert_eq!(
            outcome.usage.prompt.cache_hit_bps(),
            BasisPoints::new(7_500)
        );
    }

    #[test]
    fn a_measured_zero_is_a_measurement_and_a_missing_field_is_not() {
        let zero = parsed(
            r#"{"usage":{"input_tokens":1000,"output_tokens":0,"cache_read_input_tokens":0}}"#,
        );
        assert_eq!(
            zero.usage.prompt.cache_read,
            Reported::Reported(CachedTokens::new(0))
        );
        assert_eq!(zero.usage.prompt.cache_hit_bps(), BasisPoints::new(0));

        let missing = parsed(r#"{"usage":{"input_tokens":1000,"output_tokens":0}}"#);
        assert_eq!(missing.usage.prompt.cache_read, Reported::NotReported);
        assert_eq!(
            missing.usage.prompt.expected(),
            Reported::NotReported,
            "the denominator needs the read half of an exclusive shape"
        );
        assert_eq!(missing.usage.prompt.cache_hit_bps(), None);
        assert_eq!(missing.totals, None);
    }

    #[test]
    fn an_input_only_inclusive_report_has_an_unknown_fresh_share() {
        let outcome = parse(
            &HarnessProfile::codex(),
            br#"{"type":"turn.completed","usage":{"input_tokens":1200,"output_tokens":40}}"#,
        );
        assert_eq!(outcome.usage.prompt.fresh(), Reported::NotReported);
        assert_eq!(
            outcome.usage.prompt.expected(),
            Reported::Reported(Tokens::new(1200))
        );
        assert_eq!(outcome.usage.prompt.cache_hit_bps(), None);
    }

    #[test]
    fn a_negative_or_fractional_field_is_a_warning_not_a_clamped_zero() {
        let negative = parsed(
            r#"{"usage":{"input_tokens":-5,"output_tokens":0,"cache_read_input_tokens":0}}"#,
        );
        assert_eq!(negative.usage.prompt.reported_input, Reported::NotReported);
        assert!(
            negative.warnings.iter().any(|warning| matches!(
                warning,
                UsageWarning::InvalidField { field: "input", .. }
            ))
        );
        let fractional = parsed(
            r#"{"usage":{"input_tokens":1.5,"output_tokens":0,"cache_read_input_tokens":0}}"#,
        );
        assert_eq!(
            fractional.usage.prompt.reported_input,
            Reported::NotReported
        );
    }

    #[test]
    fn malformed_output_is_an_empty_report_with_a_warning() {
        let outcome = parsed("this is not json");
        assert_eq!(outcome.usage, empty_report(&HarnessProfile::claude()));
        assert_eq!(outcome.totals, None);
        assert!(matches!(
            outcome.warnings.as_slice(),
            [UsageWarning::MalformedDocument { .. }]
        ));
        let not_utf8 = parse(&HarnessProfile::claude(), &[0xff, 0xfe]);
        assert!(matches!(
            not_utf8.warnings.as_slice(),
            [UsageWarning::NotUtf8]
        ));
    }

    #[test]
    fn a_cumulative_harness_reports_the_delta_and_a_non_monotonic_stream_falls_back() {
        let profile = HarnessProfile::codex();
        let raw = UsageTotals::new(
            Tokens::new(1200),
            OutputTokens::new(40),
            CachedTokens::new(900),
        );
        let baseline = UsageTotals::new(
            Tokens::new(700),
            OutputTokens::new(10),
            CachedTokens::new(400),
        );
        let (delta, shape) = effective_usage(&profile, raw, Some(&baseline));
        assert_eq!(shape, UsageShape::Delta);
        assert_eq!(delta.input, Tokens::new(500));
        assert_eq!(delta.cached, CachedTokens::new(500));

        let reset = UsageTotals::new(Tokens::new(10), OutputTokens::new(1), CachedTokens::new(0));
        let (raw_fallback, shape) = effective_usage(&profile, reset, Some(&raw));
        assert_eq!(shape, UsageShape::PerRun);
        assert_eq!(raw_fallback, reset);

        // The Claude profile is per-run: a baseline never changes its number.
        let claude = HarnessProfile::claude();
        assert_eq!(
            effective_usage(&claude, raw, Some(&baseline)),
            (raw, UsageShape::PerRun)
        );
    }

    #[test]
    fn a_cumulative_baseline_is_forgotten_when_the_cli_switches_shape() {
        let profile = HarnessProfile::codex();
        let previous = Some(UsageTotals::new(
            Tokens::new(10),
            OutputTokens::new(1),
            CachedTokens::new(0),
        ));
        assert_eq!(
            next_cumulative(
                &profile,
                UsageTotals::new(Tokens::ZERO, OutputTokens::ZERO, CachedTokens::ZERO),
                UsageShape::Delta,
                previous
            ),
            previous,
            "nothing reported keeps the previous total"
        );
        let raw = UsageTotals::new(Tokens::new(5), OutputTokens::new(2), CachedTokens::new(0));
        assert_eq!(
            next_cumulative(&profile, raw, UsageShape::PerRun, previous),
            None
        );
    }

    #[test]
    fn the_jsonl_events_are_the_requests_and_the_first_turn_is_the_first_one() {
        let profile = HarnessProfile::codex();
        let output = br#"{"type":"turn.completed","usage":{"input_tokens":1000,"cached_input_tokens":0,"output_tokens":10}}
{"type":"turn.completed","usage":{"input_tokens":2500,"cached_input_tokens":1500,"output_tokens":25}}"#;
        let requests = per_request_usage(&profile, output);
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].input, Tokens::new(1000));
        assert_eq!(
            requests[1].input,
            Tokens::new(1500),
            "the second event is a delta"
        );
        assert_eq!(
            requests[1].cached,
            Reported::Reported(CachedTokens::new(1500))
        );
        assert_eq!(
            first_turn_fresh_input(&profile, output),
            Reported::Reported(FreshTokens::new(1000))
        );
    }

    #[test]
    fn a_step_event_harness_sums_its_completed_steps() {
        let profile = HarnessProfile::dsh();
        let output = br#"{"type":"status","phase":"step_start","usage":{"inputTokens":999}}
{"type":"status","phase":"step_end","usage":{"inputTokens":100,"outputTokens":10,"cacheReadTokens":50}}
{"type":"status","phase":"step_end","usage":{"inputTokens":200,"outputTokens":20}}"#;
        let outcome = parse(&profile, output);
        assert_eq!(
            outcome.usage.prompt.reported_input,
            Reported::Reported(Tokens::new(300))
        );
        assert_eq!(
            outcome.usage.output,
            Reported::Reported(OutputTokens::new(30))
        );
        assert_eq!(
            outcome.usage.prompt.cache_read,
            Reported::Reported(CachedTokens::new(50))
        );
        assert_eq!(
            outcome.usage.prompt.fresh(),
            Reported::Reported(FreshTokens::new(300))
        );
    }

    #[test]
    fn text_output_has_no_usage_and_no_fabricated_request() {
        let mut profile = HarnessProfile::claude();
        profile.output_shape = OutputShape::Text;
        let outcome = parse(&profile, b"plain text output");
        assert_eq!(outcome.usage, empty_report(&profile));
        assert!(per_request_usage(&profile, b"plain text output").is_empty());
    }

    #[test]
    fn the_percent_helpers_are_wide_and_do_not_invent_a_share() {
        assert_eq!(
            cache_hit_bps(CachedTokens::new(1), FreshTokens::new(0)),
            BasisPoints::new(10_000)
        );
        assert_eq!(
            cache_hit_bps(CachedTokens::ZERO, FreshTokens::ZERO),
            None,
            "a share of nothing is undefined"
        );
        assert_eq!(
            cache_percent(CachedTokens::ZERO, FreshTokens::new(100)),
            BasisPoints::ZERO
        );
        let profile = HarnessProfile::claude();
        assert_eq!(
            provider_cache_percent(&profile, CachedTokens::new(4000), Tokens::new(1000)),
            BasisPoints::new(8_000).expect("valid"),
            "exclusive: cached/(input+cached)"
        );
        let codex = HarnessProfile::codex();
        assert_eq!(
            provider_cache_percent(&codex, CachedTokens::new(900), Tokens::new(1200)),
            BasisPoints::new(7_500).expect("valid"),
            "inclusive: cached/input"
        );
    }

    #[test]
    fn parse_checked_refuses_a_version_outside_the_declared_range() {
        let profile = HarnessProfile::codex();
        let old = crate::newtypes::HarnessVersion::new("0.1.0").expect("valid");
        assert!(matches!(
            parse_checked(&profile, &old, b"{}"),
            Err(Error::Unsupported { .. })
        ));
        let new = crate::newtypes::HarnessVersion::new("1.0.0").expect("valid");
        assert!(parse_checked(&profile, &new, b"{}").is_ok());
    }
}
