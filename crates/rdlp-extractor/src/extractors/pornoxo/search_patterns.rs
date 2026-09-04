//! Filter vocabulary for PornoXO search and tag listings.

use rdlp_core::Result;
use rdlp_types::{SearchFilter, SearchFilterDescriptor, SearchFilterValue};

use crate::base::common::{format_std_filter_error, validate_against_descriptors};

/// The filters PornoXO accepts, and the only values it accepts for each.
///
/// `route` is PornoXO's own, not the site's: the site exposes two listing
/// paths, `/search/?q=` (Cloudflare-gated, full-text) and `/tags/<slug>/`
/// (open, slug-matched). Making the choice an explicit filter keeps a missing
/// `cf_clearance` from silently degrading a search into a tag listing, which
/// would return different videos than the operator asked for.
///
/// The other three are the site's own query parameters, read off its listing
/// controls.
pub(crate) fn supported_filters() -> Vec<SearchFilterDescriptor> {
    vec![
        SearchFilterDescriptor::new(
            "route",
            "Listing Route",
            SearchFilterValue::list([
                ("search", "Full-text search"),
                ("tag", "Tag listing (query is the tag slug)"),
            ]),
            Some("search"),
        ),
        SearchFilterDescriptor::new(
            "sort",
            "Sort By",
            SearchFilterValue::list([
                ("re", "Trending"),
                ("mr", "New"),
                ("mw", "Most Popular"),
                ("tr", "Top Rated"),
                ("lg", "Longest"),
            ]),
            Some("re"),
        ),
        SearchFilterDescriptor::new(
            "quality",
            "Quality",
            SearchFilterValue::list([("hd", "HD"), ("vr", "VR")]),
            None,
        ),
        SearchFilterDescriptor::new(
            "filter_length",
            "Duration",
            SearchFilterValue::list([
                ("all", "Any length"),
                ("short", "Short"),
                ("normal", "Normal"),
                ("long", "Long"),
            ]),
            Some("all"),
        ),
    ]
}

/// Reject any filter key or value PornoXO does not understand.
///
/// Every key uses the default `AllowedValues` policy, with no `FreeText`
/// escape: the site accepts a bad value SILENTLY — a live `?sort=zz` returns
/// the default results byte-identically — so there is no server-side check to
/// defer to and a typo would otherwise be indistinguishable from a deliberate
/// unsorted query.
pub(crate) fn validate(filters: &[SearchFilter]) -> Result<()> {
    validate_against_descriptors(filters, &supported_filters(), &[])
        .map_err(|e| format_std_filter_error("PornoXO", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_types::SearchFilter;

    fn f(key: &str, value: &str) -> SearchFilter {
        SearchFilter {
            key: key.to_owned(),
            value: value.to_owned(),
        }
    }

    #[test]
    fn accepts_every_documented_sort_code() {
        for code in ["re", "mr", "mw", "tr", "lg"] {
            assert!(
                validate(&[f("sort", code)]).is_ok(),
                "sort={code} must be accepted"
            );
        }
    }

    /// The site SILENTLY ignores an unknown sort and returns the default
    /// results byte-identically (verified live: `?sort=zz` == no `sort`).
    /// There is no server-side rejection to defer to, so this check is the
    /// only thing that can tell an operator they mistyped.
    #[test]
    fn rejects_an_unknown_sort_code() {
        let e = validate(&[f("sort", "newest")]).unwrap_err();
        let msg = e.to_string();
        assert!(
            msg.contains("newest"),
            "must name the rejected value: {msg}"
        );
        assert!(msg.contains("sort"), "must name the key: {msg}");
        assert!(
            msg.contains("re") && msg.contains("lg"),
            "must list what IS allowed: {msg}"
        );
    }

    #[test]
    fn rejects_an_unknown_filter_key() {
        // `ordering` is the key four sibling extractors use for this concept,
        // so it is the typo an operator is most likely to arrive with.
        let e = validate(&[f("ordering", "mr")]).unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains("ordering"), "must name the bad key: {msg}");
        assert!(msg.contains("sort"), "must suggest the real keys: {msg}");
    }

    #[test]
    fn accepts_both_routes_and_rejects_a_third() {
        assert!(validate(&[f("route", "search")]).is_ok());
        assert!(validate(&[f("route", "tag")]).is_ok());
        assert!(validate(&[f("route", "tags")]).is_err());
    }

    #[test]
    fn accepts_quality_and_length_vocabularies() {
        assert!(validate(&[f("quality", "hd")]).is_ok());
        assert!(validate(&[f("quality", "vr")]).is_ok());
        assert!(validate(&[f("quality", "4k")]).is_err());
        for v in ["all", "short", "normal", "long"] {
            assert!(validate(&[f("filter_length", v)]).is_ok(), "length={v}");
        }
        assert!(validate(&[f("filter_length", "medium")]).is_err());
    }

    #[test]
    fn accepts_an_empty_filter_set_and_a_valid_combination() {
        assert!(validate(&[]).is_ok());
        assert!(
            validate(&[f("route", "tag"), f("sort", "lg"), f("quality", "hd")]).is_ok(),
            "filters must be independent, not mutually exclusive"
        );
    }

    /// `supported_filters` is what the CLI advertises and what `validate`
    /// checks against; a key present in one and absent from the other is
    /// either an undocumented filter or an unenforced one.
    #[test]
    fn descriptors_cover_every_validated_key() {
        let keys: Vec<_> = supported_filters().into_iter().map(|d| d.key).collect();
        for k in ["route", "sort", "quality", "filter_length"] {
            assert!(keys.iter().any(|d| d == k), "descriptor missing for {k}");
        }
        assert_eq!(keys.len(), 4, "no undocumented filter keys");
    }

    /// Every descriptor's declared default must itself be an accepted value —
    /// a default outside its own vocabulary would be rejected the moment a
    /// frontend echoed it back.
    #[test]
    fn every_declared_default_is_itself_valid() {
        for d in supported_filters() {
            if let Some(default) = d.default {
                assert!(
                    validate(&[f(&d.key, &default)]).is_ok(),
                    "default {default} for {} is not an allowed value",
                    d.key
                );
            }
        }
    }
}
