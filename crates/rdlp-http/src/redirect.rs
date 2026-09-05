//! The redirect policy installed on every client this crate builds.
//!
//! `rdlp_security::validate_url_security` inspects a URL as given, so a caller
//! that validates a public seed learns nothing about where a 302 sends the
//! client. The hops happen inside a single `send()`, so no call site can
//! re-validate what it never observes — the policy is the only place that sees
//! each one (#662).

use rdlp_redact::RedactedUrlBuf;
use std::fmt::Display;

/// A redirect was refused because its target failed the SSRF gate.
///
/// Names the hop and the target it refused, so an operator can tell which link
/// in a CDN chain broke rather than only that one did. `target` is a
/// [`RedactedUrlBuf`] rather than a pre-sanitized `String` so the redaction is
/// a property of the type: a `Location` may carry userinfo, and this error
/// reaches logs through `wreq::Error`'s Display and its derived Debug. This
/// crate is outside `scripts/check-url-redaction.sh`'s scope, so the type is
/// the only backstop.
///
/// The refused response's body is deliberately absent: it is never read.
#[derive(Debug, thiserror::Error)]
#[error("redirect hop {hop} to {target} refused by SSRF validation: {reason}")]
pub struct RedirectRefused {
    hop: usize,
    target: RedactedUrlBuf,
    reason: String,
}

/// A redirect policy that re-validates every hop before following it.
///
/// `validate` is a parameter rather than a direct call so tests can drive the
/// following, hop-limit and Location-resolution behaviour against a server on
/// loopback, which the real validator refuses by design.
///
/// The injected validator IS the guard — a permissive one removes the check
/// entirely, which is exactly what this crate's tests do. What confines that
/// is reachability, not the signature: `build_with_validator` is
/// `pub(crate)`, so no consumer of this crate can reach it, and the only
/// production caller (`HttpClientFactory::build_inner`) passes
/// `rdlp_security::validate_url_security`. A second in-crate caller would be
/// free to weaken it, and would need reviewing on that basis.
///
/// A blanket `#[cfg(test)]` loopback exemption — the pattern used at the
/// call-site gates in `rdlp-api` and `rdlp-extractor` — would be wrong here:
/// it would disable this guard throughout this crate's own tests, including
/// the test that proves the guard bites. Those two gates also cannot be
/// converged into one shared helper, because `cfg(test)` is set only for the
/// crate being test-compiled; `rdlp-security` built as a dependency of another
/// crate's test target has it unset. A cargo feature would reach across that
/// boundary, and is worse: features are additive and unify across the graph,
/// so one crate enabling it would disarm the gate for every consumer in the
/// build, where `cfg(test)` cannot escape its crate.
pub fn ssrf_guarded_redirect_policy<V, E>(
    max_redirects: usize,
    validate: V,
) -> wreq::redirect::Policy
where
    V: Fn(&str) -> Result<(), E> + Send + Sync + 'static,
    E: Display,
{
    // `Policy::custom` does NOT apply a hop limit — wreq documents this
    // explicitly — so the limit is delegated to a `limited` policy rather
    // than counted again here.
    let hop_limit = wreq::redirect::Policy::limited(max_redirects);

    wreq::redirect::Policy::custom(move |attempt| {
        // `Attempt` exposes `uri`/`previous` as FIELDS. wreq's own doc
        // examples call `attempt.uri()`, which does not exist on this version
        // and does not compile.
        let target = attempt.uri.to_string();
        if let Err(e) = validate(&target) {
            // `previous` holds the initial URI plus each followed hop, so its
            // length is already a 1-based hop number: 1 on the first redirect.
            let hop = attempt.previous.len();
            return attempt.error(RedirectRefused {
                hop,
                target: RedactedUrlBuf::from(target),
                reason: rdlp_security::sanitize_for_logging(&e.to_string()),
            });
        }
        hop_limit.redirect(attempt)
    })
}

#[cfg(test)]
mod tests;
