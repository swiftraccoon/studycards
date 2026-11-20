//! How well a card is known, derived from its answer history.
//!
//! Two scores come from the same [`History`]. [`Level`] ranks cards when the
//! next batch is chosen; it weighs the ten most recent answers, halving the
//! weight of each older one. [`Aptitude`] is the score shown to the learner
//! and looks only at the four most recent answers.
//!
//! Both scores use integer arithmetic, so equal histories always produce equal
//! scores.

use serde::{Deserialize, Serialize};

/// The result of one attempt at a card.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The correct answer was chosen.
    Correct,
    /// A wrong answer was chosen.
    Wrong,
    /// The learner declined to guess.
    DontKnow,
}

impl Outcome {
    /// Returns `true` for [`Outcome::Correct`].
    #[must_use]
    pub const fn is_correct(self) -> bool {
        matches!(self, Self::Correct)
    }

    /// Returns `true` when the card was not answered correctly, whether by a
    /// wrong choice or by declining to guess.
    #[must_use]
    pub const fn is_miss(self) -> bool {
        !self.is_correct()
    }
}

/// A ranking score from 2 to 1026; higher means better known.
///
/// The newest answer contributes 512 when correct, the one before 256, and so
/// on down to 1 for the tenth. A card with few answers receives a small bonus
/// that halves with each of its first five answers. A card whose history holds
/// no correct answer has the fixed level [`Level::NEVER_CORRECT`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Level(u16);

impl Level {
    /// The level of an attempted card whose history holds no correct answer.
    pub const NEVER_CORRECT: Self = Self(2);

    /// One more than the highest level a history can reach.
    pub const LIMIT: u16 = 1027;

    /// Returns the level as a number.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// The learner-facing score for one card, in units of 1/400.
///
/// The four most recent answers weigh 50%, 25%, 15%, and 10%. A correct answer
/// earns its full weight and a miss earns nothing. A card with fewer than four
/// answers earns a quarter of the weight of each missing answer.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Aptitude(u16);

impl Aptitude {
    /// The score of a card whose four most recent answers are all correct.
    pub const FULL: u16 = 400;

    /// Returns the score in units of 1/400, from 0 to [`Aptitude::FULL`].
    #[must_use]
    pub const fn points(self) -> u16 {
        self.0
    }
}

/// Fractional bits carried while summing a level.
const FRACTION_BITS: u32 = 40;
/// The fixed-point value 1.
const ONE: u64 = 1 << FRACTION_BITS;
/// The fixed-point value 0.5, added before truncating to round half up.
const HALF: u64 = ONE >> 1;
/// Fixed-point weight of the newest answer (512).
const NEWEST_WEIGHT: u64 = 512 << FRACTION_BITS;
/// Fixed-point bonus of a card with no answers (48); halved per answer.
const STARTUP_BONUS: u64 = 48 << FRACTION_BITS;
/// The startup bonus stops shrinking after this many answers.
const STARTUP_ANSWERS: u32 = 5;
/// Percentage weights of the four most recent answers in an aptitude.
const APTITUDE_WEIGHTS: [u16; 4] = [50, 25, 15, 10];

/// The answers given to one card, newest first.
///
/// # Examples
///
/// A card missed once and then answered correctly scores lower, in level and
/// in aptitude, than a card answered correctly on the first attempt.
///
/// ```
/// use studycards::mastery::{Aptitude, History, Level, Outcome};
///
/// let mut recovered = History::new();
/// recovered.record(Outcome::Wrong);
/// recovered.record(Outcome::Correct);
///
/// let mut first_try = History::new();
/// first_try.record(Outcome::Correct);
///
/// assert_eq!(recovered.level().map(Level::get), Some(525));
/// assert_eq!(first_try.level().map(Level::get), Some(537));
/// assert_eq!(recovered.aptitude().map(Aptitude::points), Some(225));
/// assert_eq!(first_try.aptitude().map(Aptitude::points), Some(250));
/// ```
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(from = "Vec<Outcome>", into = "Vec<Outcome>")]
pub struct History(Vec<Outcome>);

impl History {
    /// The number of answers kept. Both scores depend only on the ten most
    /// recent answers, so older ones are dropped.
    pub const CAPACITY: usize = 10;

    /// Creates the history of a card that has not been attempted.
    #[must_use]
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    /// Adds `outcome` as the newest answer.
    pub fn record(&mut self, outcome: Outcome) {
        self.0.insert(0, outcome);
        self.0.truncate(Self::CAPACITY);
    }

    /// Returns `true` once the card has been attempted.
    #[must_use]
    pub const fn is_seen(&self) -> bool {
        !self.0.is_empty()
    }

    /// Returns the newest answer, if any.
    #[must_use]
    pub fn latest(&self) -> Option<Outcome> {
        self.0.first().copied()
    }

    /// Returns the answers, newest first.
    #[must_use]
    pub fn outcomes(&self) -> &[Outcome] {
        &self.0
    }

    /// Returns the ranking score, or `None` for a card that has not been
    /// attempted.
    #[must_use]
    pub fn level(&self) -> Option<Level> {
        if self.0.is_empty() {
            return None;
        }
        if !self.0.iter().any(|outcome| outcome.is_correct()) {
            return Some(Level::NEVER_CORRECT);
        }
        let weighted: u64 = self
            .0
            .iter()
            .enumerate()
            .filter(|(_, outcome)| outcome.is_correct())
            .map(|(age, _)| halved(NEWEST_WEIGHT, age))
            .sum();
        let bonus = halved(STARTUP_BONUS, self.0.len().min(widen(STARTUP_ANSWERS)));
        let total = ONE.saturating_add(weighted).saturating_add(bonus);
        let rounded = total.saturating_add(HALF) >> FRACTION_BITS;
        Some(Level(u16::try_from(rounded).unwrap_or(u16::MAX)))
    }

    /// Returns the learner-facing score, or `None` for a card that has not
    /// been attempted.
    #[must_use]
    pub fn aptitude(&self) -> Option<Aptitude> {
        if self.0.is_empty() {
            return None;
        }
        let mut answers = self.0.iter();
        let points = APTITUDE_WEIGHTS
            .iter()
            .map(|weight| {
                let quarters = match answers.next() {
                    Some(Outcome::Correct) => 4,
                    Some(Outcome::Wrong | Outcome::DontKnow) => 0,
                    None => 1,
                };
                weight.saturating_mul(quarters)
            })
            .sum();
        Some(Aptitude(points))
    }
}

impl From<Vec<Outcome>> for History {
    /// Builds a history from answers listed newest first, keeping the newest
    /// [`History::CAPACITY`].
    fn from(mut outcomes: Vec<Outcome>) -> Self {
        outcomes.truncate(Self::CAPACITY);
        Self(outcomes)
    }
}

impl From<History> for Vec<Outcome> {
    fn from(history: History) -> Self {
        history.0
    }
}

impl FromIterator<Outcome> for History {
    /// Builds a history from answers listed newest first, keeping the newest
    /// [`History::CAPACITY`].
    fn from_iter<I: IntoIterator<Item = Outcome>>(iter: I) -> Self {
        Self(iter.into_iter().take(Self::CAPACITY).collect())
    }
}

/// Halves a fixed-point `value` once per step, reaching zero when nothing is
/// left.
fn halved(value: u64, steps: usize) -> u64 {
    u32::try_from(steps)
        .ok()
        .and_then(|steps| value.checked_shr(steps))
        .unwrap_or(0)
}

/// Converts a small constant to `usize`.
fn widen(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use std::iter;

    use super::{Aptitude, History, Level, Outcome};

    /// Parses a history written newest first: `c` correct, `w` wrong, and `?`
    /// don't know.
    fn history(pattern: &str) -> History {
        pattern
            .chars()
            .map(|symbol| match symbol {
                'c' => Outcome::Correct,
                'w' => Outcome::Wrong,
                '?' => Outcome::DontKnow,
                other => panic!("unknown outcome symbol {other:?}"),
            })
            .collect()
    }

    /// Histories, written as [`history`] reads them, with the level and the
    /// aptitude (in 1/400) that each one scores.
    const EXPECTED: &[(&str, u16, u16)] = &[
        ("c", 537, 250),
        ("w", 2, 50),
        ("cw", 525, 225),
        ("ww", 2, 25),
        ("ccc", 903, 370),
        ("cww", 519, 210),
        ("cccc", 964, 400),
        ("cwww", 516, 200),
        ("?ccc?", 451, 200),
        ("ccc?c", 931, 360),
        ("ccccc", 995, 400),
        ("ccccw", 963, 400),
        ("ccwww", 771, 300),
        ("cwwww", 515, 200),
        ("wcccc", 483, 200),
        ("cccc??", 963, 400),
        ("cccc?c", 979, 400),
        ("cccccw", 995, 400),
        ("ccccw?", 963, 400),
        ("ccccww", 963, 400),
        ("cwccc?", 739, 300),
        ("wwwccc", 115, 40),
        ("cccccw?", 995, 400),
        ("cccccww", 995, 400),
        ("cccwccc", 955, 360),
        ("ccccwwcw", 971, 400),
        ("wwcccc??", 243, 100),
        ("cccw?ccc?", 927, 360),
        ("cccwccwww", 947, 360),
        ("cwwcccwccw", 633, 240),
        ("wwcwcwccc?", 177, 60),
    ];

    #[test]
    fn histories_score_the_expected_level_and_aptitude() {
        for &(pattern, level, aptitude) in EXPECTED {
            let history = history(pattern);
            assert_eq!(
                history.level().map(Level::get),
                Some(level),
                "level of history {pattern:?}"
            );
            assert_eq!(
                history.aptitude().map(Aptitude::points),
                Some(aptitude),
                "aptitude of history {pattern:?}"
            );
        }
    }

    #[test]
    fn unseen_card_has_no_scores() {
        let history = History::new();
        assert!(!history.is_seen(), "a new history is unseen");
        assert_eq!(history.level(), None, "an unseen card has no level");
        assert_eq!(history.aptitude(), None, "an unseen card has no aptitude");
        assert_eq!(
            history.latest(),
            None,
            "an unseen card has no latest answer"
        );
    }

    #[test]
    fn record_puts_the_newest_answer_first() {
        let mut history = History::new();
        history.record(Outcome::Wrong);
        history.record(Outcome::DontKnow);
        history.record(Outcome::Correct);
        assert_eq!(
            history.outcomes(),
            [Outcome::Correct, Outcome::DontKnow, Outcome::Wrong],
            "answers are stored newest first"
        );
        assert_eq!(
            history.latest(),
            Some(Outcome::Correct),
            "latest is the newest answer"
        );
    }

    #[test]
    fn history_keeps_only_the_newest_answers() {
        let mut history = History::new();
        for _ in 0..History::CAPACITY {
            history.record(Outcome::Correct);
        }
        history.record(Outcome::Wrong);
        assert_eq!(
            history.outcomes().len(),
            History::CAPACITY,
            "length is capped"
        );
        assert_eq!(
            history.latest(),
            Some(Outcome::Wrong),
            "the newest answer is kept"
        );
    }

    #[test]
    fn a_stored_history_longer_than_the_capacity_is_cut_when_read() {
        let stored = format!("[{}\"wrong\"]", "\"correct\",".repeat(History::CAPACITY));
        let parsed: History = serde_json::from_str(&stored).expect("history parses");
        assert_eq!(
            parsed.outcomes().len(),
            History::CAPACITY,
            "only the newest answers are kept"
        );
        assert!(
            parsed.outcomes().iter().all(|outcome| outcome.is_correct()),
            "the oldest answer is the one dropped"
        );
    }

    #[test]
    fn an_eleventh_answer_does_not_count() {
        let ten_correct = history("cccccccccc");
        let mut after_long_ago_miss = history("w");
        for _ in 0..History::CAPACITY {
            after_long_ago_miss.record(Outcome::Correct);
        }
        assert_eq!(
            after_long_ago_miss, ten_correct,
            "the eleventh newest answer is forgotten"
        );
        let mut only_misses_remembered = history("c");
        for _ in 0..History::CAPACITY {
            only_misses_remembered.record(Outcome::Wrong);
        }
        assert_eq!(
            only_misses_remembered.level(),
            Some(Level::NEVER_CORRECT),
            "a card whose remembered answers are all misses ranks as never correct"
        );
    }

    #[test]
    fn level_stays_below_the_limit() {
        let all_correct: History = iter::repeat_n(Outcome::Correct, History::CAPACITY).collect();
        let level = all_correct.level().map(Level::get);
        assert_eq!(level, Some(1026), "level of a long all-correct history");
        assert!(level < Some(Level::LIMIT), "the limit is never reached");
    }

    #[test]
    fn a_newer_correct_answer_outweighs_all_older_ones() {
        let recovered = history("cwwwwwwww");
        let lapsed = history("wcccccccc");
        assert!(
            recovered.level() > lapsed.level(),
            "one fresh correct answer ranks above any number of older ones"
        );
    }

    #[test]
    fn a_newest_miss_ranks_below_every_newest_correct_answer() {
        let mut highest_after_a_miss = 0_u16;
        let mut lowest_after_a_correct_answer = u16::MAX;
        for length in 1..=History::CAPACITY {
            for answers in 0_u32..(1 << length) {
                let history: History = (0..length)
                    .map(|age| {
                        if (answers >> age) & 1 == 1 {
                            Outcome::Correct
                        } else {
                            Outcome::Wrong
                        }
                    })
                    .collect();
                let level = history.level().expect("an attempted card has a level");
                if history.latest() == Some(Outcome::Correct) {
                    lowest_after_a_correct_answer = lowest_after_a_correct_answer.min(level.get());
                } else {
                    highest_after_a_miss = highest_after_a_miss.max(level.get());
                }
            }
        }
        assert_eq!(
            highest_after_a_miss, 514,
            "the highest level of a history whose newest answer is a miss"
        );
        assert_eq!(
            lowest_after_a_correct_answer, 515,
            "the lowest level of a history whose newest answer is correct"
        );
    }

    #[test]
    fn miss_classification() {
        assert!(Outcome::Correct.is_correct(), "correct is correct");
        assert!(!Outcome::Correct.is_miss(), "correct is not a miss");
        assert!(Outcome::Wrong.is_miss(), "wrong is a miss");
        assert!(Outcome::DontKnow.is_miss(), "declining to guess is a miss");
    }

    #[test]
    fn outcomes_serialize_as_snake_case_names() {
        let json = serde_json::to_string(&history("c?w")).expect("history serializes");
        assert_eq!(
            json, r#"["correct","dont_know","wrong"]"#,
            "stored form of a history"
        );
        let parsed: History = serde_json::from_str(&json).expect("history parses");
        assert_eq!(parsed, history("c?w"), "a history survives a round trip");
    }
}
