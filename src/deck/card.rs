//! Cards: one multiple-choice question each.

use std::fmt::{self, Display, Formatter};

use super::ids::CardId;
use super::text::{self, TextProblem};

/// The longest question or explanation, in characters.
pub const MAX_TEXT_LEN: usize = 4000;
/// The longest answer, in characters.
pub const MAX_ANSWER_LEN: usize = 500;
/// The most wrong answers a card can have; with the correct answer this gives
/// at most six choices.
pub const MAX_WRONG_ANSWERS: usize = 5;

/// What is wrong with a card.
///
/// A problem is displayed as a clause, such as "the question is empty", for a
/// message that names the card.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CardProblem {
    /// The question is unusable.
    Question(TextProblem),
    /// The correct answer is unusable.
    Answer(TextProblem),
    /// One of the wrong answers is unusable.
    WrongAnswer(TextProblem),
    /// The explanation is too long.
    Explanation(TextProblem),
    /// The card has this many wrong answers instead of one to
    /// [`MAX_WRONG_ANSWERS`].
    WrongAnswerCount(usize),
    /// The same answer text appears twice.
    DuplicateChoice(String),
}

impl Display for CardProblem {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Question(problem) => write!(f, "the question {problem}"),
            Self::Answer(problem) => write!(f, "the correct answer {problem}"),
            Self::WrongAnswer(problem) => write!(f, "a wrong answer {problem}"),
            Self::Explanation(problem) => write!(f, "the explanation {problem}"),
            Self::WrongAnswerCount(count) => write!(
                f,
                "it has {count} wrong answers; a card needs 1 to {MAX_WRONG_ANSWERS}"
            ),
            Self::DuplicateChoice(text) => write!(f, "the answer \"{text}\" appears twice"),
        }
    }
}

/// The content of a card before it has an identifier.
///
/// Text is trimmed when the draft is turned into a card, and blank wrong
/// answers and a blank explanation are dropped.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CardDraft {
    /// The question shown to the learner.
    pub question: String,
    /// The correct answer.
    pub answer: String,
    /// The wrong answers offered beside the correct one.
    pub wrong: Vec<String>,
    /// Text shown after the card has been answered.
    pub explanation: Option<String>,
}

/// One multiple-choice question.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Card {
    /// The card's identifier.
    id: CardId,
    /// The question text.
    question: String,
    /// The answers; the first is the correct one.
    choices: Vec<String>,
    /// Text shown after answering.
    explanation: Option<String>,
}

impl Card {
    /// The position of the correct answer in [`Card::choices`].
    pub const CORRECT_CHOICE: usize = 0;

    /// Validates `draft` and gives it an identifier.
    pub(super) fn try_new(id: CardId, draft: &CardDraft) -> Result<Self, CardProblem> {
        let question = text::clean(&draft.question, MAX_TEXT_LEN).map_err(CardProblem::Question)?;
        let answer = text::clean(&draft.answer, MAX_ANSWER_LEN).map_err(CardProblem::Answer)?;
        // The answers are counted before they are compared with each other:
        // a draft can hold any number of them, and comparing every pair of a
        // long list would take time in proportion to the square of its length.
        let offered: Vec<&String> = draft
            .wrong
            .iter()
            .filter(|wrong| !wrong.trim().is_empty())
            .collect();
        if !(1..=MAX_WRONG_ANSWERS).contains(&offered.len()) {
            return Err(CardProblem::WrongAnswerCount(offered.len()));
        }
        let mut choices = vec![answer];
        for offer in offered {
            let wrong = text::clean(offer, MAX_ANSWER_LEN).map_err(CardProblem::WrongAnswer)?;
            if choices.contains(&wrong) {
                return Err(CardProblem::DuplicateChoice(wrong));
            }
            choices.push(wrong);
        }
        let explanation = match draft.explanation.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(given) => {
                Some(text::clean(given, MAX_TEXT_LEN).map_err(CardProblem::Explanation)?)
            }
        };
        Ok(Self {
            id,
            question,
            choices,
            explanation,
        })
    }

    /// Returns the card's identifier.
    #[must_use]
    pub const fn id(&self) -> &CardId {
        &self.id
    }

    /// Returns the question text.
    #[must_use]
    pub fn question(&self) -> &str {
        &self.question
    }

    /// Returns the answers; the one at [`Card::CORRECT_CHOICE`] is correct.
    #[must_use]
    pub fn choices(&self) -> &[String] {
        &self.choices
    }

    /// Returns the correct answer.
    #[must_use]
    pub fn answer(&self) -> &str {
        self.choices.first().map_or("", String::as_str)
    }

    /// Returns the wrong answers.
    #[must_use]
    pub fn wrong(&self) -> &[String] {
        self.choices.get(1..).unwrap_or_default()
    }

    /// Returns the text shown after answering, if the card has any.
    #[must_use]
    pub fn explanation(&self) -> Option<&str> {
        self.explanation.as_deref()
    }

    /// Returns the card's content without its identifier.
    #[must_use]
    pub fn to_draft(&self) -> CardDraft {
        CardDraft {
            question: self.question.clone(),
            answer: self.answer().to_owned(),
            wrong: self.wrong().to_vec(),
            explanation: self.explanation.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Card, CardDraft, CardId, CardProblem, TextProblem};
    use super::{MAX_ANSWER_LEN, MAX_TEXT_LEN, MAX_WRONG_ANSWERS};

    fn id() -> CardId {
        "card".parse().expect("valid card id")
    }

    fn draft(question: &str, answer: &str, wrong: &[&str]) -> CardDraft {
        CardDraft {
            question: question.to_owned(),
            answer: answer.to_owned(),
            wrong: wrong.iter().map(|&text| text.to_owned()).collect(),
            explanation: None,
        }
    }

    #[test]
    fn the_correct_answer_is_the_first_choice() {
        let card = Card::try_new(
            id(),
            &draft("Closest to the Sun?", "Mercury", &["Venus", "Mars"]),
        )
        .expect("valid card");
        assert_eq!(card.id(), &id(), "the card keeps the identifier given");
        assert_eq!(card.question(), "Closest to the Sun?", "question");
        assert_eq!(
            card.choices(),
            ["Mercury", "Venus", "Mars"],
            "the correct answer comes first"
        );
        assert_eq!(
            card.choices().get(Card::CORRECT_CHOICE).map(String::as_str),
            Some(card.answer()),
            "the correct answer is at its documented position"
        );
        assert_eq!(card.wrong(), ["Venus", "Mars"], "wrong answers");
        assert_eq!(card.explanation(), None, "no explanation was given");
    }

    #[test]
    fn a_draft_made_from_a_card_gives_the_same_card() {
        let mut original = draft("  Largest planet?  ", " Jupiter ", &[" Saturn ", "  "]);
        original.explanation = Some(" It has the most mass. ".to_owned());
        let card = Card::try_new(id(), &original).expect("valid card");
        let cleaned = card.to_draft();
        assert_eq!(
            cleaned,
            CardDraft {
                question: "Largest planet?".to_owned(),
                answer: "Jupiter".to_owned(),
                wrong: vec!["Saturn".to_owned()],
                explanation: Some("It has the most mass.".to_owned()),
            },
            "the draft holds the trimmed text without blank answers"
        );
        assert_eq!(
            Card::try_new(id(), &cleaned),
            Ok(card),
            "the draft of a card is valid and describes that card"
        );
    }

    #[test]
    fn text_at_a_limit_is_accepted_and_one_character_more_is_not() {
        let mut longest = draft(
            &"q".repeat(MAX_TEXT_LEN),
            &"a".repeat(MAX_ANSWER_LEN),
            &[&"w".repeat(MAX_ANSWER_LEN)],
        );
        longest.explanation = Some("e".repeat(MAX_TEXT_LEN));
        assert!(
            Card::try_new(id(), &longest).is_ok(),
            "every part may be as long as its limit"
        );

        let too_long = |limit| TextProblem::TooLong { limit };
        let mut question = longest.clone();
        question.question.push('q');
        assert_eq!(
            Card::try_new(id(), &question),
            Err(CardProblem::Question(too_long(MAX_TEXT_LEN))),
            "an overlong question is rejected"
        );
        let mut answer = longest.clone();
        answer.answer.push('a');
        assert_eq!(
            Card::try_new(id(), &answer),
            Err(CardProblem::Answer(too_long(MAX_ANSWER_LEN))),
            "an overlong correct answer is rejected"
        );
        let mut wrong = longest.clone();
        wrong.wrong = vec!["w".repeat(MAX_ANSWER_LEN + 1)];
        assert_eq!(
            Card::try_new(id(), &wrong),
            Err(CardProblem::WrongAnswer(too_long(MAX_ANSWER_LEN))),
            "an overlong wrong answer is rejected"
        );
        let mut explanation = longest;
        explanation.explanation = Some("e".repeat(MAX_TEXT_LEN + 1));
        assert_eq!(
            Card::try_new(id(), &explanation),
            Err(CardProblem::Explanation(too_long(MAX_TEXT_LEN))),
            "an overlong explanation is rejected"
        );
    }

    #[test]
    fn a_card_has_one_to_five_wrong_answers() {
        let numbered: Vec<String> = (1..=(MAX_WRONG_ANSWERS + 1))
            .map(|number| number.to_string())
            .collect();
        let wrong_answers = |count: usize| -> Vec<&str> {
            numbered.iter().take(count).map(String::as_str).collect()
        };
        assert_eq!(
            Card::try_new(id(), &draft("q", "a", &wrong_answers(0))),
            Err(CardProblem::WrongAnswerCount(0)),
            "a card without wrong answers is rejected"
        );
        for count in 1..=MAX_WRONG_ANSWERS {
            assert!(
                Card::try_new(id(), &draft("q", "a", &wrong_answers(count))).is_ok(),
                "{count} wrong answers are accepted"
            );
        }
        assert_eq!(
            Card::try_new(
                id(),
                &draft("q", "a", &wrong_answers(MAX_WRONG_ANSWERS + 1))
            ),
            Err(CardProblem::WrongAnswerCount(MAX_WRONG_ANSWERS + 1)),
            "one wrong answer too many is rejected"
        );
    }

    #[test]
    fn the_number_of_wrong_answers_is_judged_before_their_text() {
        let twice: Vec<&str> = vec!["same"; MAX_WRONG_ANSWERS + 1];
        assert_eq!(
            Card::try_new(id(), &draft("q", "a", &twice)),
            Err(CardProblem::WrongAnswerCount(MAX_WRONG_ANSWERS + 1)),
            "too many answers are reported as such, whatever they say"
        );
        let blank_padded = ["b", " ", "", "c", "\t", "d", "", "e", "f", " "];
        let card = Card::try_new(id(), &draft("q", "a", &blank_padded)).expect("valid card");
        assert_eq!(
            card.wrong(),
            ["b", "c", "d", "e", "f"],
            "blank answers do not count"
        );
    }

    #[test]
    fn problems_read_as_clauses() {
        let cases = [
            (
                CardProblem::Question(TextProblem::Empty),
                "the question is empty",
            ),
            (
                CardProblem::Answer(TextProblem::Empty),
                "the correct answer is empty",
            ),
            (
                CardProblem::WrongAnswer(TextProblem::TooLong { limit: 500 }),
                "a wrong answer is longer than 500 characters",
            ),
            (
                CardProblem::Explanation(TextProblem::TooLong { limit: 4000 }),
                "the explanation is longer than 4000 characters",
            ),
            (
                CardProblem::WrongAnswerCount(0),
                "it has 0 wrong answers; a card needs 1 to 5",
            ),
            (
                CardProblem::DuplicateChoice("Mars".to_owned()),
                "the answer \"Mars\" appears twice",
            ),
        ];
        for (problem, message) in cases {
            assert_eq!(problem.to_string(), message, "message of {problem:?}");
        }
    }
}
