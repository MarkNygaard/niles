//! Telling that two ways of writing a product are the same product.
//!
//! The words arrive from a speech recogniser that hears Danish through
//! English ears — "letmælk" comes back as "let melk" — and from people
//! typing on phones. Neither is worth an LLM call when the answer is a
//! letter or two away from a product the house buys every week.

/// The form two names are compared in.
///
/// Lowercase, Danish letters spelled out the way a keyboard without
/// them spells them (`æ` → `ae`), punctuation dropped, and a leading
/// "some" or "a" taken off: "Some milk" and "milk" are one request.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            'æ' | 'ä' => out.push_str("ae"),
            'ø' | 'ö' => out.push_str("oe"),
            'å' => out.push_str("aa"),
            'é' | 'è' => out.push('e'),
            'ü' => out.push('u'),
            c if c.is_alphanumeric() => out.push(c),
            _ => out.push(' '),
        }
    }
    let words: Vec<&str> = out.split_whitespace().collect();
    let words = match words.as_slice() {
        [first, rest @ ..] if !rest.is_empty() && FILLER.contains(first) => rest,
        all => all,
    };
    words.join(" ")
}

const FILLER: &[&str] = &["a", "an", "some", "the", "more"];

/// How far apart two normalized names may be and still be one product.
///
/// Spaces do not count: "let melk" and "letmaelk" differ by one letter,
/// not by a space and a letter. Short names get no slack at all, because
/// short Danish groceries are each other's near neighbours — *mel*
/// (flour) is two letters from *mælk*, and *øl* (beer) is not *æg*.
pub(crate) fn close_enough(a: &str, b: &str) -> Option<usize> {
    let a: Vec<char> = a.chars().filter(|c| *c != ' ').collect();
    let b: Vec<char> = b.chars().filter(|c| *c != ' ').collect();
    let allowed = match a.len().min(b.len()) {
        0..=4 => 0,
        5..=8 => 1,
        _ => 2,
    };
    let d = distance(&a, &b);
    (d <= allowed).then_some(d)
}

/// Levenshtein distance.
fn distance(a: &[char], b: &[char]) -> usize {
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitute = previous[j] + usize::from(ca != cb);
            current[j + 1] = substitute.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spells_out_danish_letters() {
        assert_eq!(normalize("Letmælk"), "letmaelk");
        assert_eq!(normalize("Rødkål"), "roedkaal");
    }

    #[test]
    fn drops_punctuation_and_extra_space() {
        assert_eq!(normalize("  Skyr,  vanilje! "), "skyr vanilje");
    }

    #[test]
    fn drops_a_leading_some_but_not_a_whole_name() {
        assert_eq!(normalize("some milk"), "milk");
        assert_eq!(normalize("a cucumber"), "cucumber");
        // On its own it is the name, however odd.
        assert_eq!(normalize("more"), "more");
    }

    #[test]
    fn a_misheard_danish_word_is_close_enough() {
        assert_eq!(close_enough("let melk", "letmaelk"), Some(1));
        assert_eq!(close_enough("leverpostei", "leverpostej"), Some(1));
    }

    #[test]
    fn short_names_must_match_exactly() {
        // Flour is not milk, and beer is not eggs.
        assert_eq!(close_enough("mel", "maelk"), None);
        assert_eq!(close_enough("oel", "aeg"), None);
        assert_eq!(close_enough("mel", "mel"), Some(0));
    }

    #[test]
    fn different_products_stay_different() {
        assert_eq!(close_enough("letmaelk", "soedmaelk"), None);
        assert_eq!(close_enough("rugbroed", "franskbroed"), None);
    }
}
