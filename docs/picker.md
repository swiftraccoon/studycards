# How cards are chosen

Study cards asks cards in batches of five and decides the content of each batch
from the answer history of every card. This document states the scores and the
rules. `src/mastery.rs` computes the scores, `src/picker.rs` builds a batch,
and `src/study.rs` keeps the queue of batches.

## Scores

Each card has a history: its ten newest answers, newest first. An answer is
correct, wrong, or "I don't know"; the last two are both misses. A card is
answered once each time it is asked. What the learner chooses after a wrong
answer, while the card is still shown, is not an answer and leaves the history
as it is. Two scores are computed from the history.

**Aptitude** is the score shown to the learner. The four newest answers weigh
50%, 25%, 15%, and 10%. A correct answer earns its weight and a miss earns
nothing. A card with fewer than four answers earns a quarter of the weight of
each answer it lacks. A card answered correctly once therefore scores 62.5%,
and four correct answers in a row score 100%.

**Level** ranks cards when a batch is chosen. An attempted card whose history
holds no correct answer has level 2. For every other attempted card:

```text
level = 1 + (512 for a correct newest answer, 256 for the one before, 128, ...
             down to 1 for the tenth)
          + 48 / 2^min(n, 5)        n = number of answers in the history
```

rounded to the nearest whole number, halves upward. The last term is 24 after
one answer and 1.5 from the fifth answer on. One correct answer gives level
537, a miss followed by a correct answer 525, and three correct answers 903.
Ten correct answers in a row give 1026, the highest level. A correct answer
outweighs all the answers before it together, so the level orders cards mainly
by their current streak: every card whose newest answer is a miss ranks below
every card whose newest answer is correct.

The study page shows two percentages for the cards in view, each rounded to a
whole number with halves upward. **Seen** is the share of cards attempted.
**Aptitude** is the sum of the cards' aptitudes divided by the number of cards,
so an unattempted card counts as zero.

## Batches

A study session has a scope: the whole set or one topic. Each scope has its own
queue of up to two batches, the one being answered and the next. When a batch
is completed, another is chosen. A batch is built from the cards of the scope
that are not waiting in a batch of that scope:

1. Every attempted card whose history holds no correct answer is included.
   When there are more than five, five of them are chosen at random.
2. The other cards are ranked by level, weakest first, with equal levels in
   random order. An unattempted card receives a random level below 1027 each
   time a batch is built.
3. With `m` misses in the batch just completed, the batch takes the weakest
   ranked cards until `5 - m` places are filled, counting the cards from step 1.
   The remaining places go to the weakest cards that rank strictly above the
   last card taken. When too few cards rank above it, the cards tied with it
   fill the batch. When step 1 alone fills `5 - m` places, the remaining places
   go to the weakest ranked cards. For the first two batches of a scope, `m`
   is 0.
4. The cards are asked in random order, and the answers of each card are shown
   in an order drawn when the batch is built.

A batch is shorter than five cards when fewer are free, and a scope of five
cards or fewer has a single batch waiting.

The queues of different scopes are independent, so a topic session reaches
every card of its topic. A card can wait in a batch for the whole set and in a
batch for its topic at the same time.

Elapsed time plays no part. A card's rank depends on its answers alone, however
long ago they were given.

These rules have the following effects.

- A card missed before it has ever been answered correctly returns in the batch
  after the next one: the next batch was chosen before the miss, and step 1
  puts the card in the one after, unless more than five such cards are free.
  With full batches that is six to fourteen cards later.
- A miss on a card that has been answered correctly lowers its level below that
  of every card whose newest answer is correct, so the card is ranked ahead of
  them all.
- An unattempted card ranks below a card answered correctly once about half of
  the time. While a scope has many unattempted cards, almost all of the weakest
  ranks are theirs, and they fill the places that step 1 leaves free. When few
  remain, they mix with the weakest attempted cards. When none remain, every
  batch reviews the weakest cards.
- A correct answer does not retire a card. The card returns when its level is
  among the lowest of the cards that are free.
- After a batch with misses, the step in rule 3 brings in cards from one level
  higher instead of filling the batch with tied weakest cards. Two missed cards
  with equal levels can be separated by the step; the one left out is the
  weakest card when the following batch is built.
