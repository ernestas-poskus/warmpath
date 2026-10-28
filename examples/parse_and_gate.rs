//! Parse a harness output and gate the run on the AICS contract.
//!
//! Run with `cargo run --example parse_and_gate`.

use warmpath::gate::CacheSample;
use warmpath::gate::{Verdict, build_report, parse_target_percent, percent_text};
use warmpath::profile::HarnessProfile;
use warmpath::reported::Reported;
use warmpath::usage::parse;

fn main() {
    let profile = HarnessProfile::codex();
    let output = br#"{"type":"turn.completed","usage":{"input_tokens":12000,"cached_input_tokens":9000,"output_tokens":400}}"#;
    let outcome = parse(&profile, output);
    for warning in &outcome.warnings {
        eprintln!("warning: {warning}");
    }
    let Some(fresh) = outcome.usage.prompt.fresh().as_reported().copied() else {
        eprintln!("the run did not report enough to measure a cache share");
        return;
    };
    let cached = outcome
        .usage
        .prompt
        .cache_read
        .as_reported()
        .copied()
        .unwrap_or_default();
    let sample = CacheSample {
        cached: Reported::Reported(cached),
        fresh: Reported::Reported(fresh),
        write: Reported::NotReported,
    };
    let target = parse_target_percent("90").expect("a valid target");
    let report = build_report(&[sample], target);
    println!("{}", report.render());
    println!("verdict: {}", report.verdict());
    if report.verdict() == Verdict::Fail {
        std::process::exit(1);
    }
    assert!(percent_text(target) == "90.00%");
}
