//! Decide whether a captured session should be resumed.
//!
//! Run with `cargo run --example warm_session`.

use warmpath::cache::semantics;
use warmpath::newtypes::{
    BasisPoints, CachedTokens, EpochSeconds, FreshTokens, HarnessId, ModelId, ProviderId,
    RevisionId, SubjectId, TtlSecs,
};
use warmpath::prefix::digest;
use warmpath::profile::HarnessProfile;
use warmpath::warm::{WarmRecord, WarmRequest, decide};

fn main() {
    let profile = HarnessProfile::claude();
    let provider = ProviderId::new("claude").expect("valid provider");
    let harness = HarnessId::new("claude").expect("valid harness");
    let prefix = digest(&[b"static instructions", b"tools"]);
    let record = WarmRecord::new(
        SubjectId::new("task-1").expect("valid subject"),
        RevisionId::new("rev-1").expect("valid revision"),
        "/workspace/project",
        harness.clone(),
        provider.clone(),
        prefix.clone(),
        EpochSeconds::new(1_000),
    )
    .with_model(ModelId::new("claude-sonnet-4").expect("valid model"))
    .with_session("session-12345678")
    .with_usage(CachedTokens::new(20_000), FreshTokens::new(1_000))
    .with_files(vec!["src/lib.rs".to_owned(), "src/main.rs".to_owned()]);
    let request = WarmRequest {
        subject: record.subject.clone(),
        revision: record.revision.clone(),
        workspace: record.workspace.clone(),
        harness,
        provider: provider.clone(),
        model: record.model.clone(),
        effort: None,
        prefix,
        files: vec!["src/lib.rs".to_owned(), "src/other.rs".to_owned()],
    };
    let contract = semantics(&provider, record.model.as_ref());
    let decision = decide(
        &[record],
        &request,
        EpochSeconds::new(1_120),
        profile.warm_ttl,
        BasisPoints::new(3_000).expect("valid floor"),
        &contract,
    );
    match decision.chosen() {
        Some(route) => println!("resume: {route}"),
        None => {
            for rejection in &decision.rejected {
                println!(
                    "rejected ({}): {}",
                    rejection.reason.code(),
                    rejection.record.session_key.as_deref().unwrap_or("-")
                );
            }
        }
    }
    let _ = TtlSecs::new(300);
}
