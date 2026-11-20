//! Reading and writing the TOML form of a study set.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::card::{Card, CardDraft};
use super::error::{self, DeckError};
use super::ids::{self, CardId, SetId, TopicId};
use super::study_set::{MAX_NAME_LEN, StudySet};
use super::text;

/// The top-level table of a set file.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SetFile {
    /// The set title.
    title: String,
    /// The `[[topics]]` tables.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    topics: Vec<TopicFile>,
}

/// One `[[topics]]` table.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TopicFile {
    /// The topic identifier; derived from the name when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    /// The topic name.
    name: String,
    /// The `[[topics.cards]]` tables.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    cards: Vec<CardFile>,
}

/// One `[[topics.cards]]` table.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CardFile {
    /// The card identifier; derived from the question when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    /// The question text.
    question: String,
    /// The correct answer.
    answer: String,
    /// The wrong answers.
    wrong: Vec<String>,
    /// Text shown after answering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    explanation: Option<String>,
}

/// Reads and validates a set file.
pub(super) fn parse(id: SetId, text: &str) -> Result<StudySet, DeckError> {
    let file: SetFile =
        toml::from_str(text).map_err(|error| DeckError::Format(error.to_string()))?;
    let mut set = StudySet::try_new(id, &file.title)?;
    let mut topic_ids = BTreeSet::new();
    let mut card_ids = BTreeSet::new();
    for topic_file in file.topics {
        let name = text::clean(&topic_file.name, MAX_NAME_LEN).map_err(DeckError::TopicName)?;
        let topic_id: TopicId = error::parse_id(topic_file.id.unwrap_or_else(|| {
            ids::slugify(&name, 32).unwrap_or_else(|| ids::hashed("t-", &name))
        }))?;
        if !topic_ids.insert(topic_id.clone()) {
            return Err(DeckError::DuplicateTopic(topic_id));
        }
        let mut cards = Vec::with_capacity(topic_file.cards.len());
        for card_file in topic_file.cards {
            let question = card_file.question.trim().to_owned();
            let card_id: CardId =
                error::parse_id(card_file.id.unwrap_or_else(|| ids::hashed("q-", &question)))?;
            if !card_ids.insert(card_id.clone()) {
                return Err(DeckError::DuplicateCard(card_id));
            }
            let draft = CardDraft {
                question: card_file.question,
                answer: card_file.answer,
                wrong: card_file.wrong,
                explanation: card_file.explanation,
            };
            let card = Card::try_new(card_id, &draft)
                .map_err(|problem| DeckError::InvalidCard { question, problem })?;
            cards.push(card);
        }
        set.push_topic(topic_id, name, cards);
    }
    Ok(set)
}

/// Writes a set file with explicit identifiers.
pub(super) fn render(set: &StudySet) -> Result<String, DeckError> {
    let file = SetFile {
        title: set.title().to_owned(),
        topics: set
            .topics()
            .iter()
            .map(|topic| TopicFile {
                id: Some(topic.id().as_str().to_owned()),
                name: topic.name().to_owned(),
                cards: topic
                    .cards()
                    .iter()
                    .map(|card| CardFile {
                        id: Some(card.id().as_str().to_owned()),
                        question: card.question().to_owned(),
                        answer: card.answer().to_owned(),
                        wrong: card.wrong().to_vec(),
                        explanation: card.explanation().map(str::to_owned),
                    })
                    .collect(),
            })
            .collect(),
    };
    toml::to_string_pretty(&file).map_err(|error| DeckError::Format(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{CardDraft, DeckError, MAX_NAME_LEN, SetId, StudySet};
    use crate::deck::{CardProblem, IdProblem, TextProblem};
    use crate::deck::{MAX_ANSWER_LEN, MAX_ID_LEN, MAX_TEXT_LEN, MAX_WRONG_ANSWERS};

    const SAMPLE: &str = r#"
title = "Solar system"

[[topics]]
name = "Planets"

[[topics.cards]]
question = "Which planet is closest to the Sun?"
answer = "Mercury"
wrong = ["Venus", "Earth", "Mars"]
explanation = "Mercury orbits the Sun at about 0.39 AU."

[[topics.cards]]
id = "red-planet"
question = "Which planet is called the Red Planet?"
answer = "Mars"
wrong = ["Jupiter"]

[[topics]]
id = "moons"
name = "Moons and rings"
"#;

    fn id() -> SetId {
        "solar-system".parse().expect("valid set id")
    }

    fn parse(text: &str) -> Result<StudySet, DeckError> {
        StudySet::from_toml(id(), text)
    }

    #[test]
    fn reads_the_documented_format() {
        let set = parse(SAMPLE).expect("the sample is valid");
        assert_eq!(set.id(), &id(), "the set keeps the id it was loaded under");
        assert_eq!(set.title(), "Solar system", "title");
        let topics = set.topics();
        assert_eq!(topics.len(), 2, "two topics");
        assert_eq!(
            topics[0].id().as_str(),
            "planets",
            "a topic id defaults to a slug of its name"
        );
        assert_eq!(
            topics[1].id().as_str(),
            "moons",
            "an explicit topic id is kept"
        );
        assert_eq!(topics[1].name(), "Moons and rings", "topic name");
        assert!(topics[1].cards().is_empty(), "a topic may have no cards");
        let cards = topics[0].cards();
        assert_eq!(cards.len(), 2, "two cards");
        assert_eq!(
            cards[0].id().as_str(),
            "q-ada18dd8c0d7dcf2",
            "a card id defaults to a hash of its question"
        );
        assert_eq!(
            cards[0].choices(),
            ["Mercury", "Venus", "Earth", "Mars"],
            "choices"
        );
        assert_eq!(
            cards[0].explanation(),
            Some("Mercury orbits the Sun at about 0.39 AU."),
            "explanation"
        );
        assert_eq!(
            cards[1].id().as_str(),
            "red-planet",
            "an explicit card id is kept"
        );
        assert_eq!(cards[1].explanation(), None, "the explanation is optional");
    }

    #[test]
    fn written_files_read_back_identically() {
        let set = parse(SAMPLE).expect("the sample is valid");
        let text = set.to_toml().expect("a valid set can be written");
        assert!(
            text.contains("id = \"planets\""),
            "derived topic ids are written out:\n{text}"
        );
        assert!(
            text.contains("id = \"q-ada18dd8c0d7dcf2\""),
            "derived card ids are written out:\n{text}"
        );
        assert_eq!(
            parse(&text),
            Ok(set),
            "reading the written file gives the same set"
        );
    }

    #[test]
    fn multi_line_text_survives_a_round_trip() {
        let mut set = StudySet::try_new(id(), "Quotes \"and\" \\ slashes").expect("valid title");
        let topic = set.add_topic("Misc").expect("valid topic");
        let draft = CardDraft {
            question: "Line one\nLine two with \"quotes\" and 'apostrophes'".to_owned(),
            answer: "naïve café — ok".to_owned(),
            wrong: vec!["tab\there".to_owned()],
            explanation: Some("First.\n\nSecond.".to_owned()),
        };
        set.add_card(&topic, &draft).expect("valid card");
        let text = set.to_toml().expect("a valid set can be written");
        assert_eq!(parse(&text), Ok(set), "special characters survive:\n{text}");
    }

    #[test]
    fn the_limits_are_the_ones_the_format_documents() {
        assert_eq!(MAX_NAME_LEN, 120, "characters of a title or topic name");
        assert_eq!(
            MAX_TEXT_LEN, 4000,
            "characters of a question or explanation"
        );
        assert_eq!(MAX_ANSWER_LEN, 500, "characters of an answer");
        assert_eq!(MAX_WRONG_ANSWERS, 5, "wrong answers of a card");
        assert_eq!(MAX_ID_LEN, 64, "bytes of an identifier");
    }

    #[test]
    fn an_empty_set_needs_only_a_title() {
        let set = parse("title = \"Empty\"").expect("a title alone is a valid set");
        assert!(set.topics().is_empty(), "no topics");
        assert_eq!(
            set.to_toml().as_deref(),
            Ok("title = \"Empty\"\n"),
            "written form of an empty set"
        );
    }

    #[test]
    fn rejects_text_that_is_not_a_set_file() {
        for text in [
            "",
            "title = 3",
            "title = \"x\"\nextra = 1",
            "title = \"x\"\n[[topics]]\nname = \"t\"\ncolour = \"red\"",
            "title = \"x\"\n[[topics]]\nname = \"t\"\n[[topics.cards]]\nquestion = \"q\"\n\
             answer = \"a\"\nwrong = [\"b\"]\nhint = \"h\"",
            "not toml at all [",
        ] {
            assert!(
                matches!(parse(text), Err(DeckError::Format(_))),
                "{text:?} is not a set file"
            );
        }
        let error =
            parse("title = \"x\"\n[[topics]]\nname = \"t\"\n[[topics.cards]]\nquestion = \"q\"\n")
                .expect_err("a card without an answer is rejected");
        assert!(
            error.to_string().contains("answer"),
            "the message names the missing key: {error}"
        );
    }

    #[test]
    fn rejects_content_that_breaks_the_rules() {
        let cases: [(&str, DeckError); 6] = [
            ("title = \" \"", DeckError::Title(TextProblem::Empty)),
            (
                "title = \"x\"\n[[topics]]\nname = \"\"",
                DeckError::TopicName(TextProblem::Empty),
            ),
            (
                "title = \"x\"\n[[topics]]\nid = \"Bad Id\"\nname = \"t\"",
                DeckError::InvalidId {
                    text: "Bad Id".to_owned(),
                    problem: IdProblem::BadStart,
                },
            ),
            (
                "title = \"x\"\n[[topics]]\nname = \"Same\"\n[[topics]]\nname = \"same\"",
                DeckError::DuplicateTopic("same".parse().expect("valid id")),
            ),
            (
                "title = \"x\"\n[[topics]]\nname = \"t\"\n\
                 [[topics.cards]]\nid = \"c\"\nquestion = \"q1\"\nanswer = \"a\"\nwrong = [\"b\"]\n\
                 [[topics.cards]]\nid = \"c\"\nquestion = \"q2\"\nanswer = \"a\"\nwrong = [\"b\"]",
                DeckError::DuplicateCard("c".parse().expect("valid id")),
            ),
            (
                "title = \"x\"\n[[topics]]\nname = \"t\"\n\
                 [[topics.cards]]\nquestion = \"q\"\nanswer = \"a\"\nwrong = []",
                DeckError::InvalidCard {
                    question: "q".to_owned(),
                    problem: CardProblem::WrongAnswerCount(0),
                },
            ),
        ];
        for (text, error) in cases {
            assert_eq!(
                parse(text),
                Err(error.clone()),
                "expected rejection: {error}"
            );
        }
    }

    #[test]
    fn identical_questions_without_ids_collide() {
        let text = "title = \"x\"\n[[topics]]\nname = \"t\"\n\
                    [[topics.cards]]\nquestion = \"q\"\nanswer = \"a\"\nwrong = [\"b\"]\n\
                    [[topics.cards]]\nquestion = \" q \"\nanswer = \"c\"\nwrong = [\"d\"]";
        assert!(
            matches!(parse(text), Err(DeckError::DuplicateCard(_))),
            "cards with the same question need explicit ids"
        );
    }
}
