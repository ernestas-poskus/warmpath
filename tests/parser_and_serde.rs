//! Parser robustness (arbitrary bytes never panic) and serde round trips for public values.

use warmpath::newtypes::{
    ArtifactDigest, BasisPoints, CachedTokens, EpochSeconds, FreshTokens, HarnessId,
    HarnessVersion, ModelId, PrefixDigest, ProviderId, RevisionId, SubjectId, Tokens, TtlSecs,
};
use warmpath::profile::{CacheClass, HarnessProfile};
use warmpath::usage::parse;
use warmpath::warm::WarmRecord;

#[test]
fn parsing_arbitrary_bytes_never_panics_and_never_invents_usage() {
    let profiles = [
        HarnessProfile::codex(),
        HarnessProfile::claude(),
        HarnessProfile::agy(),
        HarnessProfile::dsh(),
        HarnessProfile::grok(),
    ];
    let mut bytes = Vec::with_capacity(256);
    for byte in 0..=u8::MAX {
        bytes.push(byte);
        for profile in &profiles {
            let outcome = parse(profile, &bytes);
            assert!(
                outcome.usage.fresh().is_reported()
                    || outcome.usage.fresh().as_reported().is_none()
            );
        }
        bytes.clear();
    }
    for profile in &profiles {
        let outcome = parse(profile, b"\xff\xfe{\"usage\":");
        assert!(!outcome.warnings.is_empty());
        assert_eq!(outcome.totals, None);
    }
}

#[test]
fn a_builtin_profile_round_trips_through_its_data() {
    let profile = HarnessProfile::claude();
    assert_eq!(profile.cache_class, CacheClass::Marker);
    assert!(profile.supports_version(&HarnessVersion::new("2.0.0").expect("valid")));
    assert!(
        !HarnessProfile::codex().supports_version(&HarnessVersion::new("0.0.1").expect("valid"))
    );
}

#[test]
fn public_values_round_trip_through_serde() {
    let record = WarmRecord::new(
        SubjectId::new("subject-1").expect("valid"),
        RevisionId::new("rev-2").expect("valid"),
        "workspace",
        HarnessId::new("codex").expect("valid"),
        ProviderId::new("openai").expect("valid"),
        PrefixDigest::parse("a".repeat(64)).expect("valid"),
        EpochSeconds::new(1_700_000_000),
    )
    .with_model(ModelId::new("gpt-5.6").expect("valid"))
    .with_session("session-12345678")
    .with_usage(CachedTokens::new(900), FreshTokens::new(100))
    .with_files(vec!["src/lib.rs".to_owned()]);
    let encoded = serde_json::to_string(&record).expect("serializes");
    let decoded: WarmRecord = serde_json::from_str(&encoded).expect("deserializes");
    assert_eq!(decoded, record);

    let basis_points = BasisPoints::new(9_970).expect("valid");
    let json = serde_json::to_string(&basis_points).expect("serializes");
    assert_eq!(json, "9970");
    assert_eq!(
        serde_json::from_str::<BasisPoints>(&json).expect("deserializes"),
        basis_points
    );
    let tokens = Tokens::new(1_234);
    let json = serde_json::to_string(&tokens).expect("serializes");
    assert_eq!(json, "1234");
    assert_eq!(
        serde_json::from_str::<Tokens>(&json).expect("deserializes"),
        tokens
    );
    let digest = ArtifactDigest::parse("b".repeat(64)).expect("valid");
    let json = serde_json::to_string(&digest).expect("serializes");
    assert_eq!(json, format!("\"{}\"", "b".repeat(64)));
    assert_eq!(
        serde_json::from_str::<ArtifactDigest>(&json).expect("deserializes"),
        digest
    );
    assert_eq!(TtlSecs::new(300).expect("valid").get(), 300);
}
