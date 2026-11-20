//! Studying a set: the queue of cards, answering, and progress.
//!
//! [`Progress`] holds everything remembered about one study set: the answer
//! [`History`] of each card and, for each [`Scope`] that has been studied, a
//! queue of up to two batches. The front batch is the one being answered; the
//! second is chosen before the first is completed, so a missed card returns no
//! sooner than in the batch after the next.
//!
//! A study session alternates between two states, reported by
//! [`Progress::view`]: a question waiting for an answer, and the answered card
//! shown until the learner moves on with [`Progress::advance`].
//!
//! A card is answered once each time it is asked: [`Progress::answer`] judges
//! the answer and adds it to the history of the card. A wrong answer leaves the
//! correct one hidden during the review. The learner can then choose again with
//! [`Progress::retry`] until the correct answer is chosen, or decline to choose
//! and have it shown. A retry is not an answer: it changes neither a history
//! nor a batch.
//!
//! # Stored form
//!
//! [`Progress`] is stored as JSON:
//!
//! ```json
//! {
//!   "cards": {
//!     "closest-to-sun": ["correct", "dont_know"],
//!     "rings": ["wrong"]
//!   },
//!   "all": {
//!     "open": [
//!       {
//!         "slots": [
//!           { "card": "rings", "order": [2, 0, 3, 1], "outcome": "wrong" },
//!           { "card": "titan", "order": [1, 3, 0, 2], "outcome": null }
//!         ]
//!       }
//!     ],
//!     "previous_misses": 0,
//!     "review": {
//!       "card": "rings",
//!       "order": [2, 0, 3, 1],
//!       "choice": { "answer": 3 },
//!       "outcome": "wrong",
//!       "retries": [{ "answer": 1 }, "dont_know"]
//!     }
//!   },
//!   "topics": {
//!     "planets": { "open": [], "previous_misses": 0, "review": null }
//!   }
//! }
//! ```
//!
//! - `cards` maps a card identifier to the answers given to that card, newest
//!   first. An answer is `"correct"`, `"wrong"`, or `"dont_know"`. The ten
//!   newest answers are read.
//! - `all` is the queue for the whole set, and `topics` maps a topic
//!   identifier to the queue for that topic.
//! - `open` lists the unfinished batches of a queue, the one being answered
//!   first. A slot names a card, the `order` in which the card's answers are
//!   shown, and the `outcome` of the card, which is `null` until the card is
//!   answered. `order` lists positions in [`Card::choices`], where position
//!   0 is the correct answer.
//! - `previous_misses` is the number of misses in the batch of the queue that
//!   was completed most recently.
//! - `review` is the card that was answered last, until the learner moves on,
//!   and `null` otherwise. Its `choice` is the answer, `"dont_know"` or
//!   `{ "answer": N }`, where `N` is a position in [`Card::choices`], and its
//!   `outcome` is how the answer was judged. `retries` lists what the learner
//!   chose after a wrong answer, oldest first, each in the form of `choice`.
//!   The review shows the correct answer once `choice` or `retries` holds
//!   position 0 or `"dont_know"`.
//!
//! A key of the top level or of a queue may be absent; it then stands for no
//! answers, no batches, zero, or `null`. The `retries` of a review may be
//! absent as well and then stand for none. Unknown keys are ignored.
//!
//! # Examples
//!
//! ```
//! use studycards::deck::{Card, StudySet};
//! use studycards::mastery::Outcome;
//! use studycards::study::{Choice, Progress, Scope, View};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let set = StudySet::from_toml(
//!     "capitals".parse()?,
//!     r#"
//!         title = "Capitals"
//!
//!         [[topics]]
//!         name = "Europe"
//!
//!         [[topics.cards]]
//!         question = "What is the capital of France?"
//!         answer = "Paris"
//!         wrong = ["Lyon", "Marseille"]
//!     "#,
//! )?;
//! let mut random = fastrand::Rng::with_seed(1);
//! let mut progress = Progress::new();
//!
//! progress.prepare(&set, &Scope::All, &mut random)?;
//! let View::Question { card, .. } = progress.view(&set, &Scope::All) else {
//!     return Err("a question is waiting".into());
//! };
//! let card = card.id().clone();
//!
//! let choice = Choice::Answer(Card::CORRECT_CHOICE);
//! let outcome =
//!     progress.answer(&set, &Scope::All, &card, choice, &mut random)?;
//! assert_eq!(outcome, Outcome::Correct);
//! assert_eq!(progress.history(&card), [Outcome::Correct]);
//!
//! // The answer stays on screen until the learner moves on.
//! assert!(matches!(progress.view(&set, &Scope::All), View::Answered { .. }));
//! progress.advance(&Scope::All);
//! assert!(matches!(progress.view(&set, &Scope::All), View::Question { .. }));
//! # Ok(())
//! # }
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::iter::Sum;

use serde::{Deserialize, Serialize};

use crate::deck::{Card, CardId, StudySet, Topic, TopicId};
use crate::mastery::{Aptitude, History, Outcome};
use crate::picker::{self, Candidate, RandomSource};

/// The number of batches kept chosen ahead of the learner.
const QUEUED_BATCHES: usize = 2;

/// The cards a study session draws from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Scope {
    /// Every card of the set.
    All,
    /// The cards of one topic.
    Topic(TopicId),
}

/// What the learner chooses for a card, as the answer to it or in a retry.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    /// The answer at this position in [`Card::choices`].
    Answer(usize),
    /// The learner declined to guess.
    DontKnow,
}

/// Why an answer or a retry was not accepted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StudyError {
    /// The set has no topic with this identifier.
    UnknownTopic(TopicId),
    /// No question is waiting: the scope has no cards, or the last answer is
    /// still being reviewed.
    NoQuestion,
    /// No card is waiting for a retry: no answer is being reviewed, or the
    /// review shows the correct answer.
    NoRetry,
    /// The choice is for a card other than the one being asked or reviewed.
    NotCurrent(CardId),
    /// The chosen answer does not exist on the card.
    NoSuchChoice(usize),
}

impl Display for StudyError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTopic(id) => write!(f, "there is no topic \"{id}\""),
            Self::NoQuestion => f.write_str("no question is waiting for an answer"),
            Self::NoRetry => f.write_str("no card is waiting for another choice"),
            Self::NotCurrent(id) => write!(f, "card \"{id}\" is not the current card"),
            Self::NoSuchChoice(index) => write!(f, "the card has no answer number {index}"),
        }
    }
}

impl Error for StudyError {}

/// One card of a batch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Slot {
    /// The card to ask.
    card: CardId,
    /// Positions in [`Card::choices`], in the order the answers are shown.
    order: Vec<usize>,
    /// The result, once answered.
    outcome: Option<Outcome>,
}

/// A group of cards chosen together.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
struct Batch {
    /// The cards in the order they are asked.
    slots: Vec<Slot>,
}

impl Batch {
    /// Returns the first card that has not been answered.
    fn current(&self) -> Option<&Slot> {
        self.slots.iter().find(|slot| slot.outcome.is_none())
    }

    /// Returns the number of answered cards that were missed.
    fn misses(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.outcome.is_some_and(Outcome::is_miss))
            .count()
    }
}

/// The card answered most recently, kept until the learner moves on.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Answered {
    /// The answered card.
    card: CardId,
    /// The order the answers were shown in.
    order: Vec<usize>,
    /// The answer: what the learner chose when the card was asked.
    choice: Choice,
    /// How the answer was judged.
    outcome: Outcome,
    /// What the learner chose after a wrong answer, oldest first.
    #[serde(default)]
    retries: Vec<Choice>,
}

impl Answered {
    /// Returns the review of the card, or `None` when `set` does not have the
    /// card.
    fn review<'session>(&'session self, set: &'session StudySet) -> Option<Review<'session>> {
        let (topic, card) = set.card(&self.card)?;
        Some(Review {
            topic,
            card,
            order: &self.order,
            choice: self.choice,
            outcome: self.outcome,
            retries: &self.retries,
        })
    }
}

/// The batches chosen for one scope.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
struct Queue {
    /// The unfinished batches, the one being answered first.
    #[serde(default)]
    open: Vec<Batch>,
    /// The number of misses in the batch completed most recently.
    #[serde(default)]
    previous_misses: usize,
    /// The card being reviewed after an answer.
    #[serde(default)]
    review: Option<Answered>,
}

/// What a study session shows.
///
/// A view borrows from the study set and the progress of its session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum View<'session> {
    /// A question is waiting for an answer.
    Question {
        /// The topic of the card.
        topic: &'session Topic,
        /// The card being asked.
        card: &'session Card,
        /// Positions in [`Card::choices`], in the order to show the answers.
        order: &'session [usize],
    },
    /// The last answer is being reviewed.
    Answered(Review<'session>),
    /// The scope has no cards.
    Empty,
}

/// A card under review: the answer given to it, and what the learner chose
/// after a wrong answer.
///
/// The review shows the correct answer once the learner has chosen it or has
/// declined to guess. Until then, [`Progress::retry`] accepts another choice;
/// its example follows a review from a wrong answer to the correct one.
///
/// A review borrows from the study set and the progress of its session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Review<'session> {
    /// The topic of the card.
    pub topic: &'session Topic,
    /// The card that was answered.
    pub card: &'session Card,
    /// The order the answers were shown in.
    pub order: &'session [usize],
    /// The answer: what the learner chose when the card was asked.
    pub choice: Choice,
    /// How the answer was judged.
    pub outcome: Outcome,
    /// What the learner chose after a wrong answer, oldest first.
    pub retries: &'session [Choice],
}

impl Review<'_> {
    /// Returns `true` when the learner chose the answer at `index` in
    /// [`Card::choices`], as the answer to the card or in a retry.
    #[must_use]
    pub fn chose(&self, index: usize) -> bool {
        let choice = Choice::Answer(index);
        self.choice == choice || self.retries.contains(&choice)
    }

    /// Returns `true` once the review shows the correct answer: the learner
    /// chose it or declined to guess, as the answer to the card or in a retry.
    #[must_use]
    pub fn shows_correct_answer(&self) -> bool {
        self.chose(Card::CORRECT_CHOICE)
            || self.choice == Choice::DontKnow
            || self.retries.contains(&Choice::DontKnow)
    }
}

/// Seen and aptitude totals for a group of cards.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Tally {
    /// The number of cards in the group.
    pub cards: usize,
    /// How many of them have been attempted.
    pub seen: usize,
    /// The sum of the [`Aptitude`] points of the attempted cards.
    pub aptitude_points: usize,
}

impl Tally {
    /// Returns the share of cards attempted, as a rounded percentage.
    #[must_use]
    pub fn seen_percent(&self) -> usize {
        rounded_ratio(self.seen.saturating_mul(100), self.cards)
    }

    /// Returns the aptitude over all cards of the group, as a rounded
    /// percentage. Unattempted cards count as zero, so the value cannot exceed
    /// [`Tally::seen_percent`].
    #[must_use]
    pub fn aptitude_percent(&self) -> usize {
        let full = self.cards.saturating_mul(usize::from(Aptitude::FULL));
        rounded_ratio(self.aptitude_points.saturating_mul(100), full)
    }
}

impl Sum for Tally {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::default(), |total, tally| Self {
            cards: total.cards.saturating_add(tally.cards),
            seen: total.seen.saturating_add(tally.seen),
            aptitude_points: total.aptitude_points.saturating_add(tally.aptitude_points),
        })
    }
}

/// Divides, rounding half up; zero when the divisor is zero.
fn rounded_ratio(numerator: usize, denominator: usize) -> usize {
    numerator
        .saturating_mul(2)
        .saturating_add(denominator)
        .checked_div(denominator.saturating_mul(2))
        .unwrap_or(0)
}

/// Everything remembered about studying one set.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Progress {
    /// The answers given to each card.
    #[serde(default)]
    cards: BTreeMap<CardId, History>,
    /// The queue for studying the whole set.
    #[serde(default)]
    all: Queue,
    /// The queues for studying single topics.
    #[serde(default)]
    topics: BTreeMap<TopicId, Queue>,
}

impl Progress {
    /// Creates the progress of a set that has not been studied.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the answers given to a card, newest first.
    #[must_use]
    pub fn history(&self, card: &CardId) -> &[Outcome] {
        self.cards.get(card).map_or(&[], History::outcomes)
    }

    /// Totals what has been seen and learned of `cards`.
    #[must_use]
    pub fn tally(&self, cards: &[Card]) -> Tally {
        let mut tally = Tally {
            cards: cards.len(),
            ..Tally::default()
        };
        let aptitudes = cards
            .iter()
            .filter_map(|card| self.cards.get(card.id())?.aptitude());
        for aptitude in aptitudes {
            tally.seen = tally.seen.saturating_add(1);
            tally.aptitude_points = tally
                .aptitude_points
                .saturating_add(usize::from(aptitude.points()));
        }
        tally
    }

    /// Brings the queue of `scope` in line with `set` and chooses batches
    /// until two are waiting or no free card is left.
    ///
    /// A card that was removed from the set, or whose number of answers
    /// changed, leaves the queue. Call this before [`Progress::view`] whenever
    /// the set may have changed.
    ///
    /// # Errors
    ///
    /// Returns [`StudyError::UnknownTopic`] when `scope` names a topic the set
    /// does not have.
    pub fn prepare(
        &mut self,
        set: &StudySet,
        scope: &Scope,
        random: &mut dyn RandomSource,
    ) -> Result<(), StudyError> {
        let cards = scope_cards(set, scope)?;
        self.reconcile(set, scope, &cards);
        self.refill(scope, &cards, random);
        Ok(())
    }

    /// Returns what the session of `scope` shows.
    ///
    /// Call [`Progress::prepare`] first; a queue that refers to cards the set
    /// no longer has is reported as [`View::Empty`].
    #[must_use]
    pub fn view<'session>(
        &'session self,
        set: &'session StudySet,
        scope: &Scope,
    ) -> View<'session> {
        let Some(queue) = self.queue(scope) else {
            return View::Empty;
        };
        if let Some(answered) = &queue.review {
            return answered.review(set).map_or(View::Empty, View::Answered);
        }
        queue
            .open
            .first()
            .and_then(Batch::current)
            .and_then(|slot| {
                let (topic, card) = set.card(&slot.card)?;
                Some(View::Question {
                    topic,
                    card,
                    order: &slot.order,
                })
            })
            .unwrap_or(View::Empty)
    }

    /// Records the learner's answer to the card being asked and returns how it
    /// was judged.
    ///
    /// The answered card is then under review until [`Progress::advance`].
    /// During the review of a wrong answer, [`Progress::retry`] lets the
    /// learner choose again. Completing a batch chooses the next one.
    ///
    /// # Errors
    ///
    /// Returns [`StudyError::UnknownTopic`] for a scope the set does not have,
    /// [`StudyError::NoQuestion`] when no question is waiting,
    /// [`StudyError::NotCurrent`] when `card` is not the card being asked, and
    /// [`StudyError::NoSuchChoice`] when `choice` is not an answer of the card.
    pub fn answer(
        &mut self,
        set: &StudySet,
        scope: &Scope,
        card: &CardId,
        choice: Choice,
        random: &mut dyn RandomSource,
    ) -> Result<Outcome, StudyError> {
        let cards = scope_cards(set, scope)?;
        let queue = self.queue_mut(scope);
        if queue.review.is_some() {
            return Err(StudyError::NoQuestion);
        }
        let batch = queue.open.first_mut().ok_or(StudyError::NoQuestion)?;
        let slot = batch
            .slots
            .iter_mut()
            .find(|slot| slot.outcome.is_none())
            .ok_or(StudyError::NoQuestion)?;
        if slot.card != *card {
            return Err(StudyError::NotCurrent(card.clone()));
        }
        let outcome = match choice {
            Choice::DontKnow => Outcome::DontKnow,
            Choice::Answer(index) if index == Card::CORRECT_CHOICE => Outcome::Correct,
            Choice::Answer(index) if slot.order.contains(&index) => Outcome::Wrong,
            Choice::Answer(index) => return Err(StudyError::NoSuchChoice(index)),
        };
        slot.outcome = Some(outcome);
        queue.review = Some(Answered {
            card: card.clone(),
            order: slot.order.clone(),
            choice,
            outcome,
            retries: Vec::new(),
        });
        if batch.current().is_none() {
            queue.previous_misses = batch.misses();
            drop(queue.open.remove(0));
        }
        self.cards.entry(card.clone()).or_default().record(outcome);
        self.refill(scope, &cards, random);
        Ok(outcome)
    }

    /// Records what the learner chose after a wrong answer to the card under
    /// review.
    ///
    /// A wrong choice leaves the correct answer hidden, so that the learner can
    /// choose again. The correct choice, or [`Choice::DontKnow`], makes the
    /// review show the correct answer, which ends the retries. Choosing an
    /// answer a second time changes nothing.
    ///
    /// A retry is not an answer: the history of the card, and with it the
    /// card's level and aptitude, stays as [`Progress::answer`] left it, and so
    /// do the batches.
    ///
    /// # Errors
    ///
    /// Returns [`StudyError::UnknownTopic`] for a scope the set does not have,
    /// [`StudyError::NoRetry`] when no card of the set is under review or the
    /// review shows the correct answer, [`StudyError::NotCurrent`] when `card` is
    /// not the card under review, and [`StudyError::NoSuchChoice`] when
    /// `choice` is not an answer of the card.
    ///
    /// # Examples
    ///
    /// ```
    /// use studycards::deck::{Card, StudySet};
    /// use studycards::mastery::Outcome;
    /// use studycards::study::{Choice, Progress, Scope, View};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let set = StudySet::from_toml(
    ///     "capitals".parse()?,
    ///     r#"
    ///         title = "Capitals"
    ///
    ///         [[topics]]
    ///         name = "Europe"
    ///
    ///         [[topics.cards]]
    ///         question = "What is the capital of France?"
    ///         answer = "Paris"
    ///         wrong = ["Lyon", "Marseille"]
    ///     "#,
    /// )?;
    /// let scope = Scope::All;
    /// let mut random = fastrand::Rng::with_seed(1);
    /// let mut progress = Progress::new();
    /// progress.prepare(&set, &scope, &mut random)?;
    /// let View::Question { card, .. } = progress.view(&set, &scope) else {
    ///     return Err("a question is waiting".into());
    /// };
    /// let card = card.id().clone();
    ///
    /// // The choices of the card are "Paris" at position 0, then "Lyon" and
    /// // "Marseille".
    /// let lyon = Choice::Answer(1);
    /// let outcome = progress.answer(&set, &scope, &card, lyon, &mut random)?;
    /// assert_eq!(outcome, Outcome::Wrong);
    ///
    /// progress.retry(&set, &scope, &card, Choice::Answer(2))?;
    /// let View::Answered(review) = progress.view(&set, &scope) else {
    ///     return Err("the card is under review".into());
    /// };
    /// assert!(review.chose(1) && review.chose(2));
    /// assert!(!review.shows_correct_answer());
    ///
    /// let paris = Choice::Answer(Card::CORRECT_CHOICE);
    /// progress.retry(&set, &scope, &card, paris)?;
    /// let View::Answered(review) = progress.view(&set, &scope) else {
    ///     return Err("the card is under review".into());
    /// };
    /// assert!(review.shows_correct_answer());
    ///
    /// // Only the answer is part of the history.
    /// assert_eq!(progress.history(&card), [Outcome::Wrong]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn retry(
        &mut self,
        set: &StudySet,
        scope: &Scope,
        card: &CardId,
        choice: Choice,
    ) -> Result<(), StudyError> {
        if let Scope::Topic(topic) = scope {
            set.topic(topic)
                .ok_or_else(|| StudyError::UnknownTopic(topic.clone()))?;
        }
        let View::Answered(review) = self.view(set, scope) else {
            return Err(StudyError::NoRetry);
        };
        if review.card.id() != card {
            return Err(StudyError::NotCurrent(card.clone()));
        }
        if review.shows_correct_answer() {
            return Err(StudyError::NoRetry);
        }
        let repeated = match choice {
            Choice::DontKnow => false,
            Choice::Answer(index) if review.card.choices().get(index).is_none() => {
                return Err(StudyError::NoSuchChoice(index));
            }
            Choice::Answer(index) => review.chose(index),
        };
        if !repeated {
            let answered = self
                .studied_queue_mut(scope)
                .and_then(|queue| queue.review.as_mut());
            answered.ok_or(StudyError::NoRetry)?.retries.push(choice);
        }
        Ok(())
    }

    /// Ends the review of the answered card, so the next question is shown.
    ///
    /// A scope that has not been studied has no review and stays as it is.
    pub fn advance(&mut self, scope: &Scope) {
        if let Some(queue) = self.studied_queue_mut(scope) {
            queue.review = None;
        }
    }

    /// Returns the queue of `scope`, if it has been studied.
    fn queue(&self, scope: &Scope) -> Option<&Queue> {
        match scope {
            Scope::All => Some(&self.all),
            Scope::Topic(topic) => self.topics.get(topic),
        }
    }

    /// Returns the queue of `scope` for changing, if it has been studied.
    fn studied_queue_mut(&mut self, scope: &Scope) -> Option<&mut Queue> {
        match scope {
            Scope::All => Some(&mut self.all),
            Scope::Topic(topic) => self.topics.get_mut(topic),
        }
    }

    /// Returns the queue of `scope`, creating it when needed.
    fn queue_mut(&mut self, scope: &Scope) -> &mut Queue {
        match scope {
            Scope::All => &mut self.all,
            Scope::Topic(topic) => self.topics.entry(topic.clone()).or_default(),
        }
    }

    /// Returns every card that waits in a batch of `scope`.
    ///
    /// Scopes queue cards independently, so that a topic session reaches all
    /// cards of the topic whatever another session has queued. A card can
    /// therefore wait in the batches of two scopes at once.
    fn queued_cards(&self, scope: &Scope) -> BTreeSet<&CardId> {
        self.queue(scope)
            .into_iter()
            .flat_map(|queue| &queue.open)
            .flat_map(|batch| &batch.slots)
            .map(|slot| &slot.card)
            .collect()
    }

    /// Removes what no longer matches the set from the queue of `scope`.
    fn reconcile(&mut self, set: &StudySet, scope: &Scope, cards: &[&Card]) {
        let queue = self.queue_mut(scope);
        if queue
            .review
            .as_ref()
            .is_some_and(|answered| set.card(&answered.card).is_none())
        {
            queue.review = None;
        }
        // The queue comes from a file, so it can be of any size: every step
        // below takes time in proportion to it and not to its square.
        let answers: BTreeMap<&CardId, usize> = cards
            .iter()
            .map(|card| (card.id(), card.choices().len()))
            .collect();
        for batch in &mut queue.open {
            batch.slots.retain(|slot| {
                answers
                    .get(&slot.card)
                    .is_some_and(|&count| is_order_of(&slot.order, count))
            });
        }
        // A batch whose unanswered cards were all removed counts as completed.
        let mut completed = None;
        queue.open.retain(|batch| {
            if batch.current().is_some() {
                return true;
            }
            if !batch.slots.is_empty() {
                completed = Some(batch.misses());
            }
            false
        });
        if let Some(misses) = completed {
            queue.previous_misses = misses;
        }
    }

    /// Chooses batches for `scope` until enough are waiting or no card is free.
    fn refill(&mut self, scope: &Scope, cards: &[&Card], random: &mut dyn RandomSource) {
        while self.queue_mut(scope).open.len() < QUEUED_BATCHES {
            let queued = self.queued_cards(scope);
            let candidates: Vec<Candidate<&Card>> = cards
                .iter()
                .filter(|card| !queued.contains(card.id()))
                .map(|&card| Candidate {
                    card,
                    level: self.cards.get(card.id()).and_then(History::level),
                })
                .collect();
            let previous_misses = self.queue_mut(scope).previous_misses;
            let chosen = picker::build_batch(candidates, previous_misses, random);
            if chosen.is_empty() {
                break;
            }
            let slots = chosen
                .into_iter()
                .map(|card| {
                    let mut order: Vec<usize> = (0..card.choices().len()).collect();
                    picker::shuffle(&mut order, random);
                    Slot {
                        card: card.id().clone(),
                        order,
                        outcome: None,
                    }
                })
                .collect();
            self.queue_mut(scope).open.push(Batch { slots });
        }
    }
}

/// Returns the cards of `scope` in set order.
fn scope_cards<'a>(set: &'a StudySet, scope: &Scope) -> Result<Vec<&'a Card>, StudyError> {
    match scope {
        Scope::All => Ok(set.cards().map(|(_, card)| card).collect()),
        Scope::Topic(id) => set
            .topic(id)
            .map(|topic| topic.cards().iter().collect())
            .ok_or_else(|| StudyError::UnknownTopic(id.clone())),
    }
}

/// Returns `true` when `order` lists each of the positions `0..len` once.
fn is_order_of(order: &[usize], len: usize) -> bool {
    order.len() == len && (0..len).all(|position| order.contains(&position))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{Choice, Progress, Review, Scope, StudyError, Tally, View};
    use crate::deck::{Card, CardDraft, CardId, StudySet, TopicId};
    use crate::mastery::Outcome;
    use crate::picker::BATCH_SIZE;

    fn rng(seed: u64) -> fastrand::Rng {
        fastrand::Rng::with_seed(seed)
    }

    /// Builds a set with the given number of cards in each topic.
    fn set_with(topic_sizes: &[usize]) -> StudySet {
        let mut set =
            StudySet::try_new("demo".parse().expect("valid id"), "Demo").expect("valid title");
        for (topic_number, &size) in topic_sizes.iter().enumerate() {
            let topic = set
                .add_topic(&format!("Topic {topic_number}"))
                .expect("valid topic");
            for card_number in 0..size {
                let draft = CardDraft {
                    question: format!("Question {topic_number} {card_number}"),
                    answer: "right".to_owned(),
                    wrong: vec!["wrong one".to_owned(), "wrong two".to_owned()],
                    explanation: None,
                };
                set.add_card(&topic, &draft).expect("valid card");
            }
        }
        set
    }

    fn topic_scope(set: &StudySet, index: usize) -> Scope {
        Scope::Topic(set.topics()[index].id().clone())
    }

    /// Returns the card being asked.
    fn asked(progress: &Progress, set: &StudySet, scope: &Scope) -> CardId {
        let view = progress.view(set, scope);
        let View::Question { card, .. } = view else {
            panic!("expected a question, found {view:?}");
        };
        card.id().clone()
    }

    /// Returns the review of the card that was answered last.
    fn reviewed<'session>(
        progress: &'session Progress,
        set: &'session StudySet,
        scope: &Scope,
    ) -> Review<'session> {
        let view = progress.view(set, scope);
        let View::Answered(review) = view else {
            panic!("expected a review, found {view:?}");
        };
        review
    }

    /// Starts a session over all cards of `set` and gives a wrong answer,
    /// the one at position `wrong` of the card's choices, to the first card.
    fn missed_card(set: &StudySet, wrong: usize, seed: u64) -> (Progress, CardId) {
        let mut progress = Progress::new();
        let mut random = rng(seed);
        progress
            .prepare(set, &Scope::All, &mut random)
            .expect("the scope exists");
        let card = asked(&progress, set, &Scope::All);
        let judged = progress
            .answer(set, &Scope::All, &card, Choice::Answer(wrong), &mut random)
            .expect("the answer is accepted");
        assert_eq!(judged, Outcome::Wrong, "the answer is judged wrong");
        (progress, card)
    }

    /// Answers the card being asked and moves on; returns the card.
    fn respond(
        progress: &mut Progress,
        set: &StudySet,
        scope: &Scope,
        correct: bool,
        random: &mut fastrand::Rng,
    ) -> CardId {
        let card = asked(progress, set, scope);
        let choice = if correct {
            Choice::Answer(Card::CORRECT_CHOICE)
        } else {
            Choice::Answer(1)
        };
        let outcome = progress
            .answer(set, scope, &card, choice, random)
            .expect("the answer is accepted");
        assert_eq!(outcome.is_correct(), correct, "the answer is judged");
        progress.advance(scope);
        card
    }

    #[test]
    fn a_new_session_queues_two_batches_of_distinct_cards() {
        let set = set_with(&[30]);
        let mut progress = Progress::new();
        progress
            .prepare(&set, &Scope::All, &mut rng(1))
            .expect("the scope exists");
        let queued = progress.queued_cards(&Scope::All);
        assert_eq!(
            queued.len(),
            2 * BATCH_SIZE,
            "two batches of distinct cards are queued"
        );
        assert!(
            matches!(progress.view(&set, &Scope::All), View::Question { .. }),
            "a question is shown"
        );
    }

    #[test]
    fn the_shown_order_lists_every_answer_once() {
        let set = set_with(&[30]);
        let mut progress = Progress::new();
        let mut orders = BTreeSet::new();
        progress
            .prepare(&set, &Scope::All, &mut rng(2))
            .expect("the scope exists");
        for _ in 0_u8..20 {
            let view = progress.view(&set, &Scope::All);
            let View::Question { card, order, .. } = view else {
                panic!("expected a question, found {view:?}");
            };
            let mut sorted = order.to_vec();
            sorted.sort_unstable();
            assert_eq!(
                sorted,
                [0, 1, 2],
                "each answer of {} is shown once",
                card.id()
            );
            orders.insert(order.to_vec());
            respond(&mut progress, &set, &Scope::All, true, &mut rng(3));
        }
        assert!(
            orders.len() > 1,
            "the answers are not always shown in the same order"
        );
    }

    #[test]
    fn an_answer_is_reviewed_until_the_learner_moves_on() {
        let set = set_with(&[30]);
        let scope = Scope::All;
        let mut progress = Progress::new();
        let mut random = rng(4);
        progress
            .prepare(&set, &scope, &mut random)
            .expect("the scope exists");
        let card = asked(&progress, &set, &scope);

        let judged = progress
            .answer(&set, &scope, &card, Choice::Answer(2), &mut random)
            .expect("the answer is accepted");
        assert_eq!(judged, Outcome::Wrong, "a wrong answer is judged wrong");
        let review = reviewed(&progress, &set, &scope);
        assert_eq!(review.card.id(), &card, "the answered card is under review");
        assert_eq!(review.choice, Choice::Answer(2), "the choice is remembered");
        assert_eq!(
            review.outcome,
            Outcome::Wrong,
            "the judgement is remembered"
        );
        assert_eq!(review.retries, [], "nothing was chosen after the answer");
        assert_eq!(
            progress.history(&card),
            [Outcome::Wrong],
            "the answer is in the history"
        );
        assert_eq!(
            progress.answer(&set, &scope, &card, Choice::DontKnow, &mut random),
            Err(StudyError::NoQuestion),
            "nothing can be answered during a review"
        );

        progress.advance(&scope);
        let next = asked(&progress, &set, &scope);
        assert_ne!(next, card, "the next card is asked after the review");
    }

    #[test]
    fn a_wrong_answer_can_be_retried_until_the_correct_one_is_chosen() {
        let set = set_with(&[30]);
        let scope = Scope::All;
        let correct = Choice::Answer(Card::CORRECT_CHOICE);
        let (mut progress, card) = missed_card(&set, 2, 20);
        let answered = progress.clone();
        let open = reviewed(&progress, &set, &scope);
        assert!(open.chose(2), "the answer counts as chosen");
        assert!(
            !open.chose(1) && !open.chose(Card::CORRECT_CHOICE),
            "the other answers have not been chosen"
        );
        assert!(
            !open.shows_correct_answer(),
            "a wrong answer leaves the correct one hidden"
        );

        assert_eq!(
            progress.retry(&set, &scope, &card, Choice::Answer(1)),
            Ok(()),
            "another wrong answer can be chosen"
        );
        let narrowed = reviewed(&progress, &set, &scope);
        assert_eq!(
            narrowed.retries,
            [Choice::Answer(1)],
            "the retry is remembered"
        );
        assert!(narrowed.chose(1), "the retried answer counts as chosen");
        assert!(
            !narrowed.shows_correct_answer(),
            "a wrong retry leaves the correct answer hidden"
        );

        assert_eq!(
            progress.retry(&set, &scope, &card, correct),
            Ok(()),
            "the correct answer can be chosen"
        );
        let found = reviewed(&progress, &set, &scope);
        assert_eq!(
            found.retries,
            [Choice::Answer(1), correct],
            "retries are kept oldest first"
        );
        assert!(
            found.shows_correct_answer(),
            "choosing the correct answer shows it"
        );
        assert_eq!(found.choice, Choice::Answer(2), "the answer stays as given");
        assert_eq!(
            found.outcome,
            Outcome::Wrong,
            "the answer stays judged wrong"
        );
        assert_eq!(
            progress.retry(&set, &scope, &card, Choice::Answer(1)),
            Err(StudyError::NoRetry),
            "the retries end once the correct answer is shown"
        );

        let mut expected = answered;
        expected
            .all
            .review
            .as_mut()
            .expect("the card is under review")
            .retries = vec![Choice::Answer(1), correct];
        assert_eq!(
            progress, expected,
            "retries change neither a history nor a batch"
        );
        assert_eq!(
            progress.history(&card),
            [Outcome::Wrong],
            "only the answer is in the history"
        );

        progress.advance(&scope);
        assert_ne!(
            asked(&progress, &set, &scope),
            card,
            "the next card is asked after the review"
        );
        assert_eq!(
            progress.all.review, None,
            "the retries are forgotten with the review"
        );
    }

    #[test]
    fn declining_to_retry_shows_the_correct_answer() {
        let set = set_with(&[30]);
        let scope = Scope::All;
        let (mut progress, card) = missed_card(&set, 1, 21);

        assert_eq!(
            progress.retry(&set, &scope, &card, Choice::DontKnow),
            Ok(()),
            "the learner can decline to choose again"
        );
        let review = reviewed(&progress, &set, &scope);
        assert!(
            review.shows_correct_answer(),
            "declining shows the correct answer"
        );
        assert!(
            !review.chose(Card::CORRECT_CHOICE),
            "the correct answer was not chosen"
        );
        assert_eq!(
            review.retries,
            [Choice::DontKnow],
            "declining is remembered"
        );
        assert_eq!(
            progress.history(&card),
            [Outcome::Wrong],
            "the answer stays a wrong one in the history"
        );
        assert_eq!(
            progress.retry(&set, &scope, &card, Choice::Answer(Card::CORRECT_CHOICE)),
            Err(StudyError::NoRetry),
            "the retries end once the correct answer is shown"
        );
    }

    #[test]
    fn choosing_an_answer_again_changes_nothing() {
        let set = set_with(&[30]);
        let scope = Scope::All;
        let (mut progress, card) = missed_card(&set, 2, 22);
        progress
            .retry(&set, &scope, &card, Choice::Answer(1))
            .expect("the retry is accepted");
        let before = progress.clone();

        for position in [2, 1] {
            assert_eq!(
                progress.retry(&set, &scope, &card, Choice::Answer(position)),
                Ok(()),
                "answer {position} can be chosen again"
            );
            assert_eq!(
                progress, before,
                "answer {position} is not recorded a second time"
            );
        }
    }

    #[test]
    fn retries_are_refused_unless_a_wrong_answer_is_under_review() {
        let set = set_with(&[30, 4]);
        let scope = Scope::All;
        let correct = Choice::Answer(Card::CORRECT_CHOICE);
        let mut progress = Progress::new();
        let mut random = rng(23);
        progress
            .prepare(&set, &scope, &mut random)
            .expect("the scope exists");

        let known = asked(&progress, &set, &scope);
        assert_eq!(
            progress.retry(&set, &scope, &known, correct),
            Err(StudyError::NoRetry),
            "a card that is being asked has not been answered"
        );
        progress
            .answer(&set, &scope, &known, correct, &mut random)
            .expect("the answer is accepted");
        assert_eq!(
            progress.retry(&set, &scope, &known, Choice::Answer(1)),
            Err(StudyError::NoRetry),
            "a correct answer leaves nothing to retry"
        );
        progress.advance(&scope);

        let declined = asked(&progress, &set, &scope);
        progress
            .answer(&set, &scope, &declined, Choice::DontKnow, &mut random)
            .expect("the answer is accepted");
        assert_eq!(
            progress.retry(&set, &scope, &declined, correct),
            Err(StudyError::NoRetry),
            "declining to answer shows the correct answer"
        );
        progress.advance(&scope);

        let missed = asked(&progress, &set, &scope);
        progress
            .answer(&set, &scope, &missed, Choice::Answer(1), &mut random)
            .expect("the answer is accepted");
        let before = progress.clone();
        assert_eq!(
            progress.retry(&set, &scope, &known, correct),
            Err(StudyError::NotCurrent(known)),
            "a retry for another card is rejected"
        );
        assert_eq!(
            progress.retry(&set, &scope, &missed, Choice::Answer(3)),
            Err(StudyError::NoSuchChoice(3)),
            "a choice the card does not have is rejected"
        );
        let missing = Scope::Topic("missing".parse().expect("valid id"));
        assert_eq!(
            progress.retry(&set, &missing, &missed, correct),
            Err(StudyError::UnknownTopic(
                "missing".parse().expect("valid id")
            )),
            "an unknown topic is rejected"
        );
        assert_eq!(
            progress.retry(&set, &topic_scope(&set, 1), &missed, correct),
            Err(StudyError::NoRetry),
            "a scope that has not been studied has no card under review"
        );
        assert_eq!(progress, before, "rejected retries leave no trace");

        progress.advance(&scope);
        assert_eq!(
            progress.retry(&set, &scope, &missed, correct),
            Err(StudyError::NoRetry),
            "the retries end when the learner moves on"
        );
    }

    #[test]
    fn a_review_in_one_scope_is_not_retried_in_another() {
        let set = set_with(&[1]);
        let topic = topic_scope(&set, 0);
        let correct = Choice::Answer(Card::CORRECT_CHOICE);
        let (mut progress, card) = missed_card(&set, 1, 24);
        let mut random = rng(25);
        progress
            .prepare(&set, &topic, &mut random)
            .expect("the topic exists");
        assert_eq!(
            asked(&progress, &set, &topic),
            card,
            "the topic asks the card that is under review for the whole set"
        );

        assert_eq!(
            progress.retry(&set, &topic, &card, correct),
            Err(StudyError::NoRetry),
            "the card has not been answered in the topic"
        );
        progress
            .answer(&set, &topic, &card, Choice::Answer(2), &mut random)
            .expect("the answer is accepted");
        progress
            .retry(&set, &topic, &card, correct)
            .expect("the retry is accepted");
        assert!(
            reviewed(&progress, &set, &topic).shows_correct_answer(),
            "the review of the topic shows the correct answer"
        );
        assert!(
            !reviewed(&progress, &set, &Scope::All).shows_correct_answer(),
            "the review of the whole set still hides it"
        );
    }

    #[test]
    fn a_retry_fits_the_card_as_it_is() {
        let mut set = set_with(&[3]);
        let scope = Scope::All;
        let (mut progress, card) = missed_card(&set, 2, 26);
        // The card loses the answer that was chosen while it is under review.
        let draft = CardDraft {
            question: "Reworded".to_owned(),
            answer: "right".to_owned(),
            wrong: vec!["only one wrong answer".to_owned()],
            explanation: None,
        };
        set.update_card(&card, &draft).expect("valid update");
        progress
            .prepare(&set, &scope, &mut rng(27))
            .expect("the scope exists");

        assert_eq!(
            progress.retry(&set, &scope, &card, Choice::Answer(2)),
            Err(StudyError::NoSuchChoice(2)),
            "an answer the card has lost cannot be chosen"
        );
        assert_eq!(
            progress.retry(&set, &scope, &card, Choice::Answer(1)),
            Ok(()),
            "an answer the card has kept can be chosen"
        );

        set.remove_card(&card).expect("the card exists");
        assert_eq!(
            progress.retry(&set, &scope, &card, Choice::DontKnow),
            Err(StudyError::NoRetry),
            "a card that left the set is not under review"
        );
    }

    #[test]
    fn moving_on_in_a_scope_that_has_not_been_studied_changes_nothing() {
        let set = set_with(&[3, 3]);
        let mut progress = Progress::new();
        progress.advance(&topic_scope(&set, 1));
        progress.advance(&Scope::Topic("missing".parse().expect("valid id")));
        progress.advance(&Scope::All);
        assert_eq!(
            progress,
            Progress::new(),
            "no queue is made for a scope by moving on in it"
        );
    }

    #[test]
    fn answers_are_judged_by_the_chosen_position() {
        let set = set_with(&[30]);
        let scope = Scope::All;
        let mut progress = Progress::new();
        let mut random = rng(5);
        progress
            .prepare(&set, &scope, &mut random)
            .expect("the scope exists");
        let cases = [
            (Choice::Answer(Card::CORRECT_CHOICE), Outcome::Correct),
            (Choice::Answer(1), Outcome::Wrong),
            (Choice::DontKnow, Outcome::DontKnow),
        ];
        for (choice, expected) in cases {
            let card = asked(&progress, &set, &scope);
            assert_eq!(
                progress.answer(&set, &scope, &card, choice, &mut random),
                Ok(expected),
                "judgement of {choice:?}"
            );
            progress.advance(&scope);
        }
    }

    #[test]
    fn answers_that_do_not_fit_the_question_are_rejected() {
        let set = set_with(&[30]);
        let scope = Scope::All;
        let mut progress = Progress::new();
        let mut random = rng(6);
        progress
            .prepare(&set, &scope, &mut random)
            .expect("the scope exists");
        let card = asked(&progress, &set, &scope);
        let other = set
            .cards()
            .map(|(_, candidate)| candidate.id().clone())
            .find(|id| *id != card)
            .expect("the set has other cards");

        assert_eq!(
            progress.answer(&set, &scope, &other, Choice::DontKnow, &mut random),
            Err(StudyError::NotCurrent(other)),
            "an answer for another card is rejected"
        );
        assert_eq!(
            progress.answer(&set, &scope, &card, Choice::Answer(3), &mut random),
            Err(StudyError::NoSuchChoice(3)),
            "a choice the card does not have is rejected"
        );
        let missing = Scope::Topic("missing".parse().expect("valid id"));
        assert_eq!(
            progress.answer(&set, &missing, &card, Choice::DontKnow, &mut random),
            Err(StudyError::UnknownTopic(
                "missing".parse().expect("valid id")
            )),
            "an unknown topic is rejected"
        );
        assert!(
            progress.history(&card).is_empty(),
            "rejected answers leave no trace"
        );
        assert_eq!(
            asked(&progress, &set, &scope),
            card,
            "the same card is still asked"
        );
    }

    #[test]
    fn missed_cards_return_in_the_batch_after_next() {
        let set = set_with(&[60]);
        let scope = Scope::All;
        let mut progress = Progress::new();
        let mut random = rng(7);
        progress
            .prepare(&set, &scope, &mut random)
            .expect("the scope exists");

        let first: Vec<CardId> = (0..BATCH_SIZE)
            .map(|_| respond(&mut progress, &set, &scope, false, &mut random))
            .collect();
        let second: Vec<CardId> = (0..BATCH_SIZE)
            .map(|_| respond(&mut progress, &set, &scope, true, &mut random))
            .collect();
        let third: BTreeSet<CardId> = (0..BATCH_SIZE)
            .map(|_| respond(&mut progress, &set, &scope, true, &mut random))
            .collect();
        let fourth: BTreeSet<CardId> = (0..BATCH_SIZE)
            .map(|_| respond(&mut progress, &set, &scope, true, &mut random))
            .collect();

        assert!(
            second.iter().all(|card| !first.contains(card)),
            "the second batch was already chosen"
        );
        assert_eq!(
            third,
            first.iter().cloned().collect(),
            "the cards missed in the first batch make up the third"
        );
        assert!(
            fourth
                .iter()
                .all(|card| !first.contains(card) && !second.contains(card)),
            "with many unseen cards left, known cards are not reviewed"
        );
    }

    #[test]
    fn every_card_is_seen_before_any_correct_card_repeats() {
        let set = set_with(&[23]);
        let scope = Scope::All;
        let mut progress = Progress::new();
        let mut random = rng(8);
        progress
            .prepare(&set, &scope, &mut random)
            .expect("the scope exists");
        let mut seen = BTreeSet::new();
        let mut answers = 0_usize;
        while seen.len() < 23 {
            seen.insert(respond(&mut progress, &set, &scope, true, &mut random));
            answers += 1;
            assert!(
                answers <= 60,
                "all cards are reached without endless repetition"
            );
        }
        // Unseen cards rank at random among once-correct cards, so a few
        // reviews happen before the last unseen card appears.
        assert!(answers < 60, "every card was seen after {answers} answers");
        let tally = progress.tally(set.topics()[0].cards());
        assert_eq!(tally.seen, 23, "all cards count as seen");
    }

    #[test]
    fn a_small_scope_keeps_cycling() {
        for size in 1..=7 {
            let set = set_with(&[size]);
            let scope = Scope::All;
            let mut progress = Progress::new();
            let mut random = rng(9);
            progress
                .prepare(&set, &scope, &mut random)
                .expect("the scope exists");
            let mut seen = BTreeSet::new();
            for round in 0_u8..30 {
                seen.insert(respond(
                    &mut progress,
                    &set,
                    &scope,
                    round % 3 != 0_u8,
                    &mut random,
                ));
            }
            assert_eq!(seen.len(), size, "all {size} cards keep being asked");
        }
    }

    #[test]
    fn an_empty_scope_shows_nothing() {
        let set = set_with(&[0, 3]);
        let empty = topic_scope(&set, 0);
        let mut progress = Progress::new();
        let mut random = rng(10);
        progress
            .prepare(&set, &empty, &mut random)
            .expect("the topic exists");
        assert_eq!(
            progress.view(&set, &empty),
            View::Empty,
            "a topic without cards is empty"
        );
        let card = set.topics()[1].cards()[0].id().clone();
        assert_eq!(
            progress.answer(&set, &empty, &card, Choice::DontKnow, &mut random),
            Err(StudyError::NoQuestion),
            "nothing can be answered in an empty scope"
        );
        assert_eq!(
            progress.prepare(
                &set,
                &Scope::Topic("missing".parse().expect("valid id")),
                &mut random
            ),
            Err(StudyError::UnknownTopic(
                "missing".parse().expect("valid id")
            )),
            "an unknown topic is reported"
        );
        assert_eq!(
            Progress::new().view(&set, &Scope::All),
            View::Empty,
            "an unprepared session shows nothing"
        );
    }

    #[test]
    fn a_topic_session_asks_only_that_topic() {
        let set = set_with(&[12, 12]);
        let scope = topic_scope(&set, 1);
        let topic_cards: BTreeSet<CardId> = set.topics()[1]
            .cards()
            .iter()
            .map(|card| card.id().clone())
            .collect();
        let mut progress = Progress::new();
        let mut random = rng(11);
        progress
            .prepare(&set, &scope, &mut random)
            .expect("the topic exists");
        for _ in 0_u8..40 {
            let card = respond(&mut progress, &set, &scope, true, &mut random);
            assert!(topic_cards.contains(&card), "{card} belongs to the topic");
        }
    }

    #[test]
    fn a_card_is_queued_once_within_a_scope() {
        let set = set_with(&[23]);
        let scope = Scope::All;
        let mut progress = Progress::new();
        let mut random = rng(12);
        progress
            .prepare(&set, &scope, &mut random)
            .expect("the scope exists");
        for round in 0_u8..60 {
            respond(&mut progress, &set, &scope, round % 4 != 0_u8, &mut random);
            let slots: Vec<&CardId> = progress
                .all
                .open
                .iter()
                .flat_map(|batch| &batch.slots)
                .map(|slot| &slot.card)
                .collect();
            let distinct: BTreeSet<&CardId> = slots.iter().copied().collect();
            assert_eq!(
                slots.len(),
                distinct.len(),
                "no card waits twice in one scope"
            );
        }
    }

    #[test]
    fn a_topic_session_reaches_every_card_of_the_topic() {
        // The session over all cards has queued ten of the thirteen cards.
        let set = set_with(&[7, 6]);
        let topic = topic_scope(&set, 0);
        let mut progress = Progress::new();
        let mut random = rng(18);
        progress
            .prepare(&set, &Scope::All, &mut random)
            .expect("the scope exists");
        progress
            .prepare(&set, &topic, &mut random)
            .expect("the topic exists");
        let mut asked_in_topic = BTreeSet::new();
        for _ in 0_u8..30 {
            asked_in_topic.insert(respond(&mut progress, &set, &topic, true, &mut random));
        }
        assert_eq!(
            asked_in_topic.len(),
            7,
            "all cards of the topic are asked while another session holds some in its queue"
        );
    }

    #[test]
    fn removed_cards_leave_the_queue() {
        let mut set = set_with(&[12]);
        let scope = Scope::All;
        let mut progress = Progress::new();
        let mut random = rng(13);
        progress
            .prepare(&set, &scope, &mut random)
            .expect("the scope exists");
        let current = asked(&progress, &set, &scope);
        progress
            .answer(&set, &scope, &current, Choice::DontKnow, &mut random)
            .expect("the answer is accepted");
        // The answered card is still part of its unfinished batch.
        let queued: Vec<CardId> = progress.queued_cards(&scope).into_iter().cloned().collect();
        assert!(
            queued.contains(&current),
            "the answered card is still queued"
        );

        for card in &queued {
            set.remove_card(card).expect("the card exists");
        }
        progress
            .prepare(&set, &scope, &mut random)
            .expect("the scope exists");

        let remaining: BTreeSet<CardId> = set.cards().map(|(_, card)| card.id().clone()).collect();
        assert!(!remaining.is_empty(), "some cards are left in the set");
        assert!(
            progress
                .queued_cards(&scope)
                .into_iter()
                .all(|card| remaining.contains(card)),
            "only existing cards are queued"
        );
        let next = asked(&progress, &set, &scope);
        assert!(remaining.contains(&next), "an existing card is asked");
    }

    #[test]
    fn a_card_with_changed_answers_is_asked_afresh() {
        let mut set = set_with(&[3]);
        let scope = Scope::All;
        let mut progress = Progress::new();
        let mut random = rng(14);
        progress
            .prepare(&set, &scope, &mut random)
            .expect("the scope exists");
        let card = asked(&progress, &set, &scope);
        let draft = CardDraft {
            question: "Reworded".to_owned(),
            answer: "right".to_owned(),
            wrong: vec!["only one wrong answer".to_owned()],
            explanation: None,
        };
        set.update_card(&card, &draft).expect("valid update");
        progress
            .prepare(&set, &scope, &mut random)
            .expect("the scope exists");
        for _ in 0_u8..6 {
            let view = progress.view(&set, &scope);
            let View::Question {
                card: shown, order, ..
            } = view
            else {
                panic!("expected a question, found {view:?}");
            };
            assert_eq!(
                order.len(),
                shown.choices().len(),
                "the order fits the card's answers"
            );
            respond(&mut progress, &set, &scope, true, &mut random);
        }
    }

    #[test]
    fn progress_survives_a_round_trip_through_json() {
        let set = set_with(&[8, 8]);
        let topic = topic_scope(&set, 1);
        let mut progress = Progress::new();
        let mut random = rng(15);
        progress
            .prepare(&set, &Scope::All, &mut random)
            .expect("the scope exists");
        progress
            .prepare(&set, &topic, &mut random)
            .expect("the topic exists");
        respond(&mut progress, &set, &Scope::All, false, &mut random);
        let missed = asked(&progress, &set, &Scope::All);
        progress
            .answer(&set, &Scope::All, &missed, Choice::Answer(1), &mut random)
            .expect("accepted");
        progress
            .retry(&set, &Scope::All, &missed, Choice::Answer(2))
            .expect("accepted");
        let card = asked(&progress, &set, &topic);
        progress
            .answer(&set, &topic, &card, Choice::DontKnow, &mut random)
            .expect("accepted");

        let json = serde_json::to_string(&progress).expect("progress serializes");
        let restored: Progress = serde_json::from_str(&json).expect("progress parses");
        assert_eq!(restored, progress, "nothing is lost");
        for scope in [Scope::All, topic] {
            assert_eq!(
                restored.view(&set, &scope),
                progress.view(&set, &scope),
                "the same view is shown"
            );
        }
        assert_eq!(
            reviewed(&restored, &set, &Scope::All).retries,
            [Choice::Answer(2)],
            "a retry is kept with its review"
        );
        let empty: Progress =
            serde_json::from_str("{}").expect("an empty object is valid progress");
        assert_eq!(empty, Progress::new(), "missing keys mean no progress");
    }

    #[test]
    fn progress_is_stored_in_the_documented_form() {
        const STORED: &str = r#"{
            "cards": {
                "question-0-0": ["correct", "dont_know"],
                "question-0-1": ["wrong"]
            },
            "all": {
                "open": [
                    {
                        "slots": [
                            {"card": "question-0-1", "order": [2, 0, 1], "outcome": "wrong"},
                            {"card": "question-0-2", "order": [1, 2, 0], "outcome": null}
                        ]
                    }
                ],
                "previous_misses": 1,
                "review": {
                    "card": "question-0-1",
                    "order": [2, 0, 1],
                    "choice": {"answer": 2},
                    "outcome": "wrong",
                    "retries": [{"answer": 1}, "dont_know"]
                }
            },
            "topics": {
                "topic-0": {"open": [], "previous_misses": 0, "review": null}
            }
        }"#;
        let set = set_with(&[3]);
        let card = |text: &str| -> CardId { text.parse().expect("valid id") };
        let mut progress: Progress = serde_json::from_str(STORED).expect("the form is read");

        assert_eq!(
            progress.history(&card("question-0-0")),
            [Outcome::Correct, Outcome::DontKnow],
            "answers are read newest first"
        );
        let review = reviewed(&progress, &set, &Scope::All);
        assert_eq!(review.card.id(), &card("question-0-1"), "the reviewed card");
        assert_eq!(
            review.order,
            [2, 0, 1],
            "the order its answers were shown in"
        );
        assert_eq!(
            review.choice,
            Choice::Answer(2),
            "the answer that was given"
        );
        assert_eq!(review.outcome, Outcome::Wrong, "how the answer was judged");
        assert_eq!(
            review.retries,
            [Choice::Answer(1), Choice::DontKnow],
            "what was chosen after the answer, oldest first"
        );
        assert!(
            review.shows_correct_answer(),
            "declining in a retry shows the correct answer"
        );

        let written = serde_json::to_value(&progress).expect("progress serializes");
        let stored: serde_json::Value = serde_json::from_str(STORED).expect("valid JSON");
        assert_eq!(
            written, stored,
            "progress is written in the form it is read"
        );

        progress.advance(&Scope::All);
        assert_eq!(
            asked(&progress, &set, &Scope::All),
            card("question-0-2"),
            "the unanswered card of the open batch is asked next"
        );
        let declined = serde_json::to_string(&Choice::DontKnow).expect("a choice serializes");
        assert_eq!(
            declined, "\"dont_know\"",
            "stored form of declining to guess"
        );
    }

    #[test]
    fn a_stored_queue_is_brought_in_line_with_the_set() {
        // Two cards of three exist; `gone` was removed from the set, and the
        // order stored for `question-0-1` has too few positions.
        const STORED: &str = r#"{
            "all": {
                "open": [
                    {"slots": []},
                    {"slots": [{"card": "gone", "order": [0, 1, 2], "outcome": null}]},
                    {"slots": [
                        {"card": "question-0-0", "order": [0, 1, 2], "outcome": "wrong"},
                        {"card": "question-0-2", "order": [2, 1, 0], "outcome": "dont_know"},
                        {"card": "gone", "order": [0, 1, 2], "outcome": null}
                    ]},
                    {"slots": [
                        {"card": "question-0-1", "order": [0, 1], "outcome": null},
                        {"card": "question-0-2", "order": [1, 0, 2], "outcome": null},
                        {"card": "question-0-2", "order": [0, 1, 7], "outcome": null}
                    ]},
                    {"slots": []}
                ],
                "previous_misses": 0,
                "review": {
                    "card": "gone",
                    "order": [0, 1, 2],
                    "choice": "dont_know",
                    "outcome": "dont_know"
                }
            }
        }"#;
        let set = set_with(&[3]);
        let mut progress: Progress = serde_json::from_str(STORED).expect("the form is read");
        progress
            .prepare(&set, &Scope::All, &mut rng(19))
            .expect("the scope exists");

        let view = progress.view(&set, &Scope::All);
        let View::Question { card, order, .. } = view else {
            panic!("expected a question, found {view:?}");
        };
        assert_eq!(
            card.id().as_str(),
            "question-0-2",
            "the first slot that still fits its card is asked"
        );
        assert_eq!(order, [1, 0, 2], "in the order stored for it");
        assert_eq!(
            progress.all.open.first().map(|batch| batch.slots.len()),
            Some(1),
            "slots for removed cards and with unusable orders are dropped"
        );
        assert_eq!(
            progress.all.previous_misses, 2,
            "the batch that lost its last unanswered card counts as completed"
        );
        assert_eq!(
            progress.all.review, None,
            "the review of a removed card ends"
        );
    }

    #[test]
    fn stored_retries_of_any_content_are_read() {
        // Position 9 is not an answer of the card, and position 2, which is
        // the answer as well, is listed twice.
        const UNENDED: &str = r#"{
            "all": {
                "review": {
                    "card": "question-0-0",
                    "order": [0, 1, 2],
                    "choice": {"answer": 2},
                    "outcome": "wrong",
                    "retries": [{"answer": 9}, {"answer": 2}, {"answer": 2}]
                }
            }
        }"#;
        // An entry follows the one that ends the retries.
        const ENDED: &str = r#"{
            "all": {
                "review": {
                    "card": "question-0-0",
                    "order": [0, 1, 2],
                    "choice": {"answer": 2},
                    "outcome": "wrong",
                    "retries": ["dont_know", {"answer": 1}]
                }
            }
        }"#;
        let set = set_with(&[3]);
        let scope = Scope::All;
        let card: CardId = "question-0-0".parse().expect("valid id");

        let mut unended: Progress = serde_json::from_str(UNENDED).expect("the form is read");
        let open = reviewed(&unended, &set, &scope);
        assert!(open.chose(2), "the answer counts as chosen");
        assert!(!open.chose(1), "answer 1 has not been chosen");
        assert!(
            !open.shows_correct_answer(),
            "no entry chooses the correct answer or declines"
        );
        assert_eq!(
            unended.retry(&set, &scope, &card, Choice::Answer(9)),
            Err(StudyError::NoSuchChoice(9)),
            "a position the card does not have cannot be chosen"
        );
        assert_eq!(
            unended.retry(&set, &scope, &card, Choice::Answer(1)),
            Ok(()),
            "the review accepts a retry"
        );

        let mut ended: Progress = serde_json::from_str(ENDED).expect("the form is read");
        let shown = reviewed(&ended, &set, &scope);
        assert!(
            shown.shows_correct_answer(),
            "declining shows the correct answer whatever follows it"
        );
        assert!(shown.chose(1), "the entry that follows counts as chosen");
        assert_eq!(
            ended.retry(&set, &scope, &card, Choice::Answer(1)),
            Err(StudyError::NoRetry),
            "the review accepts no retry"
        );
    }

    #[test]
    fn absent_keys_have_defaults_and_unknown_keys_are_ignored() {
        let sparse: Progress = serde_json::from_str(
            r#"{"all": {"open": []}, "topics": {"topic-0": {}}, "unknown": [1, 2]}"#,
        )
        .expect("sparse progress is read");
        let complete: serde_json::Value = serde_json::from_str(
            r#"{
                "cards": {},
                "all": {"open": [], "previous_misses": 0, "review": null},
                "topics": {
                    "topic-0": {"open": [], "previous_misses": 0, "review": null}
                }
            }"#,
        )
        .expect("valid JSON");
        assert_eq!(
            serde_json::to_value(&sparse).expect("progress serializes"),
            complete,
            "absent keys stand for no answers, no batches, zero, and no review"
        );
        let review: Progress = serde_json::from_str(
            r#"{"all": {"review": {
                "card": "question-0-0",
                "order": [0, 1, 2],
                "choice": {"answer": 1},
                "outcome": "wrong"
            }}}"#,
        )
        .expect("a review without retries is read");
        assert_eq!(
            serde_json::to_value(&review).expect("progress serializes")["all"]["review"]["retries"],
            serde_json::Value::Array(Vec::new()),
            "absent retries stand for none"
        );
        let misnamed: Result<Progress, serde_json::Error> =
            serde_json::from_str(r#"{"cards": {"Not An Id": []}}"#);
        assert!(
            misnamed.is_err(),
            "a card identifier that cannot be one is rejected"
        );
    }

    #[test]
    fn tallies_count_seen_cards_and_their_aptitude() {
        let set = set_with(&[4]);
        let scope = Scope::All;
        let mut progress = Progress::new();
        let mut random = rng(16);
        progress
            .prepare(&set, &scope, &mut random)
            .expect("the scope exists");
        let cards = set.topics()[0].cards();
        assert_eq!(
            progress.tally(cards),
            Tally {
                cards: 4,
                seen: 0,
                aptitude_points: 0
            },
            "nothing seen"
        );

        respond(&mut progress, &set, &scope, true, &mut random);
        respond(&mut progress, &set, &scope, false, &mut random);
        let tally = progress.tally(cards);
        // One correct answer scores 250 of 400 and one miss 50 of 400.
        assert_eq!(
            tally,
            Tally {
                cards: 4,
                seen: 2,
                aptitude_points: 300
            },
            "two cards seen"
        );
        assert_eq!(tally.seen_percent(), 50, "half of the cards are seen");
        assert_eq!(
            tally.aptitude_percent(),
            19,
            "300 of 1600 points is 18.75%, shown as 19"
        );
    }

    #[test]
    fn topic_tallies_add_up_to_the_set() {
        let set = set_with(&[3, 5]);
        let scope = Scope::All;
        let mut progress = Progress::new();
        let mut random = rng(17);
        progress
            .prepare(&set, &scope, &mut random)
            .expect("the scope exists");
        for _ in 0_u8..4 {
            respond(&mut progress, &set, &scope, true, &mut random);
        }
        let total: Tally = set
            .topics()
            .iter()
            .map(|topic| progress.tally(topic.cards()))
            .sum();
        assert_eq!(
            total,
            Tally {
                cards: 8,
                seen: 4,
                aptitude_points: 1000
            },
            "four once-correct cards of eight"
        );
    }

    #[test]
    fn percentages_round_half_up_and_tolerate_no_cards() {
        let empty = Tally::default();
        assert_eq!(empty.seen_percent(), 0, "no cards, no percentage");
        assert_eq!(empty.aptitude_percent(), 0, "no cards, no aptitude");
        let one_of_eight = Tally {
            cards: 8,
            seen: 1,
            aptitude_points: 400,
        };
        assert_eq!(one_of_eight.seen_percent(), 13, "12.5% rounds up");
        assert_eq!(one_of_eight.aptitude_percent(), 13, "12.5% rounds up");
        let one_of_three = Tally {
            cards: 3,
            seen: 1,
            aptitude_points: 250,
        };
        assert_eq!(one_of_three.seen_percent(), 33, "33.3% rounds down");
        assert_eq!(one_of_three.aptitude_percent(), 21, "20.8% rounds up");
        let all = Tally {
            cards: 3,
            seen: 3,
            aptitude_points: 1200,
        };
        assert_eq!(all.aptitude_percent(), 100, "full marks");
    }

    #[test]
    fn unknown_topic_id_type_checks() {
        let id: TopicId = "t".parse().expect("valid id");
        assert_eq!(
            StudyError::UnknownTopic(id).to_string(),
            "there is no topic \"t\"",
            "message for an unknown topic"
        );
    }

    #[test]
    fn refused_choices_are_explained() {
        let card: CardId = "c".parse().expect("valid id");
        let cases = [
            (
                StudyError::NoQuestion,
                "no question is waiting for an answer",
            ),
            (StudyError::NoRetry, "no card is waiting for another choice"),
            (
                StudyError::NotCurrent(card),
                "card \"c\" is not the current card",
            ),
            (
                StudyError::NoSuchChoice(7),
                "the card has no answer number 7",
            ),
        ];
        for (error, message) in cases {
            assert_eq!(error.to_string(), message, "message of {error:?}");
        }
    }
}
