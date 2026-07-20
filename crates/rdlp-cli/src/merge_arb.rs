//! Proptest strategies for the differential merge test (PR 1 scaffolding).
//! DELETED in Task 7 together with the differential test.
#![cfg(test)]

use crate::args::Args;
use proptest::prelude::*;
use rdlp_api::{Config, ContainerFormat, FixupPolicy};

// rdlp-cli does not depend on strum; hand-list the variants the merge touches.
const CONTAINERS: &[ContainerFormat] = &[
    ContainerFormat::Mp4,
    ContainerFormat::Mkv,
    ContainerFormat::WebM,
];
const FIXUPS: &[FixupPolicy] = &[
    FixupPolicy::Never,
    FixupPolicy::Warn,
    FixupPolicy::DetectOrWarn,
];

/// Valid `--fixup` spellings only. Arbitrary strings would test the parser,
/// not the merge.
fn arb_fixup_arg() -> impl Strategy<Value = Option<String>> {
    prop::option::of(prop::sample::select(vec![
        "never".to_owned(),
        "warn".to_owned(),
        "detect_or_warn".to_owned(),
        "detect".to_owned(),
    ]))
}

fn arb_container_arg() -> impl Strategy<Value = Option<String>> {
    prop::option::of(prop::sample::select(CONTAINERS.to_vec()).prop_map(|c| c.to_string()))
}

/// A `Config` with the fields this PR's merge actually touches varied, and
/// everything else at its default. Widening this is a follow-up, not PR 1.
pub fn arb_config() -> impl Strategy<Value = Config> {
    (
        any::<bool>(),
        any::<bool>(),
        prop::sample::select(FIXUPS.to_vec()),
        prop::option::of(prop::sample::select(CONTAINERS.to_vec())),
        prop::collection::vec("[a-z]{1,6}", 0..3),
    )
        .prop_map(|(quiet, verbose, fixup, remux, filters)| Config {
            quiet,
            verbose,
            match_filters: filters,
            postprocess: rdlp_api::PostProcess {
                fixup,
                remux_container: remux,
                ..Default::default()
            },
            ..Default::default()
        })
}

pub fn arb_args() -> impl Strategy<Value = Args> {
    (
        any::<bool>(),
        any::<bool>(),
        arb_fixup_arg(),
        arb_container_arg(),
        prop::collection::vec("[a-z]{1,6}", 0..3),
    )
        .prop_map(|(quiet, verbose, fixup, remux, match_filter)| {
            let mut a = crate::config::tests::default_args();
            a.quiet = quiet;
            a.verbose = verbose;
            a.fixup = fixup;
            a.remux = remux;
            a.match_filter = match_filter;
            a
        })
}

proptest! {
    #[test]
    fn strategies_generate(cfg in arb_config(), args in arb_args()) {
        prop_assert!(cfg.match_filters.len() < 3);
        prop_assert!(args.match_filter.len() < 3);
    }
}

use crate::config::config_legacy::merge_config_legacy;
use crate::config::{ResolvedInteractiveValues, merge_config};

fn no_interactive() -> ResolvedInteractiveValues {
    ResolvedInteractiveValues {
        audio_format: None,
        recode_video: None,
        remux_container: None,
    }
}

proptest! {
    /// The rewrite must be behaviour-preserving. Any divergence surfaces here
    /// as a concrete counterexample naming the field.
    #[test]
    fn rewrite_matches_legacy(cfg in arb_config(), args in arb_args()) {
        let new = merge_config(&args, cfg.clone(), no_interactive());
        let old = merge_config_legacy(&args, cfg, no_interactive());
        match (new, old) {
            (Ok(n), Ok(o)) => prop_assert_eq!(n, o),
            (Err(_), Err(_)) => {}
            (n, o) => prop_assert!(false, "one errored, one did not: {n:?} vs {o:?}"),
        }
    }
}
