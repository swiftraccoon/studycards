//! Choosing the cards of the next batch.
//!
//! Cards are studied in batches of [`BATCH_SIZE`]. [`build_batch`] chooses a
//! batch from the cards that are free to be asked:
//!
//! 1. Every attempted card whose history holds no correct answer is included.
//!    When there are more of them than a batch holds, a random selection is
//!    made.
//! 2. The remaining cards are ranked by [`Level`], weakest first. A card that
//!    has not been attempted gets a random level each time, so such cards fill
//!    the batch while many remain and mix with weak cards as they run out.
//! 3. The weakest ranked cards fill the batch, except that one place per miss
//!    in the previous batch goes to the weakest cards that rank strictly above
//!    the others chosen. After a batch with misses, the next one therefore
//!    reaches one step further up instead of repeating only the weakest cards.
//!
//! `docs/picker.md` in the repository states the rules in full, with the
//! scores they use and their effects.

use crate::mastery::Level;

/// The number of cards in a full batch.
pub const BATCH_SIZE: usize = 5;

/// A source of random numbers.
pub trait RandomSource {
    /// Returns a number uniformly distributed over `0..bound`, or 0 when
    /// `bound` is 0.
    fn below(&mut self, bound: u32) -> u32;
}

impl RandomSource for fastrand::Rng {
    fn below(&mut self, bound: u32) -> u32 {
        if bound == 0 { 0 } else { self.u32(0..bound) }
    }
}

/// Puts `items` in a uniformly random order.
pub fn shuffle<T>(items: &mut [T], random: &mut dyn RandomSource) {
    for upper in (1..items.len()).rev() {
        let bound = u32::try_from(upper).map_or(u32::MAX, |upper| upper.saturating_add(1));
        // A source that returned a number outside its bound would send `swap`
        // past the end of the slice.
        let other = usize::try_from(random.below(bound)).map_or(0, |other| other.min(upper));
        items.swap(upper, other);
    }
}

/// A card that is free to be asked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Candidate<T> {
    /// The handle returned for a chosen card.
    pub card: T,
    /// The card's level, or `None` when it has not been attempted.
    pub level: Option<Level>,
}

/// Chooses up to [`BATCH_SIZE`] cards from `candidates` and returns them in
/// the order they are to be asked.
///
/// `previous_misses` is the number of cards missed in the batch completed most
/// recently; pass 0 when no batch has been completed. See the [module
/// documentation](self) for the rules. The batch is shorter than
/// [`BATCH_SIZE`] only when there are fewer candidates.
///
/// # Examples
///
/// A card that has only been missed is always chosen, and the weakest of the
/// other cards fill the remaining places.
///
/// ```
/// use studycards::mastery::{History, Outcome};
/// use studycards::picker::{self, Candidate};
///
/// let candidate = |card: &'static str, answers: &[Outcome]| {
///     let history: History = answers.iter().copied().collect();
///     Candidate { card, level: history.level() }
/// };
/// let candidates = vec![
///     candidate("missed", &[Outcome::Wrong]),
///     candidate("shaky", &[Outcome::Correct, Outcome::Wrong]),
///     candidate("known", &[Outcome::Correct, Outcome::Correct]),
///     candidate("a", &[Outcome::Correct]),
///     candidate("b", &[Outcome::Correct]),
///     candidate("c", &[Outcome::Correct]),
/// ];
///
/// let mut random = fastrand::Rng::with_seed(1);
/// let mut batch = picker::build_batch(candidates, 0, &mut random);
/// batch.sort_unstable();
/// assert_eq!(batch, ["a", "b", "c", "missed", "shaky"]);
/// ```
pub fn build_batch<T>(
    candidates: Vec<Candidate<T>>,
    previous_misses: usize,
    random: &mut dyn RandomSource,
) -> Vec<T> {
    let (mut never_correct, unranked): (Vec<_>, Vec<_>) = candidates
        .into_iter()
        .partition(|candidate| candidate.level == Some(Level::NEVER_CORRECT));

    shuffle(&mut never_correct, random);
    let mut batch: Vec<T> = never_correct
        .into_iter()
        .take(BATCH_SIZE)
        .map(|candidate| candidate.card)
        .collect();

    let mut ranked: Vec<(u16, T)> = unranked
        .into_iter()
        .map(|candidate| (rank(candidate.level, random), candidate.card))
        .collect();
    // Shuffling before a stable sort leaves equal ranks in random order.
    shuffle(&mut ranked, random);
    ranked.sort_by_key(|&(rank, _)| rank);

    let places = BATCH_SIZE.saturating_sub(batch.len());
    let weakest = BATCH_SIZE
        .saturating_sub(previous_misses)
        .saturating_sub(batch.len())
        .min(places);
    let mut ranked = ranked.into_iter();
    let mut step_from = None;
    for (rank, card) in ranked.by_ref().take(weakest) {
        step_from = Some(rank);
        batch.push(card);
    }
    let mut skipped = Vec::new();
    for (rank, card) in ranked {
        if batch.len() >= BATCH_SIZE {
            break;
        }
        if step_from.is_some_and(|step_from| rank <= step_from) {
            skipped.push(card);
        } else {
            batch.push(card);
        }
    }
    // With too few cards above the step, the batch is completed from the ties
    // that were passed over.
    let missing = BATCH_SIZE.saturating_sub(batch.len());
    batch.extend(skipped.into_iter().take(missing));

    shuffle(&mut batch, random);
    batch
}

/// Returns the rank used to order a candidate: its level, or a random level
/// for a card that has not been attempted.
fn rank(level: Option<Level>, random: &mut dyn RandomSource) -> u16 {
    level.map_or_else(
        || u16::try_from(random.below(u32::from(Level::LIMIT))).unwrap_or(u16::MAX),
        Level::get,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{BATCH_SIZE, Candidate, RandomSource, build_batch, shuffle};
    use crate::mastery::{History, Level, Outcome};

    /// Returns the level of a history written newest first with `c` and `w`.
    fn level(pattern: &str) -> Level {
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
        history.level().expect("a non-empty history has a level")
    }

    fn seen(card: &'static str, pattern: &str) -> Candidate<&'static str> {
        Candidate {
            card,
            level: Some(level(pattern)),
        }
    }

    fn unseen(card: &'static str) -> Candidate<&'static str> {
        Candidate { card, level: None }
    }

    fn sorted(mut batch: Vec<&'static str>) -> Vec<&'static str> {
        batch.sort_unstable();
        batch
    }

    fn rng(seed: u64) -> fastrand::Rng {
        fastrand::Rng::with_seed(seed)
    }

    /// Always returns the same number, clamped below the bound.
    struct Constant(u32);

    impl RandomSource for Constant {
        fn below(&mut self, bound: u32) -> u32 {
            self.0.min(bound.saturating_sub(1))
        }
    }

    #[test]
    fn takes_the_weakest_cards() {
        let candidates = vec![
            seen("strong", "cccc"),
            seen("weak", "cw"),
            seen("weaker", "cww"),
            seen("weakest", "cwww"),
            seen("medium", "cc"),
            seen("solid", "ccc"),
            seen("fresh", "c"),
        ];
        for seed in 0..20 {
            let batch = build_batch(candidates.clone(), 0, &mut rng(seed));
            assert_eq!(
                sorted(batch),
                ["fresh", "medium", "weak", "weaker", "weakest"],
                "the five lowest levels are chosen (seed {seed})"
            );
        }
    }

    #[test]
    fn ties_are_broken_at_random() {
        let candidates: Vec<_> = ["a", "b", "c", "d", "e", "f", "g", "h"]
            .into_iter()
            .map(|card| seen(card, "c"))
            .collect();
        let mut selections = BTreeSet::new();
        for seed in 0..50 {
            let batch = build_batch(candidates.clone(), 0, &mut rng(seed));
            assert_eq!(
                batch.len(),
                BATCH_SIZE,
                "a full batch is chosen (seed {seed})"
            );
            selections.insert(sorted(batch));
        }
        assert!(
            selections.len() > 10,
            "equal cards are chosen in many combinations: {}",
            selections.len()
        );
    }

    #[test]
    fn never_correct_cards_are_always_included() {
        let mut candidates = vec![
            seen("missed-1", "w"),
            seen("missed-2", "ww"),
            seen("missed-3", "www"),
        ];
        candidates.extend(["n1", "n2", "n3", "n4", "n5", "n6"].into_iter().map(unseen));
        for seed in 0..50 {
            for misses in 0..=BATCH_SIZE {
                let batch = build_batch(candidates.clone(), misses, &mut rng(seed));
                assert_eq!(batch.len(), BATCH_SIZE, "a full batch is chosen");
                for card in ["missed-1", "missed-2", "missed-3"] {
                    assert!(
                        batch.contains(&card),
                        "{card} is included (seed {seed}, misses {misses})"
                    );
                }
            }
        }
    }

    #[test]
    fn an_unseen_card_never_displaces_a_never_correct_card() {
        // The random source gives every unseen card the same rank as a card
        // that has never been answered correctly.
        let mut candidates: Vec<_> = ["m1", "m2", "m3", "m4", "m5"]
            .into_iter()
            .map(|card| seen(card, "w"))
            .collect();
        candidates.extend(["n1", "n2", "n3"].into_iter().map(unseen));
        let batch = build_batch(
            candidates,
            0,
            &mut Constant(u32::from(Level::NEVER_CORRECT.get())),
        );
        assert_eq!(
            sorted(batch),
            ["m1", "m2", "m3", "m4", "m5"],
            "all never-correct cards are chosen ahead of unseen cards"
        );
    }

    #[test]
    fn a_miss_sends_the_last_place_one_step_up() {
        // One card was missed and ranks lowest; eight cards tie above it.
        let mut candidates = vec![seen("lapsed", "wccc")];
        candidates
            .extend(["t1", "t2", "t3", "t4", "t5", "t6", "t7", "t8"].map(|card| seen(card, "ccc")));
        candidates.push(seen("next", "cccwc"));
        candidates.push(seen("far", "cccc"));
        for seed in 0..50 {
            let batch = build_batch(candidates.clone(), 1, &mut rng(seed));
            assert!(
                batch.contains(&"lapsed"),
                "the missed card returns (seed {seed})"
            );
            assert!(
                batch.contains(&"next"),
                "the last place goes one step up (seed {seed})"
            );
            assert!(
                !batch.contains(&"far"),
                "only the nearest step is taken (seed {seed})"
            );
            let ties = batch.iter().filter(|card| card.starts_with('t')).count();
            assert_eq!(
                ties, 3,
                "three of the tied cards fill the rest (seed {seed})"
            );
        }
        for seed in 0..50 {
            let batch = build_batch(candidates.clone(), 0, &mut rng(seed));
            assert!(
                !batch.contains(&"next"),
                "without a miss the weakest five are chosen (seed {seed})"
            );
        }
    }

    #[test]
    fn tied_lapsed_cards_are_split_by_the_step() {
        // Three misses: two places for the weakest, three for the step above.
        // The two tied lapsed cards cannot both be below the step.
        let mut candidates = vec![
            seen("lowest", "wcwc"),
            seen("tied-1", "wccc"),
            seen("tied-2", "wccc"),
        ];
        candidates.extend(["u1", "u2", "u3", "u4", "u5"].map(|card| seen(card, "ccccw")));
        candidates.extend(["v1", "v2", "v3"].map(|card| seen(card, "cccc")));
        for seed in 0..50 {
            let batch = build_batch(candidates.clone(), 3, &mut rng(seed));
            assert!(
                batch.contains(&"lowest"),
                "the weakest card is chosen (seed {seed})"
            );
            let tied = batch.iter().filter(|card| card.starts_with("tied")).count();
            assert_eq!(
                tied, 1,
                "one of the tied cards waits for the next batch (seed {seed})"
            );
            let above = batch.iter().filter(|card| card.starts_with('u')).count();
            assert_eq!(above, 3, "the step above fills three places (seed {seed})");
        }
    }

    #[test]
    fn unseen_cards_fill_the_batch_while_many_remain() {
        let mut candidates: Vec<_> = (0_u8..200).map(|_| unseen("new")).collect();
        candidates.extend(["k1", "k2", "k3", "k4", "k5", "k6"].map(|card| seen(card, "c")));
        let mut reviews = 0;
        for seed in 0..100 {
            let batch = build_batch(candidates.clone(), 0, &mut rng(seed));
            reviews += batch.iter().filter(|&&card| card != "new").count();
        }
        assert_eq!(
            reviews, 0,
            "with 200 unseen cards no known card is reviewed"
        );
    }

    #[test]
    fn a_few_unseen_cards_mix_with_weak_cards() {
        let mut candidates: Vec<_> = (0_u8..4).map(|_| unseen("new")).collect();
        candidates.extend(["k1", "k2", "k3", "k4", "k5", "k6"].map(|card| seen(card, "c")));
        let mut new_cards = 0;
        let rounds = 400;
        for seed in 0..rounds {
            let batch = build_batch(candidates.clone(), 0, &mut rng(seed));
            new_cards += batch.iter().filter(|&&card| card == "new").count();
        }
        // A once-correct card has level 537 of 1027, so each unseen card ranks
        // below it about half the time: about two of the four per batch.
        assert!(
            (700..=900).contains(&new_cards),
            "about half of the unseen cards are chosen: {new_cards} of {}",
            rounds * 4
        );
    }

    #[test]
    fn short_supply_gives_a_short_batch() {
        let candidates = vec![seen("a", "c"), unseen("b"), seen("c", "w")];
        let batch = build_batch(candidates, 2, &mut rng(1));
        assert_eq!(
            sorted(batch),
            ["a", "b", "c"],
            "every candidate is chosen once"
        );
        let none: Vec<Candidate<u8>> = Vec::new();
        assert!(
            build_batch(none, 0, &mut rng(1)).is_empty(),
            "no candidates, no batch"
        );
    }

    #[test]
    fn passed_over_ties_complete_a_batch_when_nothing_ranks_higher() {
        let candidates: Vec<_> = ["a", "b", "c", "d", "e", "f"]
            .into_iter()
            .map(|card| seen(card, "c"))
            .collect();
        for misses in 0..=BATCH_SIZE {
            let batch = build_batch(candidates.clone(), misses, &mut rng(7));
            assert_eq!(
                batch.len(),
                BATCH_SIZE,
                "the batch is full after {misses} misses"
            );
        }
    }

    #[test]
    fn more_than_a_batch_of_never_correct_cards_is_cut_to_size() {
        let candidates: Vec<_> = ["a", "b", "c", "d", "e", "f", "g"]
            .into_iter()
            .map(|card| seen(card, "w"))
            .collect();
        let batch = build_batch(candidates, 0, &mut rng(3));
        assert_eq!(batch.len(), BATCH_SIZE, "at most a full batch is chosen");
    }

    #[test]
    fn shuffle_keeps_every_item() {
        let ordered: Vec<u32> = (0..10).collect();
        let mut items = ordered.clone();
        shuffle(&mut items, &mut rng(42));
        assert_ne!(items, ordered, "the order changes");
        items.sort_unstable();
        assert_eq!(items, ordered, "no item is lost or repeated");
        let mut single = [1_u8];
        shuffle(&mut single, &mut rng(42));
        assert_eq!(single, [1_u8], "a single item stays in place");
        let mut empty: [u8; 0] = [];
        shuffle(&mut empty, &mut Constant(0));
        assert_eq!(empty, [0_u8; 0], "an empty slice is accepted");
    }

    #[test]
    fn a_source_that_ignores_its_bound_causes_no_panic() {
        /// Returns the largest number, whatever the bound.
        struct Unbounded;

        impl RandomSource for Unbounded {
            fn below(&mut self, _bound: u32) -> u32 {
                u32::MAX
            }
        }

        let ordered: Vec<u32> = (0..10).collect();
        let mut items = ordered.clone();
        shuffle(&mut items, &mut Unbounded);
        items.sort_unstable();
        assert_eq!(items, ordered, "no item is lost or repeated");
        let candidates = vec![seen("a", "c"), unseen("b"), seen("c", "w"), unseen("d")];
        assert_eq!(
            sorted(build_batch(candidates, 1, &mut Unbounded)),
            ["a", "b", "c", "d"],
            "every candidate is chosen once"
        );
    }

    #[test]
    fn fastrand_source_respects_the_bound() {
        let mut source = rng(9);
        assert_eq!(source.below(0), 0, "a zero bound yields zero");
        assert_eq!(source.below(1), 0, "a bound of one yields zero");
        assert!(
            (0_u8..100).all(|_| source.below(7) < 7),
            "values stay below the bound"
        );
    }
}
