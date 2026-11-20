//! Study sets and their topics.

use super::card::{Card, CardDraft};
use super::error::{self, DeckError};
use super::format;
use super::ids::{self, CardId, SetId, TopicId};
use super::text;

/// The longest set title or topic name, in characters.
pub const MAX_NAME_LEN: usize = 120;

/// A named group of cards within a study set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Topic {
    /// The topic's identifier.
    id: TopicId,
    /// The name shown to the learner.
    name: String,
    /// The topic's cards, in file order.
    cards: Vec<Card>,
}

impl Topic {
    /// Returns the topic's identifier.
    #[must_use]
    pub const fn id(&self) -> &TopicId {
        &self.id
    }

    /// Returns the name shown to the learner.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the topic's cards in order.
    #[must_use]
    pub fn cards(&self) -> &[Card] {
        &self.cards
    }
}

/// A titled collection of topics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StudySet {
    /// The set's identifier.
    id: SetId,
    /// The title shown to the learner.
    title: String,
    /// The set's topics, in file order.
    topics: Vec<Topic>,
}

impl StudySet {
    /// Creates a set with no topics.
    ///
    /// # Errors
    ///
    /// Returns [`DeckError::Title`] when `title` is blank or longer than
    /// [`MAX_NAME_LEN`] characters.
    pub fn try_new(id: SetId, title: &str) -> Result<Self, DeckError> {
        let title = text::clean(title, MAX_NAME_LEN).map_err(DeckError::Title)?;
        Ok(Self {
            id,
            title,
            topics: Vec::new(),
        })
    }

    /// Reads a set from the TOML format described in the [module
    /// documentation](super).
    ///
    /// # Examples
    ///
    /// ```
    /// use studycards::deck::{SetId, StudySet};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let text = r#"
    ///     title = "Capitals"
    ///
    ///     [[topics]]
    ///     name = "Europe"
    ///
    ///     [[topics.cards]]
    ///     question = "What is the capital of France?"
    ///     answer = "Paris"
    ///     wrong = ["Lyon", "Marseille"]
    /// "#;
    /// let id: SetId = "capitals".parse()?;
    /// let set = StudySet::from_toml(id, text)?;
    ///
    /// let (topic, card) = set.cards().next().ok_or("the set has a card")?;
    /// assert_eq!(topic.name(), "Europe");
    /// assert_eq!(card.answer(), "Paris");
    /// assert_eq!(card.choices(), ["Paris", "Lyon", "Marseille"]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`DeckError::Format`] when `text` is not TOML or has unexpected
    /// keys, and the matching [`DeckError`] when the content breaks a rule of
    /// the format, such as a duplicate identifier or a card without wrong
    /// answers.
    pub fn from_toml(id: SetId, text: &str) -> Result<Self, DeckError> {
        format::parse(id, text)
    }

    /// Writes the set in the TOML format described in the [module
    /// documentation](super), with every identifier spelled out.
    ///
    /// # Errors
    ///
    /// Returns [`DeckError::Format`] if the set cannot be represented as TOML.
    pub fn to_toml(&self) -> Result<String, DeckError> {
        format::render(self)
    }

    /// Returns the set's identifier.
    #[must_use]
    pub const fn id(&self) -> &SetId {
        &self.id
    }

    /// Returns the title shown to the learner.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the set's topics in order.
    #[must_use]
    pub fn topics(&self) -> &[Topic] {
        &self.topics
    }

    /// Returns the topic with identifier `id`.
    #[must_use]
    pub fn topic(&self, id: &TopicId) -> Option<&Topic> {
        self.topics.iter().find(|topic| topic.id == *id)
    }

    /// Iterates over every card with the topic it belongs to.
    pub fn cards(&self) -> impl Iterator<Item = (&Topic, &Card)> {
        self.topics
            .iter()
            .flat_map(|topic| topic.cards.iter().map(move |card| (topic, card)))
    }

    /// Returns the card with identifier `id` and its topic.
    #[must_use]
    pub fn card(&self, id: &CardId) -> Option<(&Topic, &Card)> {
        self.cards().find(|(_, card)| card.id() == id)
    }

    /// Returns the number of cards in the set.
    #[must_use]
    pub fn card_count(&self) -> usize {
        self.topics.iter().map(|topic| topic.cards.len()).sum()
    }

    /// Changes the title.
    ///
    /// # Errors
    ///
    /// Returns [`DeckError::Title`] when `title` is blank or too long.
    pub fn set_title(&mut self, title: &str) -> Result<(), DeckError> {
        self.title = text::clean(title, MAX_NAME_LEN).map_err(DeckError::Title)?;
        Ok(())
    }

    /// Appends an empty topic and returns its identifier, which is derived
    /// from `name`.
    ///
    /// # Errors
    ///
    /// Returns [`DeckError::TopicName`] when `name` is blank or too long.
    pub fn add_topic(&mut self, name: &str) -> Result<TopicId, DeckError> {
        let name = text::clean(name, MAX_NAME_LEN).map_err(DeckError::TopicName)?;
        let base = ids::slugify(&name, 32).unwrap_or_else(|| ids::hashed("t-", &name));
        let unused = ids::first_free(&base, |candidate| {
            self.topics
                .iter()
                .any(|topic| topic.id.as_str() == candidate)
        });
        let id: TopicId = error::parse_id(unused)?;
        self.push_topic(id.clone(), name, Vec::new());
        Ok(id)
    }

    /// Appends a topic with its identifier and its cards.
    ///
    /// The caller has checked that `name` is usable and that the identifiers
    /// of the topic and of its cards do not occur in the set.
    pub(super) fn push_topic(&mut self, id: TopicId, name: String, cards: Vec<Card>) {
        self.topics.push(Topic { id, name, cards });
    }

    /// Changes a topic's name; its identifier stays the same.
    ///
    /// # Errors
    ///
    /// Returns [`DeckError::UnknownTopic`] when the set has no such topic and
    /// [`DeckError::TopicName`] when `name` is blank or too long.
    pub fn rename_topic(&mut self, id: &TopicId, name: &str) -> Result<(), DeckError> {
        let name = text::clean(name, MAX_NAME_LEN).map_err(DeckError::TopicName)?;
        self.topic_mut(id)?.name = name;
        Ok(())
    }

    /// Removes a topic together with its cards.
    ///
    /// # Errors
    ///
    /// Returns [`DeckError::UnknownTopic`] when the set has no such topic.
    pub fn remove_topic(&mut self, id: &TopicId) -> Result<(), DeckError> {
        let position = self
            .topics
            .iter()
            .position(|topic| topic.id == *id)
            .ok_or_else(|| DeckError::UnknownTopic(id.clone()))?;
        drop(self.topics.remove(position));
        Ok(())
    }

    /// Appends a card to a topic and returns the card's identifier, which is
    /// derived from the question.
    ///
    /// # Errors
    ///
    /// Returns [`DeckError::UnknownTopic`] when the set has no such topic and
    /// [`DeckError::InvalidCard`] when `draft` breaks a rule of the format.
    pub fn add_card(&mut self, topic: &TopicId, draft: &CardDraft) -> Result<CardId, DeckError> {
        let question = draft.question.trim().to_owned();
        let base = ids::slugify(&question, 40).unwrap_or_else(|| ids::hashed("q-", &question));
        let unused = ids::first_free(&base, |candidate| {
            self.cards()
                .any(|(_, card)| card.id().as_str() == candidate)
        });
        let id: CardId = error::parse_id(unused)?;
        let card = Card::try_new(id.clone(), draft)
            .map_err(|problem| DeckError::InvalidCard { question, problem })?;
        self.topic_mut(topic)?.cards.push(card);
        Ok(id)
    }

    /// Replaces a card's content; its identifier and position stay the same.
    ///
    /// # Errors
    ///
    /// Returns [`DeckError::UnknownCard`] when the set has no such card and
    /// [`DeckError::InvalidCard`] when `draft` breaks a rule of the format.
    pub fn update_card(&mut self, id: &CardId, draft: &CardDraft) -> Result<(), DeckError> {
        let question = draft.question.trim().to_owned();
        let replacement = Card::try_new(id.clone(), draft)
            .map_err(|problem| DeckError::InvalidCard { question, problem })?;
        let card = self
            .topics
            .iter_mut()
            .flat_map(|topic| topic.cards.iter_mut())
            .find(|card| card.id() == id)
            .ok_or_else(|| DeckError::UnknownCard(id.clone()))?;
        *card = replacement;
        Ok(())
    }

    /// Removes a card.
    ///
    /// # Errors
    ///
    /// Returns [`DeckError::UnknownCard`] when the set has no such card.
    pub fn remove_card(&mut self, id: &CardId) -> Result<(), DeckError> {
        for topic in &mut self.topics {
            if let Some(position) = topic.cards.iter().position(|card| card.id() == id) {
                drop(topic.cards.remove(position));
                return Ok(());
            }
        }
        Err(DeckError::UnknownCard(id.clone()))
    }

    /// Returns the topic with identifier `id` for modification.
    fn topic_mut(&mut self, id: &TopicId) -> Result<&mut Topic, DeckError> {
        self.topics
            .iter_mut()
            .find(|topic| topic.id == *id)
            .ok_or_else(|| DeckError::UnknownTopic(id.clone()))
    }
}

/// Derives an identifier for a set from the title the set is to have.
///
/// The identifier is a short form of the title; when `is_taken` reports it as
/// in use, a counter is appended.
///
/// # Errors
///
/// Returns [`DeckError::Title`] when `title` is blank or too long.
pub fn set_id_for_title(title: &str, is_taken: impl Fn(&str) -> bool) -> Result<SetId, DeckError> {
    let title = text::clean(title, MAX_NAME_LEN).map_err(DeckError::Title)?;
    let base = ids::slugify(&title, 40).unwrap_or_else(|| ids::hashed("set-", &title));
    error::parse_id(ids::first_free(&base, is_taken))
}

#[cfg(test)]
mod tests {
    use super::{Card, CardDraft, CardId, DeckError, SetId, StudySet, Topic, TopicId};
    use super::{MAX_NAME_LEN, set_id_for_title};
    use crate::deck::{CardProblem, MAX_ANSWER_LEN, MAX_WRONG_ANSWERS, TextProblem};

    fn set_id() -> SetId {
        "demo".parse().expect("valid set id")
    }

    fn draft(question: &str, answer: &str, wrong: &[&str]) -> CardDraft {
        CardDraft {
            question: question.to_owned(),
            answer: answer.to_owned(),
            wrong: wrong.iter().map(|&text| text.to_owned()).collect(),
            explanation: None,
        }
    }

    fn sample() -> (StudySet, TopicId) {
        let mut set = StudySet::try_new(set_id(), "Demo").expect("valid title");
        let topic = set.add_topic("Planets").expect("valid topic");
        set.add_card(
            &topic,
            &draft("Closest to the Sun?", "Mercury", &["Venus", "Mars"]),
        )
        .expect("valid card");
        (set, topic)
    }

    #[test]
    fn new_set_trims_and_checks_the_title() {
        let set = StudySet::try_new(set_id(), "  Demo  ").expect("valid title");
        assert_eq!(set.title(), "Demo", "the title is trimmed");
        assert_eq!(
            StudySet::try_new(set_id(), "   "),
            Err(DeckError::Title(TextProblem::Empty)),
            "a blank title is rejected"
        );
        assert_eq!(
            StudySet::try_new(set_id(), &"x".repeat(MAX_NAME_LEN + 1)),
            Err(DeckError::Title(TextProblem::TooLong {
                limit: MAX_NAME_LEN
            })),
            "an overlong title is rejected"
        );
    }

    #[test]
    fn topics_get_unique_ids_from_their_names() {
        let mut set = StudySet::try_new(set_id(), "Demo").expect("valid title");
        let first = set.add_topic("Inner Planets").expect("valid topic");
        let second = set.add_topic("Inner planets!").expect("valid topic");
        let third = set.add_topic("惑星").expect("valid topic");
        assert_eq!(first.as_str(), "inner-planets", "id is a slug of the name");
        assert_eq!(second.as_str(), "inner-planets-2", "a clash gets a counter");
        assert!(
            third.as_str().starts_with("t-"),
            "a name without ASCII letters is hashed"
        );
        assert_eq!(set.topics().len(), 3, "all topics were added");
        assert_eq!(
            set.topic(&first).map(Topic::name),
            Some("Inner Planets"),
            "lookup by id"
        );
    }

    #[test]
    fn cards_get_unique_ids_from_their_questions() {
        let (mut set, topic) = sample();
        let again = set
            .add_card(&topic, &draft("Closest to the Sun?", "Mercury", &["Earth"]))
            .expect("valid card");
        assert_eq!(
            again.as_str(),
            "closest-to-the-sun-2",
            "a repeated question gets a counter"
        );
        assert_eq!(set.card_count(), 2, "both cards exist");
        let (found_topic, card) = set.card(&again).expect("card exists");
        assert_eq!(found_topic.id(), &topic, "the card is found in its topic");
        assert_eq!(
            card.choices(),
            ["Mercury", "Earth"],
            "the correct answer comes first"
        );
        assert_eq!(card.answer(), "Mercury", "correct answer");
        assert_eq!(card.wrong(), ["Earth"], "wrong answers");
        assert_eq!(
            card.choices().get(Card::CORRECT_CHOICE).map(String::as_str),
            Some("Mercury"),
            "index of the correct choice"
        );
    }

    #[test]
    fn card_text_is_trimmed_and_blank_extras_are_dropped() {
        let (mut set, topic) = sample();
        let id = set
            .add_card(
                &topic,
                &CardDraft {
                    question: "  Largest planet?  ".to_owned(),
                    answer: " Jupiter ".to_owned(),
                    wrong: vec![" Saturn ".to_owned(), "   ".to_owned(), String::new()],
                    explanation: Some("   ".to_owned()),
                },
            )
            .expect("valid card");
        let (_, card) = set.card(&id).expect("card exists");
        assert_eq!(card.question(), "Largest planet?", "question is trimmed");
        assert_eq!(
            card.choices(),
            ["Jupiter", "Saturn"],
            "blank wrong answers are dropped"
        );
        assert_eq!(card.explanation(), None, "a blank explanation is dropped");
    }

    #[test]
    fn invalid_cards_are_rejected_with_the_reason() {
        let (mut set, topic) = sample();
        let cases = [
            (
                draft(" ", "a", &["b"]),
                CardProblem::Question(TextProblem::Empty),
            ),
            (
                draft("q", " ", &["b"]),
                CardProblem::Answer(TextProblem::Empty),
            ),
            (draft("q", "a", &[]), CardProblem::WrongAnswerCount(0)),
            (draft("q", "a", &[" "]), CardProblem::WrongAnswerCount(0)),
            (
                draft("q", "a", &["1", "2", "3", "4", "5", "6"]),
                CardProblem::WrongAnswerCount(MAX_WRONG_ANSWERS + 1),
            ),
            (
                draft("q", "a", &["b", "a"]),
                CardProblem::DuplicateChoice("a".to_owned()),
            ),
            (
                draft("q", "a", &["b", " b "]),
                CardProblem::DuplicateChoice("b".to_owned()),
            ),
            (
                draft("q", &"x".repeat(MAX_ANSWER_LEN + 1), &["b"]),
                CardProblem::Answer(TextProblem::TooLong {
                    limit: MAX_ANSWER_LEN,
                }),
            ),
        ];
        for (draft, problem) in cases {
            let question = draft.question.trim().to_owned();
            assert_eq!(
                set.add_card(&topic, &draft),
                Err(DeckError::InvalidCard {
                    question,
                    problem: problem.clone()
                }),
                "expected rejection: {problem}"
            );
        }
        assert_eq!(set.card_count(), 1, "no invalid card was added");
    }

    #[test]
    fn update_keeps_identity_and_position() {
        let (mut set, topic) = sample();
        let second = set
            .add_card(&topic, &draft("Red planet?", "Mars", &["Venus"]))
            .expect("valid");
        let first = set.topics()[0].cards()[0].id().clone();
        let mut changed = draft("Which planet is closest to the Sun?", "Mercury", &["Venus"]);
        changed.explanation = Some("It orbits at 0.39 AU.".to_owned());
        set.update_card(&first, &changed).expect("valid update");
        let cards = set.topics()[0].cards();
        assert_eq!(cards[0].id(), &first, "the id survives a reworded question");
        assert_eq!(cards[0].to_draft(), changed, "the content was replaced");
        assert_eq!(cards[1].id(), &second, "other cards keep their place");
        assert_eq!(
            set.update_card(&first, &draft("q", "a", &[])),
            Err(DeckError::InvalidCard {
                question: "q".to_owned(),
                problem: CardProblem::WrongAnswerCount(0)
            }),
            "an invalid update is rejected"
        );
        assert_eq!(
            set.topics()[0].cards()[0].to_draft(),
            changed,
            "a rejected update changes nothing"
        );
    }

    #[test]
    fn removal_and_unknown_identifiers() {
        let (mut set, topic) = sample();
        let card = set.topics()[0].cards()[0].id().clone();
        let missing_topic: TopicId = "nope".parse().expect("valid id");
        let missing_card: CardId = "nope".parse().expect("valid id");
        assert_eq!(
            set.add_card(&missing_topic, &draft("q", "a", &["b"])),
            Err(DeckError::UnknownTopic(missing_topic.clone())),
            "adding to a missing topic fails"
        );
        assert_eq!(
            set.rename_topic(&missing_topic, "x"),
            Err(DeckError::UnknownTopic(missing_topic.clone())),
            "renaming a missing topic fails"
        );
        assert_eq!(
            set.remove_card(&missing_card),
            Err(DeckError::UnknownCard(missing_card.clone())),
            "removing a missing card fails"
        );
        assert_eq!(
            set.update_card(&missing_card, &draft("q", "a", &["b"])),
            Err(DeckError::UnknownCard(missing_card)),
            "updating a missing card fails"
        );
        set.rename_topic(&topic, "Worlds").expect("topic exists");
        assert_eq!(set.topics()[0].name(), "Worlds", "the topic was renamed");
        assert_eq!(set.topics()[0].id(), &topic, "renaming keeps the id");
        set.remove_card(&card).expect("card exists");
        assert_eq!(set.card_count(), 0, "the card was removed");
        set.remove_topic(&topic).expect("topic exists");
        assert!(set.topics().is_empty(), "the topic was removed");
        assert_eq!(
            set.remove_topic(&topic),
            Err(DeckError::UnknownTopic(topic)),
            "removing twice fails"
        );
    }

    #[test]
    fn title_can_be_changed() {
        let (mut set, _) = sample();
        set.set_title(" Renamed ").expect("valid title");
        assert_eq!(
            set.title(),
            "Renamed",
            "the new title is trimmed and stored"
        );
        assert_eq!(
            set.set_title(""),
            Err(DeckError::Title(TextProblem::Empty)),
            "a blank title is rejected"
        );
        assert_eq!(set.title(), "Renamed", "a rejected title changes nothing");
    }

    #[test]
    fn set_ids_come_from_titles() {
        let taken = ["solar-system"];
        let free = set_id_for_title(" Solar System! ", |_| false).expect("valid title");
        assert_eq!(
            free.as_str(),
            "solar-system",
            "the id is a short form of the title"
        );
        let second =
            set_id_for_title("Solar system", |id| taken.contains(&id)).expect("valid title");
        assert_eq!(
            second.as_str(),
            "solar-system-2",
            "a taken id gets a counter"
        );
        let hashed = set_id_for_title("日本語", |_| false).expect("valid title");
        assert!(
            hashed.as_str().starts_with("set-"),
            "a title without ASCII letters is hashed"
        );
        assert_eq!(
            set_id_for_title("  ", |_| false),
            Err(DeckError::Title(TextProblem::Empty)),
            "a blank title is rejected"
        );
    }
}
