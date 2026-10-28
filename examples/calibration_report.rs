//! Summarize warm-resume observations into an age-band calibration report.
//!
//! A TTL is a step, not a decay curve, and the report refuses to state a realized ratio for a
//! harness with fewer than [`MIN_WARM_SAMPLES`] warm observations. Under-sampled harnesses are
//! reported as insufficient rather than averaged into a number nobody should trust.
//!
//! Run with `cargo run --example calibration_report`.

use warmpath::calibration::{MIN_WARM_SAMPLES, WarmResumeObservation, summarize, warm_probability};
use warmpath::newtypes::{CachedTokens, FreshTokens, HarnessId, Tokens, TtlSecs, WarmAgeSecs};

fn observation(
    harness: &HarnessId,
    age: u64,
    expected: u64,
    realized: u64,
) -> WarmResumeObservation {
    WarmResumeObservation {
        harness: harness.clone(),
        age: WarmAgeSecs::new(age),
        cached: CachedTokens::new(realized),
        fresh: FreshTokens::new(1_000),
        expected_saved: Tokens::new(expected),
        resumed: true,
        realized_cached: warmpath::reported::Reported::Reported(CachedTokens::new(realized)),
    }
}

fn main() {
    let codex = HarnessId::new("codex").expect("valid harness");
    let claude = HarnessId::new("claude").expect("valid harness");

    let observations = vec![
        // Codex: two observations under five minutes, one in the ten-to-thirty band.
        observation(&codex, 120, 1_000, 900),
        observation(&codex, 240, 1_000, 850),
        observation(&codex, 700, 1_000, 400),
        // Claude: a single warm observation, below the reporting floor.
        observation(&claude, 90, 2_000, 1_900),
    ];

    let report = summarize(&observations);
    print!("{}", report.render_markdown());

    let ttl = TtlSecs::new(600).expect("a positive TTL");
    println!(
        "warm at 599 s with a 600 s TTL: {}",
        warm_probability(WarmAgeSecs::new(599), Some(ttl))
    );
    println!(
        "warm at 600 s with a 600 s TTL: {}",
        warm_probability(WarmAgeSecs::new(600), Some(ttl))
    );

    let insufficient = report.insufficient();
    assert_eq!(insufficient.len(), 1, "only claude is under-sampled");
    assert_eq!(insufficient[0].as_str(), "claude");
    println!("minimum warm samples per harness: {MIN_WARM_SAMPLES}");
}
