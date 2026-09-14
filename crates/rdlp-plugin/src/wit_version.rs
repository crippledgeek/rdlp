//! Compile-time derivation of the WIT contract version from the `.wit`
//! source itself (#327), so `HOST_WIT_VERSION` cannot drift from
//! `package rdlp:plugin@X.Y.Z;`.

/// The `X.Y.Z` of the first `package rdlp:plugin@X.Y.Z;` line in `src`.
///
/// `const`: evaluated at compile time over `include_str!` input, so a
/// malformed directive is a build error, not a runtime one. Byte-wise
/// because `str` slicing is not `const`; the result is re-validated as
/// UTF-8 by `from_utf8`, which is `const` since Rust 1.63.
///
/// Walks the bytes via slice patterns and `split_at` rather than `[idx]` or
/// `.get(idx)` — `indexing_slicing` is warned for this crate's library code
/// (see `lib.rs`) and `slice::get` is not yet const-stable at this crate's
/// MSRV (1.88).
///
/// # Panics
///
/// Panics at compile time (this is a `const fn` evaluated over
/// `include_str!` input) when `src` has no `package rdlp:plugin@X.Y.Z;`
/// directive, or when the text between `@` and `;` is not valid UTF-8.
#[must_use]
pub const fn package_version(src: &str) -> &str {
    const NEEDLE: &[u8] = b"package rdlp:plugin@";
    match find_after(src.as_bytes(), NEEDLE) {
        Some(rest) => {
            let (ver, _) = rest.split_at(find_semicolon(rest));
            match core::str::from_utf8(ver) {
                Ok(s) => s,
                Err(_) => panic!("package version is not UTF-8"),
            }
        }
        None => panic!("no `package rdlp:plugin@X.Y.Z;` directive found"),
    }
}

/// The suffix of `hay` immediately after the first occurrence of `needle`,
/// or `None` if `needle` never occurs.
const fn find_after<'a>(hay: &'a [u8], needle: &[u8]) -> Option<&'a [u8]> {
    if starts_with(hay, needle) {
        let (_, rest) = hay.split_at(needle.len());
        return Some(rest);
    }
    match hay {
        [] => None,
        [_, rest @ ..] => find_after(rest, needle),
    }
}

const fn starts_with(hay: &[u8], needle: &[u8]) -> bool {
    match (hay, needle) {
        (_, []) => true,
        ([], _) => false,
        ([h, hs @ ..], [n, ns @ ..]) => *h == *n && starts_with(hs, ns),
    }
}

/// Byte offset of the first `;` in `s`, or `s.len()` if there is none.
const fn find_semicolon(s: &[u8]) -> usize {
    match s {
        [b';', ..] | [] => 0,
        [_, rest @ ..] => 1 + find_semicolon(rest),
    }
}

#[cfg(test)]
mod tests {
    use super::package_version;

    #[test]
    fn extracts_version_after_the_at_sign() {
        assert_eq!(
            package_version("package rdlp:plugin@0.5.1;\n\ninterface x {}"),
            "0.5.1"
        );
    }

    #[test]
    fn ignores_leading_comment_lines() {
        assert_eq!(
            package_version("// hi\npackage rdlp:plugin@1.2.3;"),
            "1.2.3"
        );
    }

    #[test]
    #[should_panic(expected = "no `package rdlp:plugin@")]
    fn missing_directive_panics() {
        let _ = package_version("interface x {}");
    }
}
