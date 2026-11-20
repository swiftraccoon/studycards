//! Why a study set could not be read or changed.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use super::card::CardProblem;
use super::ids::{CardId, IdProblem, TopicId};
use super::text::{self, TextProblem};

/// The most characters of a parser's message that an error repeats. The
/// message quotes the file.
const MESSAGE_CHARS: usize = 2000;
/// The most characters of a question or of a rejected identifier that an
/// error repeats.
const QUOTED_CHARS: usize = 200;

/// Why a study set could not be read or changed.
///
/// The message of an error repeats at most an excerpt of the text that it
/// names, so it stays short whatever a file holds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeckError {
    /// The text is not TOML in the study-set format.
    Format(String),
    /// The set title is unusable.
    Title(TextProblem),
    /// A topic name is unusable.
    TopicName(TextProblem),
    /// An identifier in the file is unusable.
    InvalidId {
        /// The rejected identifier.
        text: String,
        /// Why it was rejected.
        problem: IdProblem,
    },
    /// Two topics have this identifier.
    DuplicateTopic(TopicId),
    /// Two cards have this identifier.
    DuplicateCard(CardId),
    /// A card is unusable.
    InvalidCard {
        /// The card's question, to locate it.
        question: String,
        /// What is wrong with it.
        problem: CardProblem,
    },
    /// The set has no topic with this identifier.
    UnknownTopic(TopicId),
    /// The set has no card with this identifier.
    UnknownCard(CardId),
}

impl Display for DeckError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Format(message) => write!(
                f,
                "not a study set file: {}",
                text::excerpt(message, MESSAGE_CHARS)
            ),
            Self::Title(problem) => write!(f, "the set title {problem}"),
            Self::TopicName(problem) => write!(f, "a topic name {problem}"),
            Self::InvalidId {
                text: rejected,
                problem,
            } => write!(
                f,
                "\"{}\" cannot be used as an identifier: {problem}",
                text::excerpt(rejected, QUOTED_CHARS)
            ),
            Self::DuplicateTopic(id) => write!(f, "two topics share the identifier \"{id}\""),
            Self::DuplicateCard(id) => write!(
                f,
                "two cards share the identifier \"{id}\"; give cards with the same question \
                 different `id` values"
            ),
            Self::InvalidCard { question, problem } => write!(
                f,
                "card \"{}\": {problem}",
                text::excerpt(question, QUOTED_CHARS)
            ),
            Self::UnknownTopic(id) => write!(f, "there is no topic \"{id}\""),
            Self::UnknownCard(id) => write!(f, "there is no card \"{id}\""),
        }
    }
}

impl Error for DeckError {}

/// Parses identifier text, reporting a rejection as a [`DeckError`].
pub(super) fn parse_id<Id: TryFrom<String, Error = IdProblem>>(
    text: String,
) -> Result<Id, DeckError> {
    Id::try_from(text.clone()).map_err(|problem| DeckError::InvalidId { text, problem })
}

#[cfg(test)]
mod tests {
    use super::{CardId, CardProblem, DeckError, IdProblem, TextProblem, TopicId, parse_id};

    #[test]
    fn errors_read_as_sentences() {
        let topic: TopicId = "planets".parse().expect("valid topic id");
        let card: CardId = "red-planet".parse().expect("valid card id");
        let cases = [
            (
                DeckError::Format("expected a string".to_owned()),
                "not a study set file: expected a string",
            ),
            (
                DeckError::Title(TextProblem::TooLong { limit: 120 }),
                "the set title is longer than 120 characters",
            ),
            (
                DeckError::TopicName(TextProblem::Empty),
                "a topic name is empty",
            ),
            (
                DeckError::InvalidId {
                    text: "Bad Id".to_owned(),
                    problem: IdProblem::BadStart,
                },
                "\"Bad Id\" cannot be used as an identifier: an identifier starts with a \
                 lowercase letter or a digit",
            ),
            (
                DeckError::DuplicateTopic(topic.clone()),
                "two topics share the identifier \"planets\"",
            ),
            (
                DeckError::DuplicateCard(card.clone()),
                "two cards share the identifier \"red-planet\"; give cards with the same \
                 question different `id` values",
            ),
            (
                DeckError::InvalidCard {
                    question: "Largest planet?".to_owned(),
                    problem: CardProblem::WrongAnswerCount(0),
                },
                "card \"Largest planet?\": it has 0 wrong answers; a card needs 1 to 5",
            ),
            (
                DeckError::UnknownTopic(topic),
                "there is no topic \"planets\"",
            ),
            (
                DeckError::UnknownCard(card),
                "there is no card \"red-planet\"",
            ),
        ];
        for (error, message) in cases {
            assert_eq!(error.to_string(), message, "message of {error:?}");
        }
    }

    #[test]
    fn messages_stay_short_whatever_they_quote() {
        let long = "x".repeat(1_000_000);
        let errors = [
            DeckError::Format(long.clone()),
            DeckError::InvalidId {
                text: long.clone(),
                problem: IdProblem::TooLong,
            },
            DeckError::InvalidCard {
                question: long,
                problem: CardProblem::WrongAnswerCount(0),
            },
        ];
        for error in errors {
            let message = error.to_string();
            assert!(
                message.chars().count() < 2100,
                "a message of {} characters",
                message.chars().count()
            );
            assert!(message.contains("x…"), "the cut is marked: {message}");
        }
    }

    #[test]
    fn a_rejected_identifier_is_reported_with_its_text() {
        let accepted: Result<TopicId, DeckError> = parse_id("moons".to_owned());
        assert_eq!(
            accepted.as_ref().map(TopicId::as_str),
            Ok("moons"),
            "identifier text is accepted"
        );
        let rejected: Result<CardId, DeckError> = parse_id("Moons".to_owned());
        assert_eq!(
            rejected,
            Err(DeckError::InvalidId {
                text: "Moons".to_owned(),
                problem: IdProblem::BadStart,
            }),
            "the rejection names the text and the reason"
        );
    }
}
