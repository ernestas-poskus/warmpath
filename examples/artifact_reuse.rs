//! Publish and reuse a metric artifact through the async store.
//!
//! Run with `cargo run --example artifact_reuse`.

use warmpath::artifact::{ArtifactDecision, ArtifactName, ArtifactStore, validate_metric_artifact};
use warmpath::newtypes::{
    ArtifactDigest, ContractId, ContractVersion, HarnessVersion, ProfileId, ProfileVersion,
    SubjectId,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::temp_dir().join("warmpath-example-artifacts");
    let store = ArtifactStore::new(&directory).await?;
    let name = ArtifactName::new(
        SubjectId::new("task-1")?,
        ArtifactDigest::parse("a".repeat(64))?,
        Some("aics"),
        "json",
    )?;
    let bytes = serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "contract_id": "aics",
        "contract_version": 1,
        "numerator": 900,
        "denominator": 1000,
        "basis_points": 9000,
    }))?;
    let identity = warmpath::artifact::ArtifactIdentity::new(
        ContractId::new("aics")?,
        ContractVersion::new(1),
        ProfileId::new("codex-v1")?,
        ProfileVersion::new(1),
        HarnessVersion::new("1.0.0")?,
        ArtifactDigest::parse("b".repeat(64))?,
    );
    let decision = store
        .reuse_or_publish(&name, &identity, &bytes, |bytes| {
            validate_metric_artifact(bytes).map(|_| ())
        })
        .await?;
    println!("{decision:?}");
    let _ = ArtifactDecision::Published;
    Ok(())
}
