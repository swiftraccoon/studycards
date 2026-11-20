//! Checks the card picker against a recorded study history.
//!
//! `fixtures/observed_batches.json` holds the history: every batch that was
//! built and every answer that was given, in order. Replaying it gives the
//! answers each card had received whenever a batch was built.
//!
//! The picker draws random numbers, so a batch cannot be compared with one
//! exact expectation. [`could_be_built`] instead decides whether a batch is one
//! the rules in `studycards::picker` allow. Every batch in the history must be
//! allowed, and so must every batch the picker builds.

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use serde::Deserialize;
    use studycards::mastery::{History, Level, Outcome};
    use studycards::picker::{self, BATCH_SIZE, Candidate};

    /// The recorded study history.
    #[derive(Deserialize)]
    struct Fixture {
        /// The cards of each topic.
        topics: BTreeMap<String, Vec<String>>,
        /// Batch builds and gradings in the order they happened.
        events: Vec<Event>,
    }

    /// One step of the recorded history.
    #[derive(Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum Event {
        /// A batch was built for a scope: a topic, or `""` for all cards.
        Build {
            batch: u32,
            scope: String,
            cards: Vec<String>,
        },
        /// A batch was completed with these answers: `c`, `w`, or `?`.
        Grade {
            batch: u32,
            answers: BTreeMap<String, char>,
        },
    }

    fn fixture() -> Fixture {
        serde_json::from_str(include_str!("fixtures/observed_batches.json"))
            .expect("the fixture is valid JSON in the expected shape")
    }

    /// Decides whether `picked` is a batch the picker's rules allow, given the
    /// level of every free card (`None` for an unattempted card) and the number
    /// of misses in the previous batch.
    ///
    /// An unattempted card is ranked at random, so it may stand anywhere in the
    /// ranking.
    fn could_be_built(
        picked: &[&str],
        free: &BTreeMap<&str, Option<Level>>,
        previous_misses: usize,
    ) -> bool {
        let chosen: BTreeSet<&str> = picked.iter().copied().collect();
        if chosen.len() != picked.len() || !picked.iter().all(|card| free.contains_key(card)) {
            return false;
        }
        if picked.len() != BATCH_SIZE.min(free.len()) {
            return false;
        }
        let never_correct: BTreeSet<&str> = free
            .iter()
            .filter(|&(_, &level)| level == Some(Level::NEVER_CORRECT))
            .map(|(&card, _)| card)
            .collect();
        if never_correct.len() > BATCH_SIZE {
            return picked.iter().all(|card| never_correct.contains(card));
        }
        if !never_correct.iter().all(|card| chosen.contains(card)) {
            return false;
        }
        let ranked: Vec<&str> = picked
            .iter()
            .copied()
            .filter(|card| !never_correct.contains(card))
            .collect();
        let passed_over: Vec<&str> = free
            .keys()
            .copied()
            .filter(|card| !chosen.contains(card) && !never_correct.contains(card))
            .collect();
        let weakest = BATCH_SIZE
            .saturating_sub(previous_misses)
            .saturating_sub(never_correct.len())
            .min(ranked.len());

        // Try every way of splitting the ranked picks into the `weakest` cards
        // taken first and the cards taken from the step above them.
        (0_u32..(1 << ranked.len())).any(|mask| {
            let in_first = |index: usize| (mask >> index) & 1 == 1;
            let first: Vec<&str> = ranked
                .iter()
                .copied()
                .enumerate()
                .filter(|&(index, _)| in_first(index))
                .map(|(_, card)| card)
                .collect();
            let second: Vec<&str> = ranked
                .iter()
                .copied()
                .enumerate()
                .filter(|&(index, _)| !in_first(index))
                .map(|(_, card)| card)
                .collect();
            first.len() == weakest && split_is_allowed(&first, &second, &passed_over, free)
        })
    }

    /// Checks one split of the ranked picks. Ranks are doubled levels so that
    /// an unattempted card can be placed between two levels.
    fn split_is_allowed(
        first: &[&str],
        second: &[&str],
        passed_over: &[&str],
        free: &BTreeMap<&str, Option<Level>>,
    ) -> bool {
        let known = |card: &str| {
            free.get(card)
                .copied()
                .flatten()
                .map(|level| i64::from(level.get()).saturating_mul(2))
        };
        // The most favourable ranks for unattempted cards: lowest of all when
        // taken first, just above the step when taken second, highest when
        // passed over.
        let step = first.iter().map(|card| known(card).unwrap_or(-1)).max();
        let just_above_step = step.map_or(-1, |step| step.saturating_add(1));
        let second_ranks: Vec<i64> = second
            .iter()
            .map(|card| known(card).unwrap_or(just_above_step))
            .collect();
        let passed_ranks: Vec<i64> = passed_over
            .iter()
            .map(|card| known(card).unwrap_or(i64::MAX))
            .collect();
        let highest_second = second_ranks.iter().copied().max();

        step.map_or_else(
            || {
                highest_second
                    .is_none_or(|highest| passed_ranks.iter().all(|&rank| rank >= highest))
            },
            |step| {
                // Nothing left behind ranks below the first group.
                let first_is_weakest = second_ranks
                    .iter()
                    .chain(&passed_ranks)
                    .all(|&rank| rank >= step);
                // The second group is the lowest group strictly above the
                // step. Cards tied with the step are taken only when every
                // card above the step has been taken.
                let ties_taken = second_ranks.contains(&step);
                let second_is_next = if ties_taken {
                    passed_ranks.iter().all(|&rank| rank == step)
                } else {
                    highest_second.is_none_or(|highest| {
                        passed_ranks
                            .iter()
                            .all(|&rank| rank == step || rank >= highest)
                    })
                };
                first_is_weakest && second_is_next
            },
        )
    }

    /// The study state reconstructed while replaying the fixture.
    #[derive(Default)]
    struct Replay<'fixture> {
        /// Answers of each card, newest first.
        histories: BTreeMap<&'fixture str, History>,
        /// Batches that were built and are not completed: scope and cards.
        open: BTreeMap<u32, (&'fixture str, Vec<&'fixture str>)>,
        /// Misses in the batch completed most recently, per scope.
        last_misses: BTreeMap<&'fixture str, usize>,
    }

    impl<'fixture> Replay<'fixture> {
        /// Returns the level of every card of `scope` that is not in an open
        /// batch.
        ///
        /// In the history a card waits in one batch at a time, whichever scope
        /// that batch was built for.
        fn free_cards(
            &self,
            fixture: &'fixture Fixture,
            scope: &str,
        ) -> BTreeMap<&'fixture str, Option<Level>> {
            let in_open: BTreeSet<&str> = self
                .open
                .values()
                .flat_map(|(_, cards)| cards.iter().copied())
                .collect();
            fixture
                .topics
                .iter()
                .filter(|(topic, _)| scope.is_empty() || topic.as_str() == scope)
                .flat_map(|(_, cards)| cards.iter().map(String::as_str))
                .filter(|card| !in_open.contains(card))
                .map(|card| (card, self.histories.get(card).and_then(History::level)))
                .collect()
        }

        /// Records the answers of a completed batch.
        fn grade(&mut self, batch: u32, answers: &'fixture BTreeMap<String, char>) {
            let (scope, cards) = self
                .open
                .remove(&batch)
                .expect("a graded batch was built first");
            let mut misses: usize = 0;
            for card in cards {
                let outcome = match answers.get(card) {
                    Some('c') => Outcome::Correct,
                    Some('w') => Outcome::Wrong,
                    Some('?') => Outcome::DontKnow,
                    other => panic!("card {card} of batch {batch} has answer {other:?}"),
                };
                misses = misses.saturating_add(usize::from(outcome.is_miss()));
                self.histories.entry(card).or_default().record(outcome);
            }
            self.last_misses.insert(scope, misses);
        }
    }

    #[test]
    fn every_observed_batch_is_one_the_picker_could_build() {
        let fixture = fixture();
        let mut replay = Replay::default();
        let mut builds: usize = 0;
        let mut needing_the_step: usize = 0;
        for event in &fixture.events {
            match event {
                Event::Grade { batch, answers } => replay.grade(*batch, answers),
                Event::Build {
                    batch,
                    scope,
                    cards,
                } => {
                    let free = replay.free_cards(&fixture, scope);
                    let picked: Vec<&str> = cards.iter().map(String::as_str).collect();
                    let misses = replay.last_misses.get(scope.as_str()).copied().unwrap_or(0);
                    assert!(
                        could_be_built(&picked, &free, misses),
                        "batch {batch} (scope {scope:?}, {misses} previous misses) is not \
                         allowed: {picked:?}"
                    );
                    builds = builds.saturating_add(1);
                    needing_the_step = needing_the_step
                        .saturating_add(usize::from(!could_be_built(&picked, &free, 0)));
                    replay.open.insert(*batch, (scope.as_str(), picked));
                }
            }
        }
        assert_eq!(builds, 110, "every recorded build was checked");
        assert_eq!(
            needing_the_step, 7,
            "builds that are only allowed because of the step after a miss"
        );
    }

    #[test]
    fn the_check_rejects_batches_that_break_the_rules() {
        let level = |pattern: &str| {
            let history: History = pattern
                .chars()
                .map(|symbol| {
                    if symbol == 'c' {
                        Outcome::Correct
                    } else {
                        Outcome::Wrong
                    }
                })
                .collect();
            history.level()
        };
        let free: BTreeMap<&str, Option<Level>> = [
            ("missed", level("w")),
            ("weak-1", level("cw")),
            ("weak-2", level("cw")),
            ("mid-1", level("c")),
            ("mid-2", level("c")),
            ("mid-3", level("c")),
            ("strong", level("cc")),
            ("stronger", level("ccc")),
        ]
        .into_iter()
        .collect();
        let allowed = |picked: &[&str], misses: usize| could_be_built(picked, &free, misses);

        assert!(
            allowed(&["missed", "weak-1", "weak-2", "mid-1", "mid-2"], 0),
            "the weakest five"
        );
        assert!(
            allowed(&["missed", "weak-1", "weak-2", "mid-3", "strong"], 1),
            "one step up after a miss"
        );
        assert!(
            !allowed(&["missed", "weak-1", "weak-2", "mid-1", "strong"], 0),
            "no step without a miss"
        );
        assert!(
            !allowed(&["missed", "weak-1", "weak-2", "mid-1", "mid-2"], 1),
            "the step is required after a miss"
        );
        assert!(
            !allowed(&["missed", "weak-1", "weak-2", "mid-1", "stronger"], 1),
            "the step is the nearest one"
        );
        assert!(
            !allowed(&["weak-1", "weak-2", "mid-1", "mid-2", "mid-3"], 0),
            "a never-correct card is left out"
        );
        assert!(
            !allowed(&["missed", "weak-1", "mid-1", "mid-2", "mid-3"], 0),
            "a weaker card is passed over"
        );
        assert!(
            !allowed(&["missed", "weak-1", "weak-2", "mid-1"], 0),
            "the batch is short"
        );
        assert!(
            !allowed(&["missed", "weak-1", "weak-1", "mid-1", "mid-2"], 0),
            "a card is repeated"
        );
        assert!(
            !allowed(&["missed", "weak-1", "weak-2", "mid-1", "other"], 0),
            "a card is not free"
        );
        // Two misses: three weakest, then two from strictly above the third.
        assert!(
            allowed(&["missed", "weak-1", "weak-2", "mid-1", "mid-2"], 2),
            "two from the step above"
        );
        assert!(
            !allowed(&["missed", "mid-1", "mid-2", "mid-3", "strong"], 3),
            "the step skips only ties, never weaker cards"
        );
        assert!(
            !allowed(&["missed", "weak-1", "weak-2", "mid-1", "mid-2"], 3),
            "tied cards are not both taken while cards rank above the step"
        );
        assert!(
            allowed(&["missed", "weak-2", "mid-1", "mid-2", "mid-3"], 3),
            "a tied card waits after three misses"
        );
    }

    #[test]
    fn every_batch_the_picker_builds_is_allowed() {
        const PATTERNS: [&str; 12] = [
            "w", "ww", "c", "cw", "cww", "cc", "ccw", "wc", "wcc", "ccc", "cccc", "wccc",
        ];
        let mut random = fastrand::Rng::with_seed(0x5eed);
        for round in 0_u32..2000 {
            let size = random.usize(0..40);
            let names: Vec<String> = (0..size).map(|index| format!("card-{index}")).collect();
            let mut free = BTreeMap::new();
            for name in &names {
                let level = if random.u8(0..4) == 0 {
                    None
                } else {
                    let pattern = PATTERNS[random.usize(0..PATTERNS.len())];
                    let history: History = pattern
                        .chars()
                        .map(|symbol| {
                            if symbol == 'c' {
                                Outcome::Correct
                            } else {
                                Outcome::Wrong
                            }
                        })
                        .collect();
                    history.level()
                };
                free.insert(name.as_str(), level);
            }
            let misses = random.usize(0..=BATCH_SIZE);
            let candidates: Vec<Candidate<&str>> = free
                .iter()
                .map(|(&card, &level)| Candidate { card, level })
                .collect();
            let batch = picker::build_batch(candidates, misses, &mut random);
            assert!(
                could_be_built(&batch, &free, misses),
                "round {round}: {batch:?} is not allowed for {free:?} after {misses} misses"
            );
        }
    }
}
