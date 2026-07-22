//! A first-class boolean rule over a subject, and the combinators that build
//! them.
//!
//! # Why a trait and not `Box<dyn Fn(T) -> bool>`
//!
//! The predecessor of this module was a single `VideoRule` type alias over a
//! boxed closure (#647). A trait costs nothing extra — the blanket impl below
//! means every closure already *is* a `Rule` — while letting a rule that
//! captures nothing be a 1-byte named type, and letting a future combinator
//! compose without boxing each intermediate (none exists yet; `Always` below
//! is the only rule this crate builds today).
//!
//! This mirrors `winnow::Parser`, already a dependency of this crate, whose
//! combinators are provided methods returning concrete types rather than trait
//! objects. The `predicates` crate was evaluated and rejected: its
//! `Predicate<Item>: PredicateReflection: Display` supertrait requires every
//! rule to be a named type with a `Display` impl, which would turn three rule
//! factories into twenty-six structs, and it is an assertion-oriented crate
//! this foundation crate should not depend on.

/// A boolean question asked of a `T`.
///
/// Implemented blanketly for every `Fn(T) -> bool`, so a plain closure is a
/// rule with no ceremony. Object-safe: `T` is a parameter of the trait, not of
/// [`eval`](Rule::eval), so `Box<dyn Rule<T>>` works and is what a dispatch
/// table returning a different concrete rule per arm needs.
pub trait Rule<T> {
    /// Answer the rule for `subject`.
    fn eval(&self, subject: T) -> bool;
}

impl<T, F> Rule<T> for F
where
    F: Fn(T) -> bool,
{
    fn eval(&self, subject: T) -> bool {
        self(subject)
    }
}

/// A rule that ignores its subject entirely and always answers the same way.
///
/// Not generic over the subject: a rule that captures nothing needs no
/// per-`T` instantiation, and a single concrete type satisfies `Rule<T>` for
/// every `T` via the blanket impl below — including the higher-ranked
/// `Rule<&'a T>` a boxed trait-object dispatch table needs. A prior
/// `Always<T>` version (parameterised, `PhantomData<fn(T)>`-carrying) could
/// only implement `Rule<T>` for the one concrete `T` it was built with, so it
/// could not satisfy `for<'a> Rule<&'a SourceVideo>` and call sites had to
/// wrap it in a closure just to get the right bound — reconstructing `Always`
/// on every `eval` for no benefit.
#[derive(Debug, Clone, Copy)]
pub struct Always(bool);

impl<T> Rule<T> for Always {
    fn eval(&self, _subject: T) -> bool {
        self.0
    }
}

/// Build a rule that answers `answer` for every subject, for any subject type.
#[must_use]
pub const fn always(answer: bool) -> Always {
    Always(answer)
}

#[cfg(test)]
mod tests {
    use super::{Rule, always};

    #[test]
    fn a_closure_is_a_rule() {
        let is_even = |n: u32| n.is_multiple_of(2);
        assert!(is_even.eval(4));
        assert!(!is_even.eval(5));
    }

    #[test]
    fn always_ignores_its_subject() {
        let yes = always(true);
        let no = always(false);
        assert!(Rule::<u32>::eval(&yes, 0));
        assert!(Rule::<u32>::eval(&yes, u32::MAX));
        assert!(!Rule::<u32>::eval(&no, 0));
    }

    #[test]
    fn rules_are_object_safe_and_boxable() {
        let rules: Vec<Box<dyn Rule<u32> + Send + Sync>> =
            vec![Box::new(|n: u32| n > 10), Box::new(always(false))];
        assert!(rules.first().expect("two rules").eval(11));
        assert!(!rules.get(1).expect("two rules").eval(11));
    }
}
