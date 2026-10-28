//! Public-surface integration tests.
//!
//! The unit tests inside `src/` pin each module's internal rules. These tests use the crate the way
//! a consumer does — through the published modules and the root re-exports — and cover the parts
//! the unit tests only reach indirectly: the custom-profile builder, a warm request built from a
//! record, batch ordering, and an end-to-end parse → gate → calibration pipeline.
//!
//! Everything is offline and constructed in the test.

use warmpath::calibration::{MIN_WARM_SAMPLES, WarmResumeObservation, summarize};
use warmpath::gate::{CacheSample, Verdict, build_report, parse_target_percent, percent_text};
use warmpath::newtypes::{
    BasisPoints, CachedTokens, EpochSeconds, FreshTokens, HarnessId, ModelId, PrefixDigest,
    ProviderId, RevisionId, SubjectId, Tokens, TtlSecs, WarmAgeSecs,
};
use warmpath::profile::{CacheClass, HarnessProfile, InputShape, OutputShape, UsagePaths};
use warmpath::reported::Reported;
use warmpath::schedule::{ScheduleKey, WarmMark, schedule_order};
use warmpath::usage::parse;
use warmpath::warm::{WarmRecord, WarmRequest, decide};

#[test]
fn the_crate_root_reexports_its_main_types() {
    let token = warmpath::Tokens::new(1_200);
    let prompt = warmpath::PromptTokens::new(
        warmpath::Reported::Reported(token),
        warmpath::Reported::NotReported,
        warmpath::Reported::NotReported,
        warmpath::InputShape::Inclusive,
    );
    assert_eq!(prompt.expected(), warmpath::Reported::Reported(token));
    assert_eq!(
        warmpath::HarnessProfile::codex().input_shape,
        warmpath::InputShape::Inclusive
    );
    let error = warmpath::Error::from(std::io::Error::other("boom"));
    assert!(error.to_string().starts_with("io:"));
    assert!(!warmpath::CRATE_VERSION.is_empty());
}

#[test]
fn a_custom_profile_is_declared_rather_than_guessed() {
    let profile = HarnessProfile::custom("custom-v1", "my-harness", "my-provider")
        .expect("valid identifiers")
        .cache_class(CacheClass::Strict)
        .output_shape(OutputShape::Jsonl)
        .input_shape(InputShape::Exclusive)
        .usage_paths(UsagePaths::new(
            &["usage.input_tokens"],
            &["usage.output_tokens"],
            &["usage.cached_input_tokens"],
            &["usage.cache_creation_input_tokens"],
            &["usage.reasoning_tokens"],
        ))
        .session_paths(&["session_id"])
        .resume_argv(&["--resume", "{session}"])
        .warm_ttl(TtlSecs::new(300).expect("a positive TTL"))
        .build();

    assert_eq!(profile.harness.as_str(), "my-harness");
    assert_eq!(profile.provider.as_str(), "my-provider");
    assert_eq!(profile.cache_class, CacheClass::Strict);
    assert_eq!(profile.input_shape, InputShape::Exclusive);
    assert!(profile.warm_ttl.is_some());

    let output = br#"{"usage":{"input_tokens":1000,"cached_input_tokens":750,"output_tokens":20}}"#;
    let outcome = parse(&profile, output);
    assert_eq!(
        outcome
            .usage
            .prompt
            .fresh()
            .as_reported()
            .map(|tokens| tokens.get()),
        Some(1_000),
        "an exclusive input count is already fresh"
    );
    assert_eq!(
        outcome.usage.prompt.cache_hit_bps(),
        BasisPoints::new(4_286),
        "750 / (1000 + 750), rounded half up"
    );
}

#[test]
fn a_warm_request_can_be_built_from_a_record_and_decides() {
    let provider = ProviderId::new("openai").expect("valid provider");
    let prefix = PrefixDigest::parse("a".repeat(64)).expect("valid digest");
    let record = WarmRecord::new(
        SubjectId::new("task-1").expect("valid subject"),
        RevisionId::new("rev-1").expect("valid revision"),
        "/workspace/project",
        HarnessId::new("codex").expect("valid harness"),
        provider.clone(),
        prefix.clone(),
        EpochSeconds::new(1_000),
    )
    .with_model(ModelId::new("gpt-5.6").expect("valid model"))
    .with_session("session-12345678")
    .with_usage(CachedTokens::new(20_000), FreshTokens::new(1_000))
    .with_files(vec!["src/lib.rs".to_owned(), "src/main.rs".to_owned()]);

    let request = WarmRequest::for_record(
        &record,
        prefix,
        vec!["src/lib.rs".to_owned(), "src/other.rs".to_owned()],
    );
    assert_eq!(request.subject, record.subject);
    assert_eq!(request.revision, record.revision);
    assert_eq!(request.workspace, record.workspace);
    assert_eq!(request.harness, record.harness);
    assert_eq!(request.provider, record.provider);
    assert_eq!(request.model, record.model);
    assert_eq!(request.effort, record.effort);

    let contract = warmpath::cache::semantics(&provider, record.model.as_ref());
    let decision = decide(
        std::slice::from_ref(&record),
        &request,
        EpochSeconds::new(1_100),
        Some(TtlSecs::new(600).expect("a positive TTL")),
        BasisPoints::ZERO,
        &contract,
    );
    let route = decision.chosen().expect("the record is warm and matches");
    assert_eq!(route.record, record);
    assert_eq!(
        route.overlap,
        BasisPoints::new(3_333).expect("valid basis points"),
        "one shared file out of a three-file union"
    );
}

#[test]
fn a_batch_order_puts_warm_keys_first_and_is_a_permutation() {
    let codex = HarnessId::new("codex").expect("valid harness");
    let runs = vec![
        ScheduleKey::new(codex.clone(), "repo-a"),
        ScheduleKey::new(codex.clone(), "repo-b"),
        ScheduleKey::new(codex.clone(), "repo-a"),
    ];
    let warm = vec![WarmMark::new(codex, "repo-a", EpochSeconds::new(9_900))];
    let order = schedule_order(
        &runs,
        &warm,
        EpochSeconds::new(10_000),
        TtlSecs::new(600).expect("a positive TTL"),
    );
    assert_eq!(order, vec![0, 2, 1]);
}

#[test]
fn parse_gate_and_calibration_compose_through_the_public_api() {
    let profile = HarnessProfile::codex();
    let output = br#"{"type":"turn.completed","usage":{"input_tokens":12000,"cached_input_tokens":9000,"output_tokens":400}}"#;
    let outcome = parse(&profile, output);
    assert!(outcome.warnings.is_empty());
    let fresh = outcome
        .usage
        .prompt
        .fresh()
        .as_reported()
        .copied()
        .expect("fresh input is reported");
    let cached = outcome
        .usage
        .prompt
        .cache_read
        .as_reported()
        .copied()
        .expect("cache reads are reported");
    assert_eq!(fresh.get(), 3_000);

    let target = parse_target_percent("75").expect("a valid target");
    let report = build_report(&[CacheSample::measured(cached, fresh)], target);
    assert_eq!(report.bps, BasisPoints::new(7_500));
    assert_eq!(report.verdict(), Verdict::Pass);
    assert_eq!(percent_text(target), "75.00%");

    let observation = WarmResumeObservation {
        harness: HarnessId::new("codex").expect("valid harness"),
        age: WarmAgeSecs::new(120),
        cached,
        fresh,
        expected_saved: Tokens::new(2_000),
        resumed: true,
        realized_cached: Reported::Reported(cached),
    };
    let second = WarmResumeObservation {
        age: WarmAgeSecs::new(240),
        ..observation.clone()
    };
    let calibration = summarize(&[observation, second]);
    let codex = &calibration.harnesses[0];
    assert!(!codex.insufficient);
    assert_eq!(codex.warm_samples, MIN_WARM_SAMPLES);
    assert_eq!(codex.realized_ratio, Some(BasisPoints::MAX));
    assert!(calibration.render_markdown().contains("codex"));
}

#[test]
fn a_mixed_savings_report_sums_stages_without_inventing_a_before() {
    use warmpath::savings::{ReductionStage, SavingsBasis, SavingsObservation, SavingsReport};

    let report = SavingsReport::from_observations(&[
        SavingsObservation::new(
            ReductionStage::Ingest,
            SavingsBasis::Measured {
                before: Tokens::new(1_000),
                after: Tokens::new(700),
            },
        ),
        SavingsObservation::new(
            ReductionStage::Request,
            SavingsBasis::Reported {
                saved: Tokens::new(100),
            },
        ),
    ])
    .expect("distinct and consistent observations");

    assert_eq!(report.total_saved(), Some(Tokens::new(400)));
    assert_eq!(
        report.saved_at(ReductionStage::Ingest),
        Some(Tokens::new(300))
    );
    assert_eq!(
        report.saved_at(ReductionStage::Request),
        Some(Tokens::new(100))
    );
    assert_eq!(
        report.total_before(),
        None,
        "a reported stage never invents a before"
    );
    assert!(report.is_complete());
}
