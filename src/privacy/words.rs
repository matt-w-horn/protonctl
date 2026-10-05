//! The word list for aliases and keyed digests (RFC Q18): the EFF large
//! diceware list, by Joseph Bonneau for the Electronic Frontier Foundation
//! (<https://www.eff.org/deeplinks/2016/07/new-wordlists-random-passphrases>),
//! under the Creative Commons Attribution 4.0 International licence that
//! EFF's copyright policy (<https://www.eff.org/copyright>) gives its original
//! material (<https://creativecommons.org/licenses/by/4.0/>), in its order,
//! less the words `words-dropped.txt` lists with their reasons. The order is
//! fixed: changing it changes every alias, a format change (the `v1` in the
//! key labels).

use std::collections::HashSet;
use std::sync::LazyLock;

static WORDS: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| include_str!("words.txt").lines().collect());

static SET: LazyLock<HashSet<&'static str>> = LazyLock::new(|| WORDS.iter().copied().collect());

#[cfg(test)]
thread_local! {
    /// A list a test puts in place of this one on its own thread, short
    /// enough for aliases to collide (RFC section 7, Collisions).
    static SHORT: std::cell::Cell<Option<&'static [&'static str]>> =
        const { std::cell::Cell::new(None) };
}

/// The list aliases and digests are made from.
fn list() -> &'static [&'static str] {
    #[cfg(test)]
    if let Some(short) = SHORT.get() {
        return short;
    }
    &WORDS
}

/// `f`, with aliases and digests made from `short` on this thread.
#[cfg(test)]
pub fn with_list<T>(short: &'static [&'static str], f: impl FnOnce() -> T) -> T {
    SHORT.set(Some(short));
    let out = f();
    SHORT.set(None);
    out
}

/// Whether `w`, in lower case, is on the list: a common English word.
pub fn contains(w: &str) -> bool {
    SET.contains(w)
}

/// How many words there are, `N`.
pub fn count() -> u64 {
    list().len() as u64
}

/// The word at `i`, which must be below `count()`.
pub fn word(i: u64) -> &'static str {
    list()[usize::try_from(i).expect("an index below count() fits in usize")]
}

/// `v` written as `n` words, most significant first, each a digit in base `N`.
pub fn words(mut v: u64, n: u32) -> Vec<&'static str> {
    let base = count();
    let mut out = vec![""; n as usize];
    for slot in out.iter_mut().rev() {
        *slot = word(v % base);
        v /= base;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_large_plain_and_unique() {
        // Five words must hold 64 bits: N^5 >= 2^64 needs N >= 7,132.
        assert!(count() >= 7_132, "{}", count());
        assert!(u128::from(count()).pow(5) > u128::from(u64::MAX));
        let mut seen = std::collections::HashSet::new();
        for w in WORDS.iter() {
            assert!(
                !w.is_empty() && w.bytes().all(|b| b.is_ascii_lowercase()),
                "{w:?}"
            );
            assert!(seen.insert(*w), "{w} twice");
        }
    }

    #[test]
    fn numbers_become_words_most_significant_first() {
        assert_eq!(words(0, 3), [word(0); 3]);
        assert_eq!(words(1, 3), [word(0), word(0), word(1)]);
        let n = count();
        assert_eq!(words(n * n + 2 * n + 3, 3), [word(1), word(2), word(3)]);
    }
}
