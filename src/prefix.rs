//! Canonical prompt-prefix hashing and change classification.
//!
//! A provider prompt cache only hits while the request prefix is byte-identical. These helpers
//! compute a **length-delimited** SHA-256 over the static parts of a prefix (so `["ab","c"]` and
//! `["a","bc"]` differ), canonicalize the mutable-looking parts (tool definitions, JSON, framing
//! text) into a stable form, and name exactly what changed between two prefixes. A
//! [`ToolManifest`] folds a tool surface into one order-insensitive, name-sensitive digest, and
//! [`check_append_only`] turns the request-sequence invariant into a result.

// Canonicalization parses untrusted JSON into an untyped tree by design; the typed values a
// caller consumes are constructed afterwards. The crate's disallowed-untyped-value lint is for
// contract types, not for a JSON normalizer.
#![allow(clippy::disallowed_types)]

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::error::Error;
use crate::newtypes::PrefixDigest;

/// The relationship between a previous and a current prefix digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrefixChange {
    /// No previous digest exists: this is the first run.
    FirstRun,
    /// The digest matches the previous one.
    Unchanged,
    /// The digest differs from the previous one.
    Changed,
}

impl PrefixChange {
    /// The stable spelling.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            PrefixChange::FirstRun => "first_run",
            PrefixChange::Unchanged => "unchanged",
            PrefixChange::Changed => "changed",
        }
    }

    /// Whether the prefix moved.
    #[must_use]
    pub const fn changed(self) -> bool {
        matches!(self, PrefixChange::Changed)
    }
}

/// A named tool and its opaque rendered definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolEntry {
    /// The tool's name.
    pub name: String,
    /// Its rendered schema or description, exactly as it appears in the prompt.
    pub definition: String,
}

impl ToolEntry {
    /// Build an entry.
    #[must_use]
    pub fn new(name: impl Into<String>, definition: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            definition: definition.into(),
        }
    }
}

/// Compute the lowercase SHA-256 hex digest over parts, each prefixed with its 8-byte big-endian
/// length.
#[must_use]
pub fn digest(parts: &[&[u8]]) -> PrefixDigest {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    PrefixDigest::parse(hex).expect("sha256 renders as 64 lowercase hex characters")
}

/// Classify the relationship between an optional previous digest and the current one.
#[must_use]
pub fn classify(previous: Option<&PrefixDigest>, current: &PrefixDigest) -> PrefixChange {
    match previous {
        None => PrefixChange::FirstRun,
        Some(previous) if previous == current => PrefixChange::Unchanged,
        Some(_) => PrefixChange::Changed,
    }
}

/// A diagnostic fingerprint of prefix bytes (FNV-1a 64), for messages rather than security.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PrefixFingerprint(String);

impl PrefixFingerprint {
    /// Fingerprint some bytes.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
        Self(format!("{hash:016x}"))
    }

    /// The fingerprint text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PrefixFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Where two prefixes first differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixDifference {
    /// Fingerprint of the previous prefix.
    pub previous_fingerprint: PrefixFingerprint,
    /// Fingerprint of the current prefix.
    pub current_fingerprint: PrefixFingerprint,
    /// Index of the first differing byte, or the common length when one is a prefix of the other.
    pub first_differing_byte: usize,
    /// Length of the previous prefix in bytes.
    pub previous_len: usize,
    /// Length of the current prefix in bytes.
    pub current_len: usize,
}

impl PrefixDifference {
    /// Whether the difference is confined to the tail (one prefix extends the other).
    ///
    /// True for both an extension and a truncation: it says the shared bytes are a prefix of
    /// both. Use [`PrefixDifference::is_extension`] when the direction matters.
    #[must_use]
    pub fn is_append_only(&self) -> bool {
        self.first_differing_byte == self.previous_len.min(self.current_len)
    }

    /// Whether the current prefix extends the previous one without changing or dropping bytes.
    ///
    /// This is the request-sequence invariant a provider prefix cache depends on: an identical
    /// repeat counts (nothing changed), a longer current prefix counts, a shorter one does not.
    #[must_use]
    pub fn is_extension(&self) -> bool {
        self.first_differing_byte == self.previous_len && self.current_len >= self.previous_len
    }
}

/// Compare two prefixes, returning `None` when they are identical.
#[must_use]
pub fn compare_prefixes(previous: &str, current: &str) -> Option<PrefixDifference> {
    if previous == current {
        return None;
    }
    Some(PrefixDifference {
        previous_fingerprint: PrefixFingerprint::of(previous.as_bytes()),
        current_fingerprint: PrefixFingerprint::of(current.as_bytes()),
        first_differing_byte: previous
            .bytes()
            .zip(current.bytes())
            .position(|(left, right)| left != right)
            .unwrap_or(previous.len().min(current.len())),
        previous_len: previous.len(),
        current_len: current.len(),
    })
}

/// One adjacent pair in a request sequence that was not an append-only extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixVariance {
    /// The 1-based index of the step that broke the invariant: `steps[step - 1]` does not extend
    /// `steps[step - 2]`.
    pub step: usize,
    /// The difference between the previous and the current step.
    pub difference: PrefixDifference,
}

/// Verify that every step extends its predecessor without changing or dropping earlier bytes.
///
/// A request sequence is append-only when each rendered request begins with the exact bytes of
/// the request before it. An identical repeat is append-only; a rewritten byte, an insertion
/// before the tail, or a shortened request is a variance. This is the assertion a caller runs
/// over a scripted or recorded step sequence before trusting a cache-hit expectation.
///
/// # Errors
///
/// The first violating adjacent pair, as [`PrefixVariance`].
pub fn check_append_only<S: AsRef<str>>(steps: &[S]) -> Result<(), PrefixVariance> {
    for (index, pair) in steps.windows(2).enumerate() {
        let previous = pair[0].as_ref();
        let current = pair[1].as_ref();
        if let Some(difference) = compare_prefixes(previous, current)
            && !difference.is_extension()
        {
            return Err(PrefixVariance {
                step: index + 1,
                difference,
            });
        }
    }
    Ok(())
}

/// Sort tools by name and refuse ambiguous definitions; definitions stay opaque.
///
/// # Errors
///
/// [`Error::Malformed`] when two entries share a name but not a definition.
pub fn canonical_tool_surface(entries: &[ToolEntry]) -> Result<String, Error> {
    let mut tools = std::collections::BTreeMap::new();
    for entry in entries {
        if let Some(previous) = tools.insert(&entry.name, &entry.definition)
            && previous != &entry.definition
        {
            return Err(Error::Malformed {
                detail: format!("conflicting duplicate tool `{}`", entry.name),
            });
        }
    }
    let mut out = String::new();
    for definition in tools.values() {
        out.push_str(definition.trim_end_matches('\n'));
        out.push('\n');
    }
    Ok(out)
}

/// A canonical tool surface and its digest.
///
/// Entries are sorted by name; byte-identical duplicates collapse; a name carrying two different
/// definitions is refused. The digest hashes `(name, definition)` pairs, so it is insensitive to
/// input order but changes when a tool is renamed, added, removed, or redefined. Definitions stay
/// opaque: the caller owns their rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolManifest {
    entries: Vec<ToolEntry>,
    digest: PrefixDigest,
}

impl ToolManifest {
    /// Canonicalize a tool surface.
    ///
    /// # Errors
    ///
    /// [`Error::Malformed`] when two entries share a name but not a definition.
    pub fn new(entries: &[ToolEntry]) -> Result<Self, Error> {
        let mut sorted = entries.to_vec();
        sorted.sort_by(|left, right| left.name.cmp(&right.name));
        sorted.dedup();
        for pair in sorted.windows(2) {
            if pair[0].name == pair[1].name {
                return Err(Error::Malformed {
                    detail: format!("conflicting duplicate tool `{}`", pair[0].name),
                });
            }
        }
        let parts: Vec<&[u8]> = sorted
            .iter()
            .flat_map(|entry| [entry.name.as_bytes(), entry.definition.as_bytes()])
            .collect();
        let digest = digest(&parts);
        Ok(Self {
            entries: sorted,
            digest,
        })
    }

    /// The sorted entries.
    #[must_use]
    pub fn entries(&self) -> &[ToolEntry] {
        &self.entries
    }

    /// The digest over the ordered `(name, definition)` pairs.
    #[must_use]
    pub fn digest(&self) -> &PrefixDigest {
        &self.digest
    }
}

/// Compact JSON with recursively sorted object keys (not RFC 8785 number normalization).
///
/// # Errors
///
/// [`Error::Malformed`] when the input is not JSON or cannot be rendered.
pub fn canonical_json(text: &str) -> Result<String, Error> {
    fn sorted(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let ordered: std::collections::BTreeMap<_, _> = map
                    .into_iter()
                    .map(|(key, value)| (key, sorted(value)))
                    .collect();
                serde_json::Value::Object(ordered.into_iter().collect())
            }
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.into_iter().map(sorted).collect())
            }
            other => other,
        }
    }
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|error| Error::Malformed {
            detail: format!("canonical JSON input: {error}"),
        })?;
    let mut json = serde_json::to_string(&sorted(value)).map_err(|error| Error::Malformed {
        detail: format!("canonical JSON render: {error}"),
    })?;
    json.push('\n');
    Ok(json)
}

/// Normalize framing text: line endings, trailing whitespace, and blank-line runs.
#[must_use]
pub fn canonical_text(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<&str> = Vec::new();
    for line in normalized.lines().map(str::trim_end) {
        if line.is_empty() && lines.last() == Some(&"") {
            continue;
        }
        lines.push(line);
    }
    while lines.last() == Some(&"") {
        lines.pop();
    }
    if lines.iter().all(|line| line.is_empty()) {
        String::new()
    } else {
        let mut out = lines.join("\n");
        out.push('\n');
        out
    }
}

/// The immutable identity of a rendered prefix, kept beside its digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixFingerprintParts {
    /// Digest of the canonical static parts.
    pub digest: PrefixDigest,
    /// Fingerprint of the rendered prefix, for a message.
    pub render: PrefixFingerprint,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_delimiting_distinguishes_absence_from_empty_and_prevents_collisions() {
        assert_ne!(digest(&[]), digest(&[b""]));
        assert_ne!(digest(&[b"ab", b"c"]), digest(&[b"a", b"bc"]));
        assert_ne!(digest(&[b"a", b"b"]), digest(&[b"b", b"a"]));
        assert_eq!(digest(&[b"a"]), digest(&[b"a"]));
    }

    #[test]
    fn change_classification_names_the_three_cases() {
        let current = digest(&[b"a"]);
        assert_eq!(classify(None, &current), PrefixChange::FirstRun);
        assert_eq!(classify(Some(&current), &current), PrefixChange::Unchanged);
        let other = digest(&[b"b"]);
        assert_eq!(classify(Some(&current), &other), PrefixChange::Changed);
        assert!(PrefixChange::Changed.changed());
    }

    #[test]
    fn duplicate_tools_must_agree_and_definitions_stay_opaque() {
        let entries = vec![
            ToolEntry::new("b", "second\n\n"),
            ToolEntry::new("a", "first\n"),
            ToolEntry::new("a", "first\n"),
        ];
        let canonical = canonical_tool_surface(&entries).expect("agreeing duplicates are fine");
        assert_eq!(canonical, "first\nsecond\n");
        let conflicting = vec![ToolEntry::new("a", "one"), ToolEntry::new("a", "two")];
        assert!(canonical_tool_surface(&conflicting).is_err());
    }

    #[test]
    fn canonical_json_sorts_keys_recursively() {
        let canonical =
            canonical_json(r#"{"b":1,"a":{"d":2,"c":[{"z":1,"y":2}]}}"#).expect("valid json");
        assert_eq!(
            canonical,
            "{\"a\":{\"c\":[{\"y\":2,\"z\":1}],\"d\":2},\"b\":1}\n"
        );
        assert!(canonical_json("{not json").is_err());
    }

    #[test]
    fn canonical_text_normalizes_framing_without_touching_content() {
        assert_eq!(canonical_text("a  \r\n\r\n\r\nb\r\n"), "a\n\nb\n");
        assert_eq!(canonical_text("\n\n"), "");
        assert_eq!(canonical_text(""), "");
    }

    #[test]
    fn a_difference_names_the_first_byte_and_the_append_case() {
        assert!(compare_prefixes("same", "same").is_none());
        let difference = compare_prefixes("abc", "abd").expect("differs");
        assert_eq!(difference.first_differing_byte, 2);
        assert!(!difference.is_append_only());
        let appended = compare_prefixes("abc", "abcdef").expect("differs");
        assert_eq!(appended.first_differing_byte, 3);
        assert!(appended.is_append_only());
        assert_ne!(
            difference.previous_fingerprint,
            difference.current_fingerprint
        );
    }

    #[test]
    fn a_manifest_digest_is_order_insensitive_but_name_sensitive() {
        let entries = vec![
            ToolEntry::new("read", "read a file"),
            ToolEntry::new("edit", "edit a file"),
        ];
        let mut reversed = entries.clone();
        reversed.reverse();
        let forward = ToolManifest::new(&entries).expect("distinct names");
        let backward = ToolManifest::new(&reversed).expect("distinct names");
        assert_eq!(forward.digest(), backward.digest(), "order is not content");
        assert_eq!(
            forward.entries()[0].name,
            "edit",
            "entries are sorted by name"
        );

        let renamed = ToolManifest::new(&[
            ToolEntry::new("read", "read a file"),
            ToolEntry::new("write", "edit a file"),
        ])
        .expect("distinct names");
        assert_ne!(
            forward.digest(),
            renamed.digest(),
            "a rename is a different surface"
        );

        let redefined = ToolManifest::new(&[
            ToolEntry::new("read", "read a file"),
            ToolEntry::new("edit", "edit two files"),
        ])
        .expect("distinct names");
        assert_ne!(forward.digest(), redefined.digest());
    }

    #[test]
    fn a_manifest_collapses_identical_duplicates_and_refuses_conflicts() {
        let duplicate = ToolManifest::new(&[
            ToolEntry::new("read", "read a file"),
            ToolEntry::new("read", "read a file"),
        ])
        .expect("agreeing duplicates collapse");
        assert_eq!(duplicate.entries().len(), 1);
        assert!(
            ToolManifest::new(&[ToolEntry::new("read", "one"), ToolEntry::new("read", "two")])
                .is_err()
        );
    }

    #[test]
    fn append_only_accepts_growth_and_identical_repeats() {
        let steps = ["head\n", "head\nturn one\n", "head\nturn one\nturn two\n"];
        assert_eq!(check_append_only(&steps), Ok(()));
        let repeated = ["head\n", "head\n"];
        assert_eq!(check_append_only(&repeated), Ok(()));
        assert_eq!(check_append_only::<&str>(&[]), Ok(()));
    }

    #[test]
    fn append_only_names_the_first_rewritten_step() {
        let steps = [
            "head\n",
            "head\nturn one\n",
            "head\nturn X\n",
            "head\nturn X\nturn three\n",
        ];
        let variance = check_append_only(&steps).expect_err("step 2 rewrote the tail");
        assert_eq!(variance.step, 2);
        assert_eq!(variance.difference.first_differing_byte, 10);
        assert!(!variance.difference.is_extension());
    }

    #[test]
    fn a_shorter_step_is_a_variance_even_though_it_is_append_only_in_both_directions() {
        let steps = ["head\nturn one\n", "head\n"];
        let variance = check_append_only(&steps).expect_err("a truncated request cannot hit");
        assert_eq!(variance.step, 1);
        assert!(variance.difference.is_append_only());
        assert!(!variance.difference.is_extension());
    }
}
