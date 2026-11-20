//! Local flash-card study for your own question sets.
//!
//! The crate is the library behind the `studycards` program. A
//! [`StudySet`](deck::StudySet) is a titled list of topics, and a topic is a
//! named list of multiple-choice cards. Studying a set asks its cards in
//! batches. Every answer joins the [`History`](mastery::History) of its card,
//! and the histories decide which cards the following batches hold: the cards
//! known least return first. [`Progress`](study::Progress) holds the histories
//! and the batches that are waiting, and is what the program stores between
//! runs.
//!
//! - [`deck`] defines study sets and their TOML file format.
//! - [`mastery`] scores how well a card is known from its answer history.
//! - [`picker`] chooses the cards of the next batch.
//! - [`study`] runs a study session and records progress.
//! - [`store`] keeps sets and progress as files.
//! - [`http`] is a small loopback HTTP server, and [`web`] the application
//!   served through it.
//! - [`cli`] is the program's command line.
//!
//! # Examples
//!
//! Reading a set, answering its card, and totalling what has been learned. The
//! examples in [`study`], [`picker`], and [`mastery`] show the steps in detail,
//! and the example in [`web`] serves the application.
//!
//! ```
//! use studycards::deck::{Card, StudySet};
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
//! let scope = Scope::All;
//! let mut random = fastrand::Rng::with_seed(1);
//! let mut progress = Progress::new();
//!
//! progress.prepare(&set, &scope, &mut random)?;
//! let View::Question { card, .. } = progress.view(&set, &scope) else {
//!     return Err("a question is waiting".into());
//! };
//! let card = card.id().clone();
//! let paris = Choice::Answer(Card::CORRECT_CHOICE);
//! progress.answer(&set, &scope, &card, paris, &mut random)?;
//!
//! let europe = set.topics().first().ok_or("the set has a topic")?;
//! let tally = progress.tally(europe.cards());
//! assert_eq!(tally.seen_percent(), 100);
//! assert_eq!(tally.aptitude_percent(), 63);
//! # Ok(())
//! # }
//! ```

pub mod cli;
pub mod deck;
pub mod http;
pub mod mastery;
pub mod picker;
#[cfg(test)]
mod scratch;
pub mod store;
pub mod study;
pub mod web;
