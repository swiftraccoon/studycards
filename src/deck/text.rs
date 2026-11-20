//! Checking the text of titles, names, questions, and answers.

use std::fmt::{self, Display, Formatter};

/// What is wrong with a piece of text.
///
/// A problem is displayed as the end of a sentence that begins with the name
/// of the text, such as "is empty" after "the set title".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextProblem {
    /// The text is empty or only whitespace.
    Empty,
    /// The text is longer than `limit` characters.
    TooLong {
        /// The number of characters allowed.
        limit: usize,
    },
}

impl Display for TextProblem {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Empty => f.write_str("is empty"),
            Self::TooLong { limit } => write!(f, "is longer than {limit} characters"),
        }
    }
}

/// Trims `text` and checks that it is non-empty and at most `limit` characters.
pub(super) fn clean(text: &str, limit: usize) -> Result<String, TextProblem> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(TextProblem::Empty);
    }
    if trimmed.chars().count() > limit {
        return Err(TextProblem::TooLong { limit });
    }
    Ok(trimmed.to_owned())
}

/// Returns the first `limit` characters of `text`, followed by `…` when
/// `text` has more.
///
/// A message that repeats text from a file repeats an excerpt, so that the
/// message stays short whatever the file holds.
pub(super) fn excerpt(text: &str, limit: usize) -> String {
    let mut shown: String = text.chars().take(limit).collect();
    if text.chars().nth(limit).is_some() {
        shown.push('…');
    }
    shown
}

#[cfg(test)]
mod tests {
    use super::{TextProblem, clean, excerpt};

    #[test]
    fn text_is_trimmed_before_it_is_measured() {
        assert_eq!(
            clean("  two words \n", 9).as_deref(),
            Ok("two words"),
            "surrounding whitespace is removed and does not count"
        );
        assert_eq!(
            clean(" \t\n", 9),
            Err(TextProblem::Empty),
            "whitespace alone is empty"
        );
        assert_eq!(clean("", 9), Err(TextProblem::Empty), "no text is empty");
    }

    #[test]
    fn the_limit_counts_characters() {
        assert_eq!(
            clean("naïve café", 10).as_deref(),
            Ok("naïve café"),
            "ten characters fit a limit of ten, whatever their size in bytes"
        );
        assert_eq!(
            clean("naïve café!", 10),
            Err(TextProblem::TooLong { limit: 10 }),
            "eleven characters exceed a limit of ten"
        );
    }

    #[test]
    fn an_excerpt_keeps_the_beginning() {
        assert_eq!(excerpt("", 3), "", "nothing to shorten");
        assert_eq!(excerpt("abc", 3), "abc", "text of the limit is whole");
        assert_eq!(excerpt("abcd", 3), "abc…", "longer text is marked");
        assert_eq!(
            excerpt("naïve café", 5),
            "naïve…",
            "the limit counts characters"
        );
    }

    #[test]
    fn problems_complete_a_sentence() {
        assert_eq!(TextProblem::Empty.to_string(), "is empty", "empty text");
        assert_eq!(
            TextProblem::TooLong { limit: 120 }.to_string(),
            "is longer than 120 characters",
            "overlong text"
        );
    }
}
