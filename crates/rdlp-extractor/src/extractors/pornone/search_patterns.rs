//! Search filters for PornOne — the site's search form takes only `q`, so
//! there are none; any supplied filter is rejected by name.

use crate::base::common::{format_std_filter_error, validate_against_descriptors};
use rdlp_core::Result;
use rdlp_types::{SearchFilter, SearchFilterDescriptor};

pub(crate) fn supported_filters() -> Vec<SearchFilterDescriptor> {
    Vec::new()
}

pub(crate) fn validate(filters: &[SearchFilter]) -> Result<()> {
    validate_against_descriptors(filters, &supported_filters(), &[])
        .map_err(|e| format_std_filter_error(super::NAME, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_filters_is_ok() {
        assert!(validate(&[]).is_ok());
    }

    #[test]
    fn any_supplied_filter_is_rejected_by_name() {
        let err = validate(&[SearchFilter {
            key: "ordering".to_owned(),
            value: "newest".to_owned(),
        }])
        .expect_err("PornOne accepts no filters at all");
        assert!(err.to_string().contains("ordering"), "{err}");
    }
}
