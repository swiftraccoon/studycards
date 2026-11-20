//! Identifiers for sets, topics, and cards.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The longest identifier accepted, in bytes.
pub const MAX_ID_LEN: usize = 64;

/// Why a string was rejected as an identifier.
///
/// A string with several problems is rejected for the first of them in the
/// order `Empty`, `BadStart`, `BadCharacter`, `TooLong`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdProblem {
    /// The string is empty.
    Empty,
    /// The string is longer than [`MAX_ID_LEN`] bytes.
    TooLong,
    /// The string does not start with a lowercase ASCII letter or digit.
    BadStart,
    /// The string contains a character other than a lowercase ASCII letter,
    /// digit, `-`, or `_`.
    BadCharacter(char),
}

impl Display for IdProblem {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Empty => f.write_str("an identifier cannot be empty"),
            Self::TooLong => write!(f, "an identifier has at most {MAX_ID_LEN} characters"),
            Self::BadStart => {
                f.write_str("an identifier starts with a lowercase letter or a digit")
            }
            Self::BadCharacter(character) => write!(
                f,
                "an identifier cannot contain '{character}'; use lowercase letters, digits, '-' \
                 and '_'"
            ),
        }
    }
}

impl Error for IdProblem {}

/// Checks that `text` can be used in file names and URLs without escaping.
fn check(text: &str) -> Result<(), IdProblem> {
    let Some(first) = text.chars().next() else {
        return Err(IdProblem::Empty);
    };
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err(IdProblem::BadStart);
    }
    if let Some(character) = text.chars().find(|&character| !is_id_character(character)) {
        return Err(IdProblem::BadCharacter(character));
    }
    // Every character of `text` is an ASCII character here, so its length in
    // bytes is its length in characters.
    if text.len() > MAX_ID_LEN {
        return Err(IdProblem::TooLong);
    }
    Ok(())
}

/// Returns `true` for the characters an identifier may contain.
const fn is_id_character(character: char) -> bool {
    character.is_ascii_lowercase() || character.is_ascii_digit() || matches!(character, '-' | '_')
}

/// Defines a validated identifier type.
macro_rules! id_type {
    ($(#[$attribute:meta])* $visibility:vis struct $name:ident;) => {
        $(#[$attribute])*
        ///
        /// An identifier is 1 to 64 lowercase ASCII letters, digits, `-`, or
        /// `_`, starting with a letter or digit.
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(try_from = "String", into = "String")]
        $visibility struct $name(String);

        impl $name {
            /// Returns the identifier as text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = IdProblem;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                check(text).map(|()| Self(text.to_owned()))
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdProblem;

            fn try_from(text: String) -> Result<Self, Self::Error> {
                check(&text).map(|()| Self(text))
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> Self {
                id.0
            }
        }
    };
}

id_type! {
    /// Identifies a study set; it is also the stem of the set's file name.
    pub struct SetId;
}

id_type! {
    /// Identifies a topic within one study set.
    pub struct TopicId;
}

id_type! {
    /// Identifies a card within one study set.
    pub struct CardId;
}

/// Turns free text into identifier text: lowercase ASCII letters and digits
/// are kept, every other run of characters becomes a single `-`.
///
/// Returns `None` when `text` has no ASCII letter or digit. The result is at
/// most `max_len` bytes and never longer than [`MAX_ID_LEN`].
pub(super) fn slugify(text: &str, max_len: usize) -> Option<String> {
    let limit = max_len.min(MAX_ID_LEN);
    let mut slug = String::new();
    let mut pending_separator = false;
    for character in text.chars() {
        if slug.len() >= limit {
            break;
        }
        if character.is_ascii_alphanumeric() {
            if pending_separator && !slug.is_empty() && slug.len().saturating_add(1) < limit {
                slug.push('-');
            }
            pending_separator = false;
            slug.push(character.to_ascii_lowercase());
        } else {
            pending_separator = true;
        }
    }
    (!slug.is_empty()).then_some(slug)
}

/// Returns `base`, or `base` followed by `-2`, `-3`, … so that `taken`
/// rejects the result.
///
/// `base` is shortened when needed to keep the result within [`MAX_ID_LEN`].
pub(super) fn first_free(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    let mut number: u32 = 2;
    loop {
        let suffix = format!("-{number}");
        let room = MAX_ID_LEN.saturating_sub(suffix.len());
        let stem: String = base.chars().take(room).collect();
        let candidate = format!("{stem}{suffix}");
        if !taken(&candidate) {
            return candidate;
        }
        number = number.saturating_add(1);
    }
}

/// Derives a stable identifier from text with the 64-bit FNV-1a hash.
pub(super) fn hashed(prefix: &str, text: &str) -> String {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let hash = text.bytes().fold(OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(PRIME)
    });
    format!("{prefix}{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::{CardId, IdProblem, MAX_ID_LEN, SetId, first_free, hashed, slugify};

    #[test]
    fn accepts_identifier_text() {
        for text in [
            "a",
            "solar-system",
            "q_1",
            "2024-notes",
            &"x".repeat(MAX_ID_LEN),
        ] {
            let parsed: Result<SetId, IdProblem> = text.parse();
            assert_eq!(
                parsed.map(|id| id.to_string()),
                Ok(text.to_owned()),
                "{text:?} is a valid identifier"
            );
        }
    }

    #[test]
    fn rejects_text_that_needs_escaping() {
        let cases = [
            ("", IdProblem::Empty),
            ("-a", IdProblem::BadStart),
            ("_a", IdProblem::BadStart),
            ("Abc", IdProblem::BadStart),
            ("a b", IdProblem::BadCharacter(' ')),
            ("a/b", IdProblem::BadCharacter('/')),
            ("a.b", IdProblem::BadCharacter('.')),
            ("aB", IdProblem::BadCharacter('B')),
            ("né", IdProblem::BadCharacter('é')),
        ];
        for (text, problem) in cases {
            let parsed: Result<CardId, IdProblem> = text.parse();
            assert_eq!(parsed, Err(problem), "{text:?} is rejected");
        }
        let long: Result<CardId, IdProblem> = "x".repeat(MAX_ID_LEN + 1).parse();
        assert_eq!(long, Err(IdProblem::TooLong), "65 characters is too long");
    }

    #[test]
    fn the_length_is_judged_last() {
        // Forty of these characters are eighty bytes.
        let accented: Result<CardId, IdProblem> = "é".repeat(40).parse();
        assert_eq!(
            accented,
            Err(IdProblem::BadStart),
            "a string of 40 characters is not reported as longer than 64"
        );
        let spaced: Result<CardId, IdProblem> = "a ".repeat(MAX_ID_LEN).parse();
        assert_eq!(
            spaced,
            Err(IdProblem::BadCharacter(' ')),
            "an overlong string is rejected for a character it must not contain"
        );
    }

    #[test]
    fn problems_are_explained() {
        let cases = [
            (IdProblem::Empty, "an identifier cannot be empty"),
            (
                IdProblem::TooLong,
                "an identifier has at most 64 characters",
            ),
            (
                IdProblem::BadStart,
                "an identifier starts with a lowercase letter or a digit",
            ),
            (
                IdProblem::BadCharacter('/'),
                "an identifier cannot contain '/'; use lowercase letters, digits, '-' and '_'",
            ),
        ];
        for (problem, message) in cases {
            assert_eq!(problem.to_string(), message, "message of {problem:?}");
        }
    }

    #[test]
    fn identifiers_round_trip_through_serde() {
        let id: CardId = serde_json::from_str("\"card-1\"").expect("a valid id parses");
        assert_eq!(id.as_str(), "card-1", "parsed identifier text");
        assert_eq!(
            serde_json::to_string(&id).expect("an id serializes"),
            "\"card-1\"",
            "an identifier is stored as a string"
        );
        let invalid: Result<CardId, serde_json::Error> = serde_json::from_str("\"Not Valid\"");
        assert!(
            invalid.is_err(),
            "invalid identifiers are rejected when parsing"
        );
    }

    #[test]
    fn slugify_keeps_letters_and_digits() {
        assert_eq!(
            slugify("  What is 2 + 2?  ", 40).as_deref(),
            Some("what-is-2-2"),
            "punctuation and spaces collapse to single dashes"
        );
        assert_eq!(
            slugify("Élan vital", 40).as_deref(),
            Some("lan-vital"),
            "non-ASCII is dropped"
        );
        assert_eq!(
            slugify("日本語", 40),
            None,
            "text without ASCII letters has no slug"
        );
        assert_eq!(slugify("?!", 40), None, "punctuation alone has no slug");
    }

    #[test]
    fn slugify_respects_the_length_limit() {
        let slug = slugify("alpha beta gamma delta", 12).expect("text has letters");
        assert!(slug.len() <= 12, "slug {slug:?} fits the limit");
        assert!(
            !slug.ends_with('-'),
            "slug {slug:?} does not end with a separator"
        );
        let long = slugify(&"word ".repeat(40), 500).expect("text has letters");
        assert!(
            long.len() <= MAX_ID_LEN,
            "a slug never exceeds the identifier limit"
        );
        let parsed: Result<SetId, IdProblem> = long.parse();
        assert!(parsed.is_ok(), "a slug is a valid identifier");
    }

    #[test]
    fn first_free_appends_a_counter() {
        let taken = ["planets", "planets-2"];
        assert_eq!(
            first_free("moons", |id| taken.contains(&id)),
            "moons",
            "a free id is kept"
        );
        assert_eq!(
            first_free("planets", |id| taken.contains(&id)),
            "planets-3",
            "the first unused counter is appended"
        );
        let base = "y".repeat(MAX_ID_LEN);
        let unique = first_free(&base, |id| id == base);
        assert_eq!(
            unique.len(),
            MAX_ID_LEN,
            "the base is shortened to make room"
        );
        assert!(
            unique.ends_with("-2"),
            "the counter is kept when shortening"
        );
    }

    #[test]
    fn hashed_ids_are_stable_and_valid() {
        let id = hashed("q-", "What is the capital of France?");
        assert_eq!(
            id,
            hashed("q-", "What is the capital of France?"),
            "hashing is deterministic"
        );
        assert_ne!(
            id,
            hashed("q-", "What is the capital of Spain?"),
            "different text differs"
        );
        assert_eq!(id.len(), 18, "prefix plus sixteen hex digits");
        let parsed: Result<CardId, IdProblem> = id.parse();
        assert!(parsed.is_ok(), "a hashed id is a valid identifier");
        assert_eq!(
            hashed("q-", ""),
            "q-cbf29ce484222325",
            "FNV-1a offset basis for empty input"
        );
        assert_eq!(
            hashed("", "a"),
            "af63dc4c8601ec8c",
            "FNV-1a reference vector for a"
        );
        assert_eq!(
            hashed("", "foobar"),
            "85944171f73967e8",
            "FNV-1a reference vector for foobar"
        );
    }
}
