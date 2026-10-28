//! The PromptTokens matrix: three shapes × absent / measured zero / present / inconsistent.

use warmpath::newtypes::{BasisPoints, CacheWriteTokens, CachedTokens, FreshTokens, Tokens};
use warmpath::profile::InputShape;
use warmpath::prompt::PromptTokens;
use warmpath::reported::Reported;

fn prompt(
    shape: InputShape,
    input: Reported<Tokens>,
    read: Reported<CachedTokens>,
    write: Reported<CacheWriteTokens>,
) -> PromptTokens {
    PromptTokens::new(input, read, write, shape)
}

fn reported(value: u64) -> Reported<Tokens> {
    Reported::Reported(Tokens::new(value))
}

fn read(value: u64) -> Reported<CachedTokens> {
    Reported::Reported(CachedTokens::new(value))
}

fn write(value: u64) -> Reported<CacheWriteTokens> {
    Reported::Reported(CacheWriteTokens::new(value))
}

#[test]
fn every_shape_treats_absence_as_unknown() {
    for shape in [
        InputShape::Inclusive,
        InputShape::Exclusive,
        InputShape::Partitioned,
    ] {
        let tokens = prompt(
            shape,
            Reported::NotReported,
            Reported::NotReported,
            Reported::NotReported,
        );
        assert_eq!(tokens.fresh(), Reported::NotReported, "{shape:?}");
        assert_eq!(tokens.expected(), Reported::NotReported, "{shape:?}");
        assert_eq!(tokens.cache_hit_bps(), None, "{shape:?}");
    }
}

#[test]
fn every_shape_keeps_a_measured_zero_as_a_measurement() {
    for shape in [
        InputShape::Inclusive,
        InputShape::Exclusive,
        InputShape::Partitioned,
    ] {
        let tokens = prompt(shape, reported(0), read(0), write(0));
        assert_eq!(
            tokens.fresh(),
            Reported::Reported(FreshTokens::new(0)),
            "{shape:?}"
        );
        assert_eq!(
            tokens.expected(),
            Reported::Reported(Tokens::new(0)),
            "{shape:?}"
        );
        assert_eq!(
            tokens.cache_hit_bps(),
            None,
            "{shape:?}: a share of nothing is undefined"
        );
    }
}

#[test]
fn the_three_shapes_normalize_a_present_report_differently() {
    let inclusive = prompt(InputShape::Inclusive, reported(1_200), read(900), write(0));
    assert_eq!(inclusive.fresh(), Reported::Reported(FreshTokens::new(300)));
    assert_eq!(inclusive.expected(), Reported::Reported(Tokens::new(1_200)));
    assert_eq!(inclusive.cache_hit_bps(), BasisPoints::new(7_500));

    let exclusive = prompt(
        InputShape::Exclusive,
        reported(1_000),
        read(3_000),
        write(400),
    );
    assert_eq!(
        exclusive.fresh(),
        Reported::Reported(FreshTokens::new(1_000))
    );
    assert_eq!(exclusive.expected(), Reported::Reported(Tokens::new(4_000)));
    assert_eq!(exclusive.cache_hit_bps(), BasisPoints::new(7_500));

    let partitioned = prompt(
        InputShape::Partitioned,
        reported(1_000),
        read(4_000),
        write(500),
    );
    assert_eq!(
        partitioned.fresh(),
        Reported::Reported(FreshTokens::new(1_000))
    );
    assert_eq!(
        partitioned.expected(),
        Reported::Reported(Tokens::new(5_500))
    );
    assert_eq!(partitioned.cache_hit_bps(), BasisPoints::new(7_273));
}

#[test]
fn an_inconsistent_inclusive_report_saturates_rather_than_going_negative() {
    let tokens = prompt(
        InputShape::Inclusive,
        reported(1_000),
        read(1_500),
        write(0),
    );
    assert_eq!(
        tokens.fresh(),
        Reported::Reported(FreshTokens::new(0)),
        "a read larger than input cannot invent a negative count"
    );
    assert_eq!(
        tokens.cache_hit_bps(),
        BasisPoints::new(10_000),
        "the ratio clamps at 100% rather than exceeding it"
    );
}

#[test]
fn a_missing_read_half_makes_an_exclusive_denominator_unknown() {
    let tokens = prompt(
        InputShape::Exclusive,
        reported(1_000),
        Reported::NotReported,
        Reported::NotReported,
    );
    assert_eq!(tokens.fresh(), Reported::Reported(FreshTokens::new(1_000)));
    assert_eq!(tokens.expected(), Reported::NotReported);
    assert_eq!(tokens.cache_hit_bps(), None);
}

#[test]
fn a_missing_write_partition_makes_a_partitioned_denominator_unknown() {
    let tokens = prompt(
        InputShape::Partitioned,
        reported(1_000),
        read(4_000),
        Reported::NotReported,
    );
    assert_eq!(tokens.expected(), Reported::NotReported);
    assert_eq!(tokens.cache_hit_bps(), None);
}

#[test]
fn an_inclusive_report_whose_read_is_unknown_has_no_fresh_number() {
    let tokens = prompt(
        InputShape::Inclusive,
        reported(1_200),
        Reported::NotReported,
        Reported::NotReported,
    );
    assert_eq!(tokens.fresh(), Reported::NotReported);
    assert_eq!(
        tokens.expected(),
        Reported::Reported(Tokens::new(1_200)),
        "the denominator is known even though the fresh share is not"
    );
    assert_eq!(tokens.cache_hit_bps(), None);
}
