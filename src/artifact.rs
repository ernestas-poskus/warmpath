//! Async, path-confined storage for reuse artifacts.
//!
//! Only this module performs I/O, and it does so with `tokio::fs` and
//! `tokio::io::AsyncWriteExt`: the crate never starts a runtime and never calls blocking `std::fs`
//! from an async path. An artifact is opaque bytes with a **name**, an **identity**, and a
//! **validation** step:
//!
//! - the name is built from a subject, a digest, an optional variant, and an extension, and every
//!   component is validated so a name can never escape the store root;
//! - a publish is atomic and no-clobber: the bytes go to a temporary file in the same directory,
//!   are synced, and are linked into place with a hard link that fails if the name exists;
//! - a reuse reads under a bound, validates the bytes as untrusted input, and compares the recorded
//!   identity. A mismatch is returned as **stale**, never silently reused and never overwritten.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::Error;
use crate::newtypes::{
    ArtifactDigest, BasisPoints, ContractId, ContractVersion, HarnessVersion, ProfileId,
    ProfileVersion, SubjectId, Tokens,
};

/// The default bound for an artifact read (one mebibyte).
pub const DEFAULT_MAX_ARTIFACT_BYTES: usize = 1024 * 1024;

/// The artifact schema version this build writes and accepts.
pub const ARTIFACT_SCHEMA_VERSION: u32 = 1;

/// A validated artifact file name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactName {
    subject: SubjectId,
    digest: ArtifactDigest,
    variant: Option<String>,
    extension: String,
}

impl ArtifactName {
    /// Build a name: `subject-<digest[..8]>[-variant].extension`.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidName`] when a component is empty, too long, contains a path separator, a
    /// control character, or is `.`/`..`.
    pub fn new(
        subject: SubjectId,
        digest: ArtifactDigest,
        variant: Option<&str>,
        extension: &str,
    ) -> Result<Self, Error> {
        let variant = match variant {
            Some(variant) => Some(bounded_component("variant", variant, 64)?),
            None => None,
        };
        let extension = bounded_component("extension", extension, 16)?;
        if !extension
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
        {
            return Err(Error::InvalidName {
                kind: "extension",
                value: extension,
                detail: "must be ASCII alphanumeric",
            });
        }
        Ok(Self {
            subject,
            digest,
            variant,
            extension,
        })
    }

    /// The rendered file name.
    #[must_use]
    pub fn render(&self) -> String {
        let short = self.digest.short();
        match &self.variant {
            Some(variant) => format!("{}-{short}-{variant}.{}", self.subject, self.extension),
            None => format!("{}-{short}.{}", self.subject, self.extension),
        }
    }

    /// The digest the name is keyed by.
    #[must_use]
    pub fn digest(&self) -> &ArtifactDigest {
        &self.digest
    }
}

/// Validate one path component.
fn bounded_component(kind: &'static str, value: &str, max: usize) -> Result<String, Error> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidName {
            kind,
            value: value.to_owned(),
            detail: "must not be empty",
        });
    }
    if trimmed.len() > max {
        return Err(Error::InvalidName {
            kind,
            value: value.to_owned(),
            detail: "is longer than its bound",
        });
    }
    if trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.chars().any(char::is_control)
    {
        return Err(Error::InvalidName {
            kind,
            value: value.to_owned(),
            detail: "must not contain a path separator or control character",
        });
    }
    Ok(trimmed.to_owned())
}

/// The identity an artifact's bytes were produced under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactIdentity {
    /// The crate version that wrote the artifact.
    pub crate_version: String,
    /// The metric contract the artifact was produced for.
    pub contract_id: ContractId,
    /// The contract's version.
    pub contract_version: ContractVersion,
    /// The harness profile.
    pub profile_id: ProfileId,
    /// The profile's version.
    pub profile_version: ProfileVersion,
    /// The harness version the input came from.
    pub harness_version: HarnessVersion,
    /// The digest of the input the artifact was computed from.
    pub input_digest: ArtifactDigest,
}

impl ArtifactIdentity {
    /// Build an identity.
    #[must_use]
    pub fn new(
        contract_id: ContractId,
        contract_version: ContractVersion,
        profile_id: ProfileId,
        profile_version: ProfileVersion,
        harness_version: HarnessVersion,
        input_digest: ArtifactDigest,
    ) -> Self {
        Self {
            crate_version: crate::CRATE_VERSION.to_owned(),
            contract_id,
            contract_version,
            profile_id,
            profile_version,
            harness_version,
            input_digest,
        }
    }
}

/// What a publish or reuse attempt decided.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ArtifactDecision {
    /// The existing artifact was validated and reused.
    Reused,
    /// A new artifact was published.
    Published,
    /// The name already exists; nothing was overwritten.
    Conflict,
    /// An existing artifact exists but is not usable for this identity or content.
    Stale {
        /// Why it was not reused.
        reason: String,
    },
}

/// A validated metric artifact's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedMetric {
    /// The contract id the artifact claims.
    pub contract_id: ContractId,
    /// The contract version.
    pub contract_version: ContractVersion,
    /// The measured numerator.
    pub numerator: Tokens,
    /// The measured denominator.
    pub denominator: Tokens,
    /// The share, which must not exceed 100 %.
    pub bps: BasisPoints,
}

/// Validate untrusted metric-artifact bytes.
///
/// Checks the schema version, the required fields, `numerator <= denominator`, and a basis-point
/// value within `0..=10_000`; anything else is [`Error::Malformed`]. This is the validation a reuse
/// must pass before an artifact is trusted.
///
/// # Errors
///
/// [`Error::Malformed`] when the document does not satisfy the contract, or
/// [`Error::Unsupported`] for a newer schema version.
pub fn validate_metric_artifact(bytes: &[u8]) -> Result<ValidatedMetric, Error> {
    #[derive(Deserialize)]
    struct Raw {
        schema_version: u32,
        contract_id: String,
        contract_version: u32,
        numerator: u64,
        denominator: u64,
        basis_points: u16,
    }
    let raw: Raw = serde_json::from_slice(bytes).map_err(|error| Error::Malformed {
        detail: format!("metric artifact: {error}"),
    })?;
    if raw.schema_version > ARTIFACT_SCHEMA_VERSION {
        return Err(Error::Unsupported {
            harness: "artifact",
            version: raw.schema_version.to_string(),
        });
    }
    if raw.numerator > raw.denominator {
        return Err(Error::Malformed {
            detail: "numerator exceeds denominator".to_owned(),
        });
    }
    let contract_id = ContractId::new(raw.contract_id)?;
    let bps = BasisPoints::new(raw.basis_points).ok_or_else(|| Error::Malformed {
        detail: "basis points exceed 10 000".to_owned(),
    })?;
    Ok(ValidatedMetric {
        contract_id,
        contract_version: ContractVersion::new(raw.contract_version),
        numerator: Tokens::new(raw.numerator),
        denominator: Tokens::new(raw.denominator),
        bps,
    })
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// An async, path-confined artifact store.
#[derive(Debug, Clone)]
pub struct ArtifactStore {
    root: PathBuf,
}

impl ArtifactStore {
    /// Open (and create) a store rooted at `root`.
    ///
    /// # Errors
    ///
    /// Any filesystem error from creating the directory.
    pub async fn new(root: impl Into<PathBuf>) -> Result<Self, Error> {
        let root = root.into();
        tokio::fs::create_dir_all(&root).await?;
        Ok(Self { root })
    }

    /// The store root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The absolute path for a name, confined to the root.
    ///
    /// # Errors
    ///
    /// [`Error::Path`] when the joined path would leave the root (defence in depth: names are
    /// already validated, so this can only fire on a programming error).
    pub fn path_for(&self, name: &ArtifactName) -> Result<PathBuf, Error> {
        let path = self.root.join(name.render());
        if !path.starts_with(&self.root) {
            return Err(Error::Path {
                path: path.display().to_string(),
                detail: "the artifact path leaves the store root".to_owned(),
            });
        }
        Ok(path)
    }

    /// Read an artifact under a byte bound.
    ///
    /// Returns `Ok(None)` when no artifact exists. An artifact larger than `max_bytes` is refused
    /// rather than partially read: a truncated document is not the artifact that was validated.
    ///
    /// # Errors
    ///
    /// Any filesystem error, or [`Error::InvalidTarget`] when the artifact exceeds the bound.
    pub async fn read_bounded(
        &self,
        name: &ArtifactName,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, Error> {
        let path = self.path_for(name)?;
        let file = match tokio::fs::File::open(&path).await {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        let limit = u64::try_from(max_bytes)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let read = file.take(limit).read_to_end(&mut bytes).await?;
        if read > max_bytes {
            return Err(Error::InvalidTarget {
                detail: format!(
                    "artifact `{}` exceeds the {max_bytes}-byte read bound",
                    name.render()
                ),
            });
        }
        Ok(Some(bytes))
    }

    /// Publish bytes atomically and without clobbering an existing name.
    ///
    /// # Errors
    ///
    /// Any filesystem error other than the no-clobber conflict, which is
    /// [`ArtifactDecision::Conflict`].
    pub async fn publish_once(
        &self,
        name: &ArtifactName,
        bytes: &[u8],
    ) -> Result<ArtifactDecision, Error> {
        let final_path = self.path_for(name)?;
        let temp_path = self.temp_path(name);
        {
            let mut file = tokio::fs::File::create(&temp_path).await?;
            file.write_all(bytes).await?;
            file.flush().await?;
            file.sync_all().await?;
        }
        match tokio::fs::hard_link(&temp_path, &final_path).await {
            Ok(()) => {
                let _ = tokio::fs::remove_file(&temp_path).await;
                Ok(ArtifactDecision::Published)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = tokio::fs::remove_file(&temp_path).await;
                Ok(ArtifactDecision::Conflict)
            }
            Err(error) => {
                let _ = tokio::fs::remove_file(&temp_path).await;
                Err(error.into())
            }
        }
    }

    /// Reuse a validated artifact when its identity matches, otherwise publish a new one.
    ///
    /// An existing artifact that fails validation or carries a different identity is
    /// [`ArtifactDecision::Stale`]: never silently reused, never overwritten.
    ///
    /// # Errors
    ///
    /// Filesystem errors, or the provenance write failing.
    pub async fn reuse_or_publish(
        &self,
        name: &ArtifactName,
        identity: &ArtifactIdentity,
        bytes: &[u8],
        validate: impl Fn(&[u8]) -> Result<(), Error>,
    ) -> Result<ArtifactDecision, Error> {
        let existing = self.read_bounded(name, DEFAULT_MAX_ARTIFACT_BYTES).await?;
        if let Some(existing) = existing {
            if let Err(error) = validate(&existing) {
                return Ok(ArtifactDecision::Stale {
                    reason: format!("validation failed: {error}"),
                });
            }
            match self.read_identity(name).await {
                Ok(Some(recorded)) if &recorded == identity => return Ok(ArtifactDecision::Reused),
                Ok(Some(_)) => {
                    return Ok(ArtifactDecision::Stale {
                        reason: "the recorded identity differs from this build's".to_owned(),
                    });
                }
                Ok(None) => {
                    return Ok(ArtifactDecision::Stale {
                        reason: "no provenance was recorded beside the artifact".to_owned(),
                    });
                }
                Err(error) => {
                    return Ok(ArtifactDecision::Stale {
                        reason: format!("provenance unreadable: {error}"),
                    });
                }
            }
        }
        validate(bytes)?;
        match self.publish_once(name, bytes).await? {
            ArtifactDecision::Published => {
                self.write_identity(name, identity).await?;
                Ok(ArtifactDecision::Published)
            }
            ArtifactDecision::Conflict => Ok(ArtifactDecision::Conflict),
            other => Ok(other),
        }
    }

    /// The provenance document's path for a name.
    fn identity_path(&self, name: &ArtifactName) -> PathBuf {
        self.root.join(format!("{}.meta", name.render()))
    }

    /// A per-call unique temporary path in the same directory.
    fn temp_path(&self, name: &ArtifactName) -> PathBuf {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        self.root.join(format!(
            ".{}.tmp-{}-{counter}",
            name.render(),
            std::process::id()
        ))
    }

    /// Read the recorded identity beside an artifact.
    async fn read_identity(&self, name: &ArtifactName) -> Result<Option<ArtifactIdentity>, Error> {
        match tokio::fs::read(self.identity_path(name)).await {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(identity) => Ok(Some(identity)),
                Err(error) => Err(Error::Malformed {
                    detail: format!("artifact provenance: {error}"),
                }),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Write the identity beside an artifact through a temp-then-rename.
    async fn write_identity(
        &self,
        name: &ArtifactName,
        identity: &ArtifactIdentity,
    ) -> Result<(), Error> {
        let bytes = serde_json::to_vec(identity).map_err(|error| Error::Malformed {
            detail: format!("artifact provenance render: {error}"),
        })?;
        let final_path = self.identity_path(name);
        let temp_path = self.root.join(format!(
            ".{}.meta.tmp-{}-{}",
            name.render(),
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        {
            let mut file = tokio::fs::File::create(&temp_path).await?;
            file.write_all(&bytes).await?;
            file.flush().await?;
            file.sync_all().await?;
        }
        tokio::fs::rename(&temp_path, &final_path).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(subject: &str, digest: &str) -> ArtifactName {
        ArtifactName::new(
            SubjectId::new(subject).expect("valid subject"),
            ArtifactDigest::parse(digest).expect("valid digest"),
            None,
            "json",
        )
        .expect("valid artifact name")
    }

    fn digest(byte: char) -> String {
        byte.to_string().repeat(64)
    }

    fn identity() -> ArtifactIdentity {
        ArtifactIdentity::new(
            ContractId::new("aics").expect("valid contract"),
            ContractVersion::new(1),
            ProfileId::new("codex-v1").expect("valid profile"),
            ProfileVersion::new(1),
            HarnessVersion::new("1.0.0").expect("valid version"),
            ArtifactDigest::parse(digest('a')).expect("valid digest"),
        )
    }

    fn metric_bytes() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": ARTIFACT_SCHEMA_VERSION,
            "contract_id": "aics",
            "contract_version": 1,
            "numerator": 900,
            "denominator": 1000,
            "basis_points": 9000
        }))
        .expect("serializes")
    }

    fn accept_metric(bytes: &[u8]) -> Result<(), Error> {
        validate_metric_artifact(bytes).map(|_| ())
    }

    #[test]
    fn a_name_is_subject_short_digest_variant_and_extension() {
        let named = name("subject-1", &digest('a'));
        assert_eq!(named.render(), "subject-1-aaaaaaaa.json");
        let variant = ArtifactName::new(
            SubjectId::new("subject-1").expect("valid"),
            ArtifactDigest::parse(digest('b')).expect("valid"),
            Some("stats"),
            "json",
        )
        .expect("valid");
        assert_eq!(variant.render(), "subject-1-bbbbbbbb-stats.json");
    }

    #[test]
    fn a_name_rejects_path_smuggling() {
        for variant in ["..", "../escape", "a/b", "a\\b", "", "x\u{0}y"] {
            assert!(
                ArtifactName::new(
                    SubjectId::new("s").expect("valid"),
                    ArtifactDigest::parse(digest('a')).expect("valid"),
                    Some(variant),
                    "json"
                )
                .is_err(),
                "`{variant}` must be refused"
            );
        }
        assert!(
            ArtifactName::new(
                SubjectId::new("s").expect("valid"),
                ArtifactDigest::parse(digest('a')).expect("valid"),
                None,
                "j/son"
            )
            .is_err()
        );
    }

    #[test]
    fn an_untrusted_metric_artifact_is_validated() {
        let valid = validate_metric_artifact(&metric_bytes()).expect("valid");
        assert_eq!(valid.contract_id.as_str(), "aics");
        assert_eq!(valid.bps, BasisPoints::new(9_000).expect("valid"));
        let wrong = serde_json::json!({
            "schema_version": ARTIFACT_SCHEMA_VERSION,
            "contract_id": "aics",
            "contract_version": 1,
            "numerator": 1001,
            "denominator": 1000,
            "basis_points": 9000
        });
        assert!(
            validate_metric_artifact(&serde_json::to_vec(&wrong).expect("serializes")).is_err()
        );
        let future = serde_json::json!({
            "schema_version": ARTIFACT_SCHEMA_VERSION + 1,
            "contract_id": "aics",
            "contract_version": 1,
            "numerator": 1,
            "denominator": 2,
            "basis_points": 5000
        });
        assert!(
            validate_metric_artifact(&serde_json::to_vec(&future).expect("serializes")).is_err()
        );
    }

    #[tokio::test]
    async fn publish_is_no_clobber_and_reuse_requires_a_matching_identity() {
        let directory = tempfile::tempdir().expect("temp dir");
        let store = ArtifactStore::new(directory.path()).await.expect("opens");
        let named = name("subject-1", &digest('a'));
        assert_eq!(
            store
                .publish_once(&named, &metric_bytes())
                .await
                .expect("publishes"),
            ArtifactDecision::Published
        );
        assert_eq!(
            store
                .publish_once(&named, &metric_bytes())
                .await
                .expect("no clobber"),
            ArtifactDecision::Conflict
        );
        // Reuse through the full path: matching identity reuses, a changed identity is stale.
        assert_eq!(
            store
                .reuse_or_publish(&named, &identity(), &metric_bytes(), accept_metric)
                .await
                .expect("reuses"),
            ArtifactDecision::Stale {
                reason: "no provenance was recorded beside the artifact".to_owned()
            }
        );
        let empty_store = ArtifactStore::new(directory.path().join("second"))
            .await
            .expect("opens");
        assert_eq!(
            empty_store
                .reuse_or_publish(&named, &identity(), &metric_bytes(), accept_metric)
                .await
                .expect("publishes"),
            ArtifactDecision::Published
        );
        assert_eq!(
            empty_store
                .reuse_or_publish(&named, &identity(), &metric_bytes(), accept_metric)
                .await
                .expect("reuses"),
            ArtifactDecision::Reused
        );
        let mut changed = identity();
        changed.input_digest = ArtifactDigest::parse(digest('c')).expect("valid");
        assert!(matches!(
            empty_store
                .reuse_or_publish(&named, &changed, &metric_bytes(), accept_metric)
                .await
                .expect("stale"),
            ArtifactDecision::Stale { .. }
        ));
    }

    #[tokio::test]
    async fn a_read_is_bounded_and_an_absent_artifact_is_none() {
        let directory = tempfile::tempdir().expect("temp dir");
        let store = ArtifactStore::new(directory.path()).await.expect("opens");
        let named = name("subject-1", &digest('a'));
        assert_eq!(store.read_bounded(&named, 10).await.expect("absent"), None);
        store
            .publish_once(&named, b"0123456789")
            .await
            .expect("publishes");
        assert_eq!(
            store.read_bounded(&named, 10).await.expect("exact bound"),
            Some(b"0123456789".to_vec())
        );
        assert!(matches!(
            store.read_bounded(&named, 9).await,
            Err(Error::InvalidTarget { .. })
        ));
    }

    #[tokio::test]
    async fn an_invalid_artifact_is_never_published() {
        let directory = tempfile::tempdir().expect("temp dir");
        let store = ArtifactStore::new(directory.path()).await.expect("opens");
        let named = name("subject-1", &digest('a'));
        assert!(
            store
                .reuse_or_publish(&named, &identity(), b"not json", accept_metric)
                .await
                .is_err()
        );
        assert_eq!(
            store.read_bounded(&named, 64).await.expect("absent"),
            None,
            "a rejected artifact writes nothing"
        );
    }

    #[tokio::test]
    async fn a_stale_artifact_is_labelled_not_silently_reused() {
        let directory = tempfile::tempdir().expect("temp dir");
        let store = ArtifactStore::new(directory.path()).await.expect("opens");
        let named = name("subject-1", &digest('a'));
        store
            .publish_once(&named, &metric_bytes())
            .await
            .expect("publishes");
        // Corrupt the identity to a different profile: reuse must refuse rather than overwrite.
        let other = identity();
        store
            .write_identity(
                &named,
                &ArtifactIdentity {
                    profile_id: ProfileId::new("other-v1").expect("valid"),
                    ..other.clone()
                },
            )
            .await
            .expect("identity written");
        let decision = store
            .reuse_or_publish(&named, &other, &metric_bytes(), accept_metric)
            .await
            .expect("decides");
        assert!(matches!(decision, ArtifactDecision::Stale { .. }));
    }

    #[test]
    fn the_identity_serializes_and_round_trips() {
        let identity = identity();
        let encoded = serde_json::to_string(&identity).expect("serializes");
        let decoded: ArtifactIdentity = serde_json::from_str(&encoded).expect("deserializes");
        assert_eq!(decoded, identity);
        assert_eq!(identity.crate_version, crate::CRATE_VERSION);
    }
}
