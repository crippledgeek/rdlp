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
/// Walks the bytes via `while let` slice patterns rather than `[idx]` or
/// `.get(idx)` — `indexing_slicing` is warned for this crate's library code
/// (see `lib.rs`) and `slice::get` is not yet const-stable at this crate's
/// MSRV (1.88) — AND iteratively rather than one-byte-per-recursive-call:
/// a recursive walk hits rustc's const-eval stack-frame cap on a realistic
/// `.wit` file (measured: a ~154-byte leading header already failed with
/// `E0080 reached the configured maximum number of stack frames`; `.wit`
/// files carry multi-paragraph doc comments before their first `package`
/// line, so a long header is normal input, not an adversarial one).
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
            let (ver, _) = rest.split_at(semicolon_offset(rest));
            match core::str::from_utf8(ver) {
                Ok(s) => s,
                Err(_) => panic!("package version is not UTF-8"),
            }
        }
        None => panic!("no `package rdlp:plugin@X.Y.Z;` directive found"),
    }
}

/// The suffix of `hay` immediately after the first occurrence of `needle`,
/// or `None` if `needle` never occurs. Iterative: advances one byte per
/// loop iteration via `split_at`, never recursing per byte.
const fn find_after<'a>(hay: &'a [u8], needle: &[u8]) -> Option<&'a [u8]> {
    let mut window = hay;
    loop {
        if starts_with(window, needle) {
            let (_, rest) = window.split_at(needle.len());
            return Some(rest);
        }
        match window {
            [] => return None,
            [_, rest @ ..] => window = rest,
        }
    }
}

const fn starts_with(hay: &[u8], needle: &[u8]) -> bool {
    let mut h = hay;
    let mut n = needle;
    loop {
        match (h, n) {
            (_, []) => return true,
            ([], _) => return false,
            ([hb, h_rest @ ..], [nb, n_rest @ ..]) => {
                if *hb != *nb {
                    return false;
                }
                h = h_rest;
                n = n_rest;
            }
        }
    }
}

/// Byte offset of the first `;` in `s`, or `s.len()` if there is none.
const fn semicolon_offset(s: &[u8]) -> usize {
    let mut rest = s;
    let mut offset = 0;
    while let [b, tail @ ..] = rest {
        if *b == b';' {
            break;
        }
        rest = tail;
        offset += 1;
    }
    offset
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

    /// A leading header long enough to overflow rustc's const-eval frame
    /// cap under a byte-per-recursive-call implementation. Must be
    /// evaluated in a CONST context (a `const` item initializer), not a
    /// runtime call — the frame cap is a const-eval limit only; calling a
    /// `const fn` from ordinary runtime code is just a normal function
    /// call with normal stack depth and would not reproduce the failure.
    /// `.wit` files carry multi-paragraph doc comments before their first
    /// `package` line (see `host.wit`'s `host-extract-helpers` docs), so a
    /// large header is realistic input, not an adversarial one.
    #[test]
    fn tolerates_a_large_leading_header() {
        const SRC: &str = include_str!("wit_version_test_fixtures/large_header.wit");
        const RESULT: &str = package_version(SRC);
        assert_eq!(RESULT, "9.9.9");
    }
}
