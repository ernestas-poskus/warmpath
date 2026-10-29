//! Reuse measurement keys for caches a caller deliberately does not build.
//!
//! A semantic cache and an exact-request replay cache are both unsafe to serve from in a
//! code-modifying pipeline: a wrong hit is silent, and it corrupts usage-denominated accounting.
//! The measurement that must precede either decision is cheap and safe, and it lives here:
//!
//! - [`exact_request_key`] digests the identity of one request — prefix, declared tool surface,
//!   route, model, effort, and the rendered payload — so byte-identical requests can be counted;
//! - [`dynamic_tail_key`] digests the normalized dynamic tail of a rendered document (the static
//!   head removed) so a near-duplicate rate is visible as a lower bound for a semantic match;
//! - [`summarize`] counts duplicate groups, duplicate runs, and zero-usage observations.
//!
//! Nothing here stores a payload, performs an embedding lookup, or can serve a request. The keys
//! are digests, and the summary is a measurement, never a hit.
//!
//! # Digests, not text
//!
//! Both keys are lowercase SHA-256 digests over length-delimited parts, so the record a caller
//! persists never contains the prompt, the plan, or the brief. A changed byte in any identity fact
//! changes the key; absent facts are `None`, never a guessed empty value.

use std::collections::BTreeMap;

/// One measurement observation: the two keys a caller recorded for a run, plus its zero-usage flag.
///
/// The dispatch plane maps its own persisted record onto this borrowed view so the aggregation
/// stays domain-neutral and allocation-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowObservation<'a> {
    /// Digest of the exact request identity.
    pub request_key: &'a str,
    /// Digest of the normalized dynamic tail.
    pub tail_key: &'a str,
    /// Whether a successful run reported zero prompt, cache, and output tokens.
    pub zero_usage: bool,
}

/// Counts over a set of shadow observations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShadowSummary {
    /// Observed runs.
    pub runs: u32,
    /// Keys that appeared two or more times.
    pub exact_duplicate_groups: u32,
    /// Runs whose exact request key appeared two or more times.
    pub exact_duplicate_runs: u32,
    /// Tail keys that appeared two or more times.
    pub tail_duplicate_groups: u32,
    /// Runs whose tail key appeared two or more times.
    pub tail_duplicate_runs: u32,
    /// Runs that reported zero prompt, cache, and output tokens despite succeeding.
    pub zero_usage_runs: u32,
}

/// Build the exact request key a replay cache would use.
///
/// Every part is trimmed; an absent tool surface or effort contributes no bytes. The digest is
/// length-delimited, so two different part sequences cannot collide by concatenation.
#[must_use]
pub fn exact_request_key(
    prefix_digest: &str,
    tool_digest: Option<&str>,
    provider: &str,
    model: &str,
    effort: Option<&str>,
    payload_digest: &str,
) -> String {
    let normalize = |value: &str| value.trim().to_owned();
    crate::prefix::digest(&[
        prefix_digest.as_bytes(),
        normalize(tool_digest.unwrap_or_default()).as_bytes(),
        normalize(provider).as_bytes(),
        normalize(model).as_bytes(),
        normalize(effort.unwrap_or_default()).as_bytes(),
        payload_digest.as_bytes(),
    ])
    .to_string()
}

/// Build the dynamic tail key: strip the static head, lowercase, collapse punctuation and
/// whitespace, and digest.
///
/// This is an exact normalized-key comparison, not an embedding. It is a lower bound on how often
/// two runs ask "the same" dynamic question; a semantic match would be a superset, so a report
/// must present it as a lower bound rather than imply one was measured.
#[must_use]
pub fn dynamic_tail_key(document: &str, static_prefix: &str) -> String {
    let tail = document.strip_prefix(static_prefix).unwrap_or(document);
    let mut normalized = String::with_capacity(tail.len());
    let mut pending_space = false;
    for character in tail.chars() {
        if character.is_alphanumeric() {
            if pending_space && !normalized.is_empty() {
                normalized.push(' ');
            }
            pending_space = false;
            for lower in character.to_lowercase() {
                normalized.push(lower);
            }
        } else {
            pending_space = true;
        }
    }
    crate::prefix::digest(&[normalized.as_bytes()]).to_string()
}

/// Summarize a set of observations.
#[must_use]
pub fn summarize(observations: &[ShadowObservation<'_>]) -> ShadowSummary {
    let mut exact: BTreeMap<&str, u32> = BTreeMap::new();
    let mut tails: BTreeMap<&str, u32> = BTreeMap::new();
    let mut summary = ShadowSummary {
        runs: u32::try_from(observations.len()).unwrap_or(u32::MAX),
        ..ShadowSummary::default()
    };
    for observation in observations {
        *exact.entry(observation.request_key).or_default() += 1;
        *tails.entry(observation.tail_key).or_default() += 1;
        if observation.zero_usage {
            summary.zero_usage_runs += 1;
        }
    }
    for count in exact.values().copied().filter(|count| *count > 1) {
        summary.exact_duplicate_groups += 1;
        summary.exact_duplicate_runs = summary.exact_duplicate_runs.saturating_add(count);
    }
    for count in tails.values().copied().filter(|count| *count > 1) {
        summary.tail_duplicate_groups += 1;
        summary.tail_duplicate_runs = summary.tail_duplicate_runs.saturating_add(count);
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation<'a>(request: &'a str, tail: &'a str) -> ShadowObservation<'a> {
        ShadowObservation {
            request_key: request,
            tail_key: tail,
            zero_usage: false,
        }
    }

    #[test]
    fn the_request_key_changes_with_every_identity_fact() {
        let base = exact_request_key("p", Some("t"), "codex", "m", Some("high"), "b");
        assert_eq!(
            base,
            exact_request_key("p", Some("t"), "codex", "m", Some("high"), "b")
        );
        for changed in [
            exact_request_key("p2", Some("t"), "codex", "m", Some("high"), "b"),
            exact_request_key("p", Some("t2"), "codex", "m", Some("high"), "b"),
            exact_request_key("p", None, "codex", "m", Some("high"), "b"),
            exact_request_key("p", Some("t"), "openai", "m", Some("high"), "b"),
            exact_request_key("p", Some("t"), "codex", "m2", Some("high"), "b"),
            exact_request_key("p", Some("t"), "codex", "m", Some("low"), "b"),
            exact_request_key("p", Some("t"), "codex", "m", Some("high"), "b2"),
        ] {
            assert_ne!(base, changed);
        }
    }

    #[test]
    fn the_tail_key_ignores_case_whitespace_and_punctuation() {
        let prefix = "## static head\n";
        let a = dynamic_tail_key(&format!("{prefix}Fix the Parser:  it fails."), prefix);
        let b = dynamic_tail_key(&format!("{prefix}fix the parser it fails"), prefix);
        assert_eq!(a, b);
        assert_ne!(
            a,
            dynamic_tail_key(&format!("{prefix}Fix the lexer; it fails."), prefix)
        );
    }

    #[test]
    fn the_tail_key_drops_the_static_head() {
        let prefix = "identical static instructions";
        let a = dynamic_tail_key(&format!("{prefix}plan one"), prefix);
        let b = dynamic_tail_key(&format!("{prefix}plan one"), prefix);
        let c = dynamic_tail_key(&format!("{prefix}plan two"), prefix);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn duplicate_groups_count_runs_not_pairs() {
        let records = [
            observation("a", "t1"),
            observation("a", "t1"),
            observation("a", "t1"),
            observation("b", "t2"),
            observation("c", "t2"),
        ];
        let summary = summarize(&records);
        assert_eq!(summary.runs, 5);
        assert_eq!(summary.exact_duplicate_groups, 1);
        assert_eq!(summary.exact_duplicate_runs, 3);
        assert_eq!(summary.tail_duplicate_groups, 2);
        assert_eq!(summary.tail_duplicate_runs, 5);
        assert_eq!(summary.zero_usage_runs, 0);
    }

    #[test]
    fn a_zero_usage_run_is_counted() {
        let records = [ShadowObservation {
            request_key: "a",
            tail_key: "t",
            zero_usage: true,
        }];
        let summary = summarize(&records);
        assert_eq!(summary.zero_usage_runs, 1);
    }
}
