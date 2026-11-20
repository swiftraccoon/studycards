//! Study sets: topics and the cards in them.
//!
//! A study set is a titled list of topics, and a topic is a named list of
//! multiple-choice cards. A set is stored as a TOML file:
//!
//! ```toml
//! title = "Solar system"
//!
//! [[topics]]
//! name = "Planets"
//!
//! [[topics.cards]]
//! question = "Which planet is closest to the Sun?"
//! answer = "Mercury"
//! wrong = ["Venus", "Earth", "Mars"]
//! explanation = "Mercury orbits the Sun at about 0.39 AU."
//! ```
//!
//! - `title` and the `name` of a topic are required and have at most 120
//!   characters ([`MAX_NAME_LEN`]). A set may have no topics, and a topic no
//!   cards.
//! - A card has a `question`, one correct `answer`, and one to five `wrong`
//!   answers ([`MAX_WRONG_ANSWERS`]), which differ from the correct one and
//!   from each other. A question has at most 4000 characters
//!   ([`MAX_TEXT_LEN`]) and an answer at most 500 ([`MAX_ANSWER_LEN`]).
//! - `explanation` is optional text of at most 4000 characters, shown after
//!   the card has been answered.
//! - Topics and cards may carry an `id` key, written as [`TopicId`] and
//!   [`CardId`] describe. A topic without one is identified by a short form of
//!   its name, and a card without one by a hash of its question, so rewording
//!   such a question makes it a different card. No two topics of a set share
//!   an identifier, and no two cards do; progress is recorded per card
//!   identifier.
//! - Whitespace around a text is ignored. Blank `wrong` answers and a blank
//!   `explanation` count as absent; any other blank text is an error.
//! - A key that is not named here is an error.
//!
//! [`StudySet::from_toml`] reads this format and [`StudySet::to_toml`] writes
//! it with every identifier spelled out.

pub use self::card::{Card, CardDraft, CardProblem};
pub use self::card::{MAX_ANSWER_LEN, MAX_TEXT_LEN, MAX_WRONG_ANSWERS};
pub use self::error::DeckError;
pub use self::ids::{CardId, IdProblem, MAX_ID_LEN, SetId, TopicId};
pub use self::study_set::{MAX_NAME_LEN, StudySet, Topic, set_id_for_title};
pub use self::text::TextProblem;

mod card;
mod error;
mod format;
mod ids;
mod study_set;
mod text;
