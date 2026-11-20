//! The pages of the application.

use super::element::Element;
use crate::deck::{Card, CardDraft, CardId, SetId, StudySet, Topic, TopicId};
use crate::deck::{MAX_ANSWER_LEN, MAX_NAME_LEN, MAX_TEXT_LEN, MAX_WRONG_ANSWERS};
use crate::study::{Progress, Review, Scope, Tally, View};

/// The name shown in the page header and titles.
const APP_NAME: &str = "Study cards";
/// The letters that label answers, in the order they are shown.
const LETTERS: [&str; MAX_WRONG_ANSWERS + 1] = ["A", "B", "C", "D", "E", "F"];

/// Returns the path of a set's overview page.
pub(super) fn set_path(set: &SetId) -> String {
    format!("/sets/{set}")
}

/// Returns the path of the study page for a scope.
pub(super) fn study_path(set: &SetId, scope: &Scope) -> String {
    match scope {
        Scope::All => format!("/sets/{set}/study"),
        Scope::Topic(topic) => format!("/sets/{set}/study/{topic}"),
    }
}

/// Returns the path of a set's editor.
pub(super) fn edit_path(set: &SetId) -> String {
    format!("/sets/{set}/edit")
}

/// Returns the path of the form that edits a card.
pub(super) fn card_path(set: &SetId, card: &CardId) -> String {
    format!("/sets/{set}/edit/cards/{card}")
}

/// Returns the path of the form that adds a card to a topic.
pub(super) fn new_card_path(set: &SetId, topic: &TopicId) -> String {
    format!("/sets/{set}/edit/topics/{topic}/cards/new")
}

/// Wraps page content in the document shell.
fn page(title: &str, trail: &[(String, &str)], main: Element) -> String {
    let head = Element::new("head")
        .child(Element::new("meta").attr("charset", "utf-8"))
        .child(
            Element::new("meta")
                .attr("name", "viewport")
                .attr("content", "width=device-width, initial-scale=1"),
        )
        .child(Element::new("title").text(&format!("{title} · {APP_NAME}")))
        .child(
            Element::new("link")
                .attr("rel", "stylesheet")
                .attr("href", "/assets/app.css"),
        )
        .child(
            Element::new("link")
                .attr("rel", "icon")
                .attr("type", "image/svg+xml")
                .attr("href", "/assets/icon.svg"),
        )
        .child(
            Element::new("script")
                .attr("type", "module")
                .attr("src", "/assets/app.js"),
        );
    let header = Element::new("header")
        .class("site")
        .child(link("/", APP_NAME).class("brand"))
        .children(
            trail
                .iter()
                .map(|(href, text)| link(href, text).class("crumb")),
        );
    Element::new("html")
        .attr("lang", "en")
        .child(head)
        .child(Element::new("body").child(header).child(main))
        .to_document()
}

/// Creates a link.
fn link(href: &str, text: &str) -> Element {
    Element::new("a").attr("href", href).text(text)
}

/// Creates a link styled as a button.
fn button_link(href: &str, text: &str) -> Element {
    link(href, text).class("button")
}

/// Creates a form that posts to `action`.
fn post_form(action: &str) -> Element {
    Element::new("form")
        .attr("method", "post")
        .attr("action", action)
}

/// Creates a submit button.
fn submit(text: &str) -> Element {
    Element::new("button").attr("type", "submit").text(text)
}

/// Whether a form can be sent while one of its fields is empty.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Entry {
    /// The field must be filled in.
    Required,
    /// The field may stay empty.
    Optional,
}

/// Creates a labelled single-line text field.
fn text_field(label: &str, name: &'static str, value: &str, max: usize, entry: Entry) -> Element {
    Element::new("label")
        .class("field")
        .child(Element::new("span").text(label))
        .child(
            Element::new("input")
                .attr("type", "text")
                .attr("name", name)
                .attr("value", value)
                .attr("maxlength", &max.to_string())
                .attr("autocomplete", "off")
                .flag("required", entry == Entry::Required),
        )
}

/// Creates a labelled multi-line text field.
fn text_area(label: &str, name: &'static str, value: &str, entry: Entry) -> Element {
    Element::new("label")
        .class("field")
        .child(Element::new("span").text(label))
        .child(
            Element::new("textarea")
                .attr("name", name)
                .attr("rows", "4")
                .attr("maxlength", &MAX_TEXT_LEN.to_string())
                .flag("required", entry == Entry::Required)
                .text(value),
        )
}

/// Creates a checkbox that must be ticked before a destructive form is sent.
fn confirmation(text: &str) -> Element {
    Element::new("label")
        .class("confirm")
        .child(
            Element::new("input")
                .attr("type", "checkbox")
                .attr("name", "confirm")
                .attr("value", "yes")
                .flag("required", true),
        )
        .text(&format!(" {text}"))
}

/// Renders text as paragraphs: blank lines separate paragraphs and single
/// line breaks are kept.
fn paragraphs(text: &str) -> Vec<Element> {
    text.split("\n\n")
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty())
        .map(|paragraph| {
            let mut lines = paragraph.lines();
            let first = Element::new("p").text(lines.next().unwrap_or_default());
            lines.fold(first, |element, line| {
                element.child(Element::new("br")).text(line)
            })
        })
        .collect()
}

/// Formats seen and aptitude percentages for a summary line.
fn summary(tally: Tally) -> String {
    let cards = if tally.cards == 1 { "card" } else { "cards" };
    format!(
        "{} {cards} · {}% seen · {}% aptitude",
        tally.cards,
        tally.seen_percent(),
        tally.aptitude_percent()
    )
}

/// Totals the progress on every card of a set.
pub(super) fn set_tally(set: &StudySet, progress: &Progress) -> Tally {
    set.topics()
        .iter()
        .map(|topic| progress.tally(topic.cards()))
        .sum()
}

/// A study set as listed on the home page.
#[derive(Clone, Debug)]
pub(super) struct SetSummary {
    /// The set's identifier.
    pub(super) id: SetId,
    /// Whether the set can be used.
    pub(super) state: SetState,
}

/// Whether a listed study set can be used.
#[derive(Clone, Debug)]
pub(super) enum SetState {
    /// The set and its progress were read.
    Usable {
        /// The set's title.
        title: String,
        /// The progress on every card of the set.
        tally: Tally,
    },
    /// The set or its progress could not be read.
    Broken {
        /// Why, in words for the learner.
        problem: String,
    },
}

/// Renders the home page: the list of sets and the form that creates one.
pub(super) fn home(sets: &[SetSummary], unusable_files: &[String]) -> String {
    let items = sets.iter().map(|set| match &set.state {
        SetState::Usable { title, tally } => Element::new("li")
            .class("set")
            .child(Element::new("h2").child(link(&set_path(&set.id), title)))
            .child(Element::new("p").class("summary").text(&summary(*tally)))
            .child(
                Element::new("p")
                    .class("actions")
                    .child(button_link(&study_path(&set.id, &Scope::All), "Study"))
                    .child(button_link(&edit_path(&set.id), "Edit").class("button quiet")),
            ),
        SetState::Broken { problem } => Element::new("li")
            .class("set broken")
            .child(Element::new("h2").text(set.id.as_str()))
            .child(Element::new("p").class("error").text(problem)),
    });
    let list = if sets.is_empty() {
        Element::new("p")
            .class("empty")
            .text("No study sets yet. Create one below.")
    } else {
        Element::new("ul").class("sets").children(items)
    };
    let unusable = (!unusable_files.is_empty()).then(|| {
        Element::new("p").class("error").text(&format!(
            "These files are skipped because their names cannot be used as identifiers \
             (use lowercase letters, digits, '-' and '_'): {}",
            unusable_files.join(", ")
        ))
    });
    let create = post_form("/sets")
        .class("create")
        .child(text_field(
            "Title of a new set",
            "title",
            "",
            MAX_NAME_LEN,
            Entry::Required,
        ))
        .child(submit("Create set"));
    page(
        "Sets",
        &[],
        Element::new("main")
            .child(Element::new("h1").text("Study sets"))
            .child(list)
            .children(unusable)
            .child(create),
    )
}

/// Renders the overview of one set.
pub(super) fn overview(set: &StudySet, progress: &Progress) -> String {
    let id = set.id();
    let topics = set.topics().iter().map(|topic| {
        let tally = progress.tally(topic.cards());
        Element::new("li")
            .child(link(
                &study_path(id, &Scope::Topic(topic.id().clone())),
                topic.name(),
            ))
            .child(
                Element::new("span")
                    .class("summary")
                    .text(&format!(" {}", summary(tally))),
            )
    });
    let main = Element::new("main")
        .child(Element::new("h1").text(set.title()))
        .child(
            Element::new("p")
                .class("summary")
                .text(&summary(set_tally(set, progress))),
        )
        .child(
            Element::new("p")
                .class("actions")
                .child(button_link(&study_path(id, &Scope::All), "Study all cards"))
                .child(button_link(&edit_path(id), "Edit set").class("button quiet")),
        )
        .child(Element::new("h2").text("Topics"))
        .child(if set.topics().is_empty() {
            Element::new("p")
                .class("empty")
                .text("This set has no topics yet. Edit the set to add some.")
        } else {
            Element::new("ul").class("topics").children(topics)
        });
    page(set.title(), &[], main)
}

/// Renders the study page of a scope.
pub(super) fn study(set: &StudySet, progress: &Progress, scope: &Scope) -> String {
    let id = set.id();
    let scopes =
        Element::new("nav")
            .class("scopes")
            .attr("aria-label", "Cards to study")
            .child(scope_link(id, &Scope::All, "All cards", scope))
            .children(set.topics().iter().map(|topic| {
                scope_link(id, &Scope::Topic(topic.id().clone()), topic.name(), scope)
            }));
    let card = match progress.view(set, scope) {
        View::Question { topic, card, order } => question(id, scope, topic, card, order),
        View::Answered(review) => answered(id, scope, &review),
        View::Empty => Element::new("section")
            .class("card empty")
            .child(Element::new("p").text("There are no cards to study here."))
            .child(Element::new("p").child(button_link(&edit_path(id), "Add cards"))),
    };
    let main = Element::new("main").class("study").child(scopes).child(
        Element::new("div")
            .class("columns")
            .child(card)
            .child(stats(set, progress, scope)),
    );
    page(set.title(), &[(set_path(id), set.title())], main)
}

/// Creates the link that switches the study page to `target`.
fn scope_link(set: &SetId, target: &Scope, text: &str, current: &Scope) -> Element {
    let element = link(&study_path(set, target), text);
    if target == current {
        element.attr("aria-current", "page")
    } else {
        element
    }
}

/// One answer of a card as the study page shows it.
#[derive(Clone, Copy, Debug)]
struct ShownAnswer<'card> {
    /// The place of the answer on the page, counted from zero. A form of the
    /// study page names an answer by its place, which does not tell whether
    /// the answer is the correct one.
    place: usize,
    /// The letter that labels the answer.
    letter: &'static str,
    /// The position of the answer in [`Card::choices`].
    index: usize,
    /// The text of the answer.
    text: &'card str,
}

impl ShownAnswer<'_> {
    /// Returns the letter and the text of the answer.
    fn label(&self) -> [Element; 2] {
        [
            Element::new("span").class("letter").text(self.letter),
            Element::new("span").class("text").text(self.text),
        ]
    }

    /// Creates the button that chooses the answer.
    fn button(&self) -> Element {
        choice_button(&self.place.to_string(), &self.letter.to_ascii_lowercase())
            .class("answer")
            .children(self.label())
    }
}

/// Lists the answers of `card` in the order the study page shows them. A
/// position in `order` that the card does not have is left out.
fn shown_answers<'card>(card: &'card Card, order: &[usize]) -> Vec<ShownAnswer<'card>> {
    order
        .iter()
        .zip(LETTERS)
        .enumerate()
        .filter_map(|(place, (&index, letter))| {
            Some(ShownAnswer {
                place,
                letter,
                index,
                text: card.choices().get(index)?,
            })
        })
        .collect()
}

/// Creates a button that submits `value` as the choice of a study form. The
/// key `key` presses the button.
fn choice_button(value: &str, key: &str) -> Element {
    Element::new("button")
        .attr("type", "submit")
        .attr("name", "choice")
        .attr("value", value)
        .attr("data-key", key)
}

/// Creates the field that names the card a study form is about.
fn card_field(card: &Card) -> Element {
    Element::new("input")
        .attr("type", "hidden")
        .attr("name", "card")
        .attr("value", card.id().as_str())
}

/// Renders a card waiting for an answer.
fn question(set: &SetId, scope: &Scope, topic: &Topic, card: &Card, order: &[usize]) -> Element {
    let answers = shown_answers(card, order);
    let form = post_form(&format!("{}/answer", study_path(set, scope)))
        .class("answers")
        .child(card_field(card))
        .children(answers.iter().map(ShownAnswer::button))
        .child(
            choice_button("unknown", "?")
                .class("unknown")
                .text("I don't know"),
        );
    Element::new("section")
        .class("card")
        .child(Element::new("p").class("topic").text(topic.name()))
        .child(
            Element::new("div")
                .class("question")
                .children(paragraphs(card.question())),
        )
        .child(form)
        .child(
            Element::new("p")
                .class("hint")
                .text("Type the letter or click an answer."),
        )
}

/// Renders a card that has been answered.
///
/// The marks in the list of answers are all that tells how the card was
/// answered: a chosen answer is marked as correct or as wrong, and the correct
/// answer is marked once the review shows it. After a wrong answer the correct
/// one stays hidden: the answers that have not been chosen are offered again,
/// next to a button that shows the correct one.
fn answered(set: &SetId, scope: &Scope, review: &Review<'_>) -> Element {
    let card = review.card;
    let shown = shown_answers(card, review.order);
    let shows_correct = review.shows_correct_answer();
    // An order from a damaged progress file can lack the correct answer. The
    // list then has nothing to mark, and the answer is given in words.
    let listed = shown
        .iter()
        .any(|answer| answer.index == Card::CORRECT_CHOICE);
    let unlisted = (shows_correct && !listed)
        .then(|| Element::new("p").text(&format!("The correct answer is: {}", card.answer())));
    let items = shown.iter().map(|answer| {
        let correct = answer.index == Card::CORRECT_CHOICE;
        let marks = match (review.chose(answer.index), correct) {
            (true, true) => Some("answer chosen right"),
            (true, false) => Some("answer chosen wrong"),
            (false, true) if shows_correct => Some("answer right"),
            (false, _) if shows_correct => Some("answer"),
            // While the correct answer is hidden, an answer that has not been
            // chosen is a button.
            (false, _) => None,
        };
        marks.map_or_else(
            || Element::new("li").child(answer.button()),
            |marks| Element::new("li").class(marks).children(answer.label()),
        )
    });
    let list = Element::new("ol").class("answers").children(items);
    let (answers, hint) = if shows_correct {
        (list, None)
    } else {
        let retry = post_form(&format!("{}/retry", study_path(set, scope)))
            .class("retry")
            .child(card_field(card))
            .child(list)
            .child(
                choice_button("unknown", "s")
                    .class("reveal")
                    .text("Show correct answer"),
            );
        let hint = Element::new("p")
            .class("hint")
            .text("Choose another answer or show the correct one. Only your first answer counts.");
        (retry, Some(hint))
    };
    let explanation = card.explanation().map(|text| {
        Element::new("details")
            .class("explanation")
            .child(
                Element::new("summary")
                    .attr("data-key", "x")
                    .text("Explanation"),
            )
            .children(paragraphs(text))
    });
    let next = post_form(&format!("{}/next", study_path(set, scope)))
        .class("next")
        .child(
            submit("Next")
                .attr("data-key", "n Enter Space ArrowRight ArrowDown")
                .flag("autofocus", true),
        );
    Element::new("section")
        .class("card answered")
        .child(Element::new("p").class("topic").text(review.topic.name()))
        .child(
            Element::new("div")
                .class("question")
                .children(paragraphs(card.question())),
        )
        .child(answers)
        .children(unlisted)
        .children(hint)
        .children(explanation)
        .child(next)
}

/// Renders the progress panel of the study page.
fn stats(set: &StudySet, progress: &Progress, scope: &Scope) -> Element {
    let id = set.id();
    let (heading, tally) = match scope {
        Scope::All => ("All cards".to_owned(), set_tally(set, progress)),
        Scope::Topic(topic) => set.topic(topic).map_or_else(
            || (String::new(), Tally::default()),
            |topic| (topic.name().to_owned(), progress.tally(topic.cards())),
        ),
    };
    let total = |label: &str, percent: usize| {
        Element::new("div")
            .child(Element::new("dd").text(&format!("{percent}%")))
            .child(Element::new("dt").text(label))
    };
    let totals = Element::new("dl")
        .class("totals")
        .child(total("Aptitude", tally.aptitude_percent()))
        .child(total("Seen", tally.seen_percent()));
    let tallies: Vec<(&Topic, Tally)> = set
        .topics()
        .iter()
        .map(|topic| (topic, progress.tally(topic.cards())))
        .collect();
    let breakdown = (*scope == Scope::All && !tallies.is_empty()).then(|| {
        let legend = tallies.iter().map(|(topic, topic_tally)| {
            Element::new("li")
                .child(link(
                    &study_path(id, &Scope::Topic(topic.id().clone())),
                    topic.name(),
                ))
                .child(
                    Element::new("span")
                        .class("summary")
                        .text(&format!(" {}", summary(*topic_tally))),
                )
        });
        [
            chart(id, &tallies),
            Element::new("ol").class("legend").children(legend),
        ]
    });
    Element::new("aside")
        .class("stats")
        .attr("aria-label", "Progress")
        .child(Element::new("h2").text(&heading))
        .child(totals)
        .children(breakdown.into_iter().flatten())
}

/// Draws seen and aptitude bars for each topic.
fn chart(set: &SetId, tallies: &[(&Topic, Tally)]) -> Element {
    /// Horizontal space given to one topic.
    const SLOT: usize = 36;
    /// The width of one bar.
    const BAR: usize = 12;
    /// The height of a bar at 100%.
    const FULL: usize = 100;
    /// Empty space around the bars.
    const MARGIN: usize = 8;
    /// The narrowest drawing area.
    const MIN_WIDTH: usize = 260;

    let baseline = MARGIN.saturating_add(FULL);
    // A minimum width keeps the bars of a set with few topics from being
    // drawn oversized.
    let width = tallies
        .len()
        .saturating_mul(SLOT)
        .saturating_add(MARGIN.saturating_mul(2))
        .max(MIN_WIDTH);
    let height = baseline.saturating_add(22);
    let bar = |class: &'static str, x: usize, percent: usize| {
        let filled = percent.min(FULL);
        Element::new("rect")
            .class(class)
            .attr("x", &x.to_string())
            .attr("y", &baseline.saturating_sub(filled).to_string())
            .attr("width", &BAR.to_string())
            .attr("height", &filled.to_string())
    };
    let groups = tallies
        .iter()
        .enumerate()
        .map(|(position, (topic, tally))| {
            let left = position
                .saturating_mul(SLOT)
                .saturating_add(MARGIN)
                .saturating_add(4);
            Element::new("a")
                .attr("href", &study_path(set, &Scope::Topic(topic.id().clone())))
                .child(Element::new("title").text(&format!(
                    "{}: {}",
                    topic.name(),
                    summary(*tally)
                )))
                .child(bar("seen", left, tally.seen_percent()))
                .child(bar(
                    "aptitude",
                    left.saturating_add(BAR).saturating_add(2),
                    tally.aptitude_percent(),
                ))
                .child(
                    Element::new("text")
                        .attr("x", &left.saturating_add(BAR).saturating_add(1).to_string())
                        .attr("y", &baseline.saturating_add(16).to_string())
                        .attr("text-anchor", "middle")
                        .text(&position.saturating_add(1).to_string()),
                )
        });
    Element::new("svg")
        .class("chart")
        .attr("viewBox", &format!("0 0 {width} {height}"))
        .attr("role", "img")
        .attr("aria-label", "Seen and aptitude by topic")
        .child(
            Element::new("line")
                .attr("x1", "0")
                .attr("x2", &width.to_string())
                .attr("y1", &baseline.to_string())
                .attr("y2", &baseline.to_string()),
        )
        .children(groups)
}

/// Renders the editor of a set.
pub(super) fn editor(set: &StudySet) -> String {
    let id = set.id();
    let base = edit_path(id);
    let title_form = post_form(&format!("{base}/title"))
        .class("row")
        .child(text_field(
            "Title",
            "title",
            set.title(),
            MAX_NAME_LEN,
            Entry::Required,
        ))
        .child(submit("Save title"));
    let topics = set.topics().iter().map(|topic| {
        let topic_base = format!("{base}/topics/{}", topic.id());
        let cards = topic.cards().iter().map(|card| {
            Element::new("li")
                .child(link(&card_path(id, card.id()), card.question()))
                .child(
                    Element::new("span")
                        .class("summary")
                        .text(&format!(" {}", card.answer())),
                )
        });
        let count = topic.cards().len();
        Element::new("section")
            .class("topic")
            .child(Element::new("h2").text(topic.name()))
            .child(
                post_form(&format!("{topic_base}/rename"))
                    .class("row")
                    .child(text_field(
                        "Topic name",
                        "name",
                        topic.name(),
                        MAX_NAME_LEN,
                        Entry::Required,
                    ))
                    .child(submit("Rename")),
            )
            .child(if count == 0 {
                Element::new("p").class("empty").text("No cards yet.")
            } else {
                Element::new("ul").class("cards").children(cards)
            })
            .child(
                Element::new("p").child(button_link(&new_card_path(id, topic.id()), "Add a card")),
            )
            .child(
                post_form(&format!("{topic_base}/delete"))
                    .class("row danger")
                    .child(confirmation(&format!(
                        "Delete this topic and its {count} card{}",
                        if count == 1 { "" } else { "s" }
                    )))
                    .child(submit("Delete topic")),
            )
    });
    let add_topic = post_form(&format!("{base}/topics"))
        .class("row")
        .child(text_field(
            "New topic",
            "name",
            "",
            MAX_NAME_LEN,
            Entry::Required,
        ))
        .child(submit("Add topic"));
    let danger = Element::new("section")
        .class("danger")
        .child(Element::new("h2").text("Start over or remove"))
        .child(
            post_form(&format!("{base}/reset"))
                .class("row")
                .child(confirmation("Forget all progress on this set"))
                .child(submit("Reset progress")),
        )
        .child(
            post_form(&format!("{base}/delete"))
                .class("row")
                .child(confirmation("Delete this set and its progress"))
                .child(submit("Delete set")),
        );
    let main = Element::new("main")
        .class("editor")
        .child(Element::new("h1").text(&format!("Edit {}", set.title())))
        .child(title_form)
        .children(topics)
        .child(add_topic)
        .child(danger);
    page(
        &format!("Edit {}", set.title()),
        &[(set_path(id), set.title())],
        main,
    )
}

/// Where a card form saves to.
#[derive(Clone, Copy, Debug)]
pub(super) enum CardTarget<'set> {
    /// A card to add to this topic of the set.
    New(&'set Topic),
    /// The card of the set that has this identifier.
    Existing(&'set CardId),
}

/// Renders the form that adds or edits a card. `problem` is shown above the
/// form when the previous submission was rejected.
pub(super) fn card_form(
    set: &StudySet,
    target: CardTarget<'_>,
    draft: &CardDraft,
    problem: Option<&str>,
) -> String {
    let id = set.id();
    let (heading, action) = match target {
        CardTarget::New(topic) => (
            format!("New card in {}", topic.name()),
            new_card_path(id, topic.id()),
        ),
        CardTarget::Existing(card) => ("Edit card".to_owned(), card_path(id, card)),
    };
    let wrong = (0..MAX_WRONG_ANSWERS).map(|position| {
        let value = draft.wrong.get(position).map_or("", String::as_str);
        let label = format!("Wrong answer {}", position.saturating_add(1));
        // A card needs one wrong answer, and the first field is the place for
        // it.
        let entry = if position == 0 {
            Entry::Required
        } else {
            Entry::Optional
        };
        text_field(&label, "wrong", value, MAX_ANSWER_LEN, entry)
    });
    let form = post_form(&action)
        .class("card-form")
        .child(text_area(
            "Question",
            "question",
            &draft.question,
            Entry::Required,
        ))
        .child(text_field(
            "Correct answer",
            "answer",
            &draft.answer,
            MAX_ANSWER_LEN,
            Entry::Required,
        ))
        .children(wrong)
        .child(text_area(
            "Explanation (optional)",
            "explanation",
            draft.explanation.as_deref().unwrap_or_default(),
            Entry::Optional,
        ))
        .child(
            Element::new("p")
                .class("actions")
                .child(submit("Save card"))
                .child(button_link(&edit_path(id), "Cancel").class("button quiet")),
        );
    let delete = match target {
        CardTarget::New(_) => None,
        CardTarget::Existing(card) => Some(
            post_form(&format!("{}/delete", card_path(id, card)))
                .class("row danger")
                .child(confirmation("Delete this card"))
                .child(submit("Delete card")),
        ),
    };
    let main = Element::new("main")
        .class("editor")
        .child(Element::new("h1").text(heading.as_str()))
        .children(problem.map(|text| {
            Element::new("p")
                .class("error")
                .attr("role", "alert")
                .text(text)
        }))
        .child(form)
        .children(delete);
    page(
        &heading,
        &[(set_path(id), set.title()), (edit_path(id), "Edit")],
        main,
    )
}

/// Renders a page that reports a problem, with a link that leads on.
pub(super) fn notice(heading: &str, text: &str, back: &str) -> String {
    let main = Element::new("main")
        .child(Element::new("h1").text(heading))
        .child(
            Element::new("p")
                .class("error")
                .attr("role", "alert")
                .text(text),
        )
        .child(Element::new("p").child(button_link(back, "Go back")));
    page(heading, &[], main)
}

#[cfg(test)]
mod tests {
    use super::{CardTarget, Element, SetState, SetSummary};
    use super::{answered, card_form, editor, home, notice, overview, paragraphs, study};
    use crate::deck::{Card, CardDraft, StudySet};
    use crate::mastery::Outcome;
    use crate::study::{Choice, Progress, Review, Scope, Tally, View};

    fn sample() -> StudySet {
        let text = r#"
title = "Solar <system>"

[[topics]]
name = "Planets & moons"

[[topics.cards]]
id = "closest"
question = "Which planet is closest to the Sun?\nSecond line.\n\nSecond paragraph."
answer = "Mercury"
wrong = ["Venus", "Earth", "<b>Mars</b>"]
explanation = "It orbits at 0.39 AU."

[[topics]]
name = "Stars"
"#;
        StudySet::from_toml("solar".parse().expect("valid id"), text).expect("valid set")
    }

    fn prepared(set: &StudySet) -> Progress {
        let mut progress = Progress::new();
        progress
            .prepare(set, &Scope::All, &mut fastrand::Rng::with_seed(1))
            .expect("the scope exists");
        progress
    }

    #[test]
    fn paragraphs_keep_line_breaks() {
        let rendered = Element::new("div")
            .children(paragraphs("one\ntwo\n\n\n three \n\n"))
            .to_document();
        assert_eq!(
            rendered, "<!DOCTYPE html>\n<div><p>one<br>two</p><p>three</p></div>\n",
            "blank lines split paragraphs and single breaks are kept"
        );
    }

    #[test]
    fn home_lists_sets_and_problems() {
        let sets = [
            SetSummary {
                id: "solar".parse().expect("valid id"),
                state: SetState::Usable {
                    title: "Solar <system>".to_owned(),
                    tally: Tally {
                        cards: 1,
                        seen: 1,
                        aptitude_points: 250,
                    },
                },
            },
            SetSummary {
                id: "broken".parse().expect("valid id"),
                state: SetState::Broken {
                    problem: "broken.toml: not a study set file".to_owned(),
                },
            },
        ];
        let html = home(&sets, &["Bad Name.toml".to_owned()]);
        assert!(
            html.contains("<a href=\"/sets/solar\">Solar &lt;system&gt;</a>"),
            "set link:\n{html}"
        );
        assert!(
            html.contains("1 card · 100% seen · 63% aptitude"),
            "summary:\n{html}"
        );
        assert!(
            html.contains("href=\"/sets/solar/study\""),
            "study link:\n{html}"
        );
        assert!(
            html.contains("broken.toml: not a study set file"),
            "problem of a broken set:\n{html}"
        );
        assert!(
            html.contains("Bad Name.toml"),
            "unusable file names:\n{html}"
        );
        assert!(
            html.contains("<form method=\"post\" action=\"/sets\""),
            "create form:\n{html}"
        );
        assert!(home(&[], &[]).contains("No study sets yet"), "empty state");
    }

    #[test]
    fn overview_lists_topics_with_progress() {
        let set = sample();
        let html = overview(&set, &Progress::new());
        assert!(
            html.contains("<h1>Solar &lt;system&gt;</h1>"),
            "escaped title:\n{html}"
        );
        assert!(
            html.contains("href=\"/sets/solar/study/planets-moons\""),
            "topic link:\n{html}"
        );
        assert!(
            html.contains("Planets &amp; moons"),
            "escaped topic name:\n{html}"
        );
        assert!(
            html.contains("1 card · 0% seen · 0% aptitude"),
            "topic summary:\n{html}"
        );
        assert!(
            html.contains("0 cards · 0% seen"),
            "summary of an empty topic:\n{html}"
        );
    }

    #[test]
    fn the_overview_of_a_set_without_topics_says_how_to_add_some() {
        let set =
            StudySet::try_new("empty".parse().expect("valid id"), "Empty").expect("valid title");
        let html = overview(&set, &Progress::new());
        assert!(
            html.contains("This set has no topics yet. Edit the set to add some."),
            "empty state:\n{html}"
        );
        assert!(
            html.contains("0 cards · 0% seen · 0% aptitude"),
            "summary of a set without cards:\n{html}"
        );
    }

    #[test]
    fn a_question_offers_each_answer_and_declining() {
        let set = sample();
        let progress = prepared(&set);
        let html = study(&set, &progress, &Scope::All);
        assert!(
            html.contains("<form method=\"post\" action=\"/sets/solar/study/answer\""),
            "form:\n{html}"
        );
        assert!(
            html.contains("name=\"card\" value=\"closest\""),
            "card id field:\n{html}"
        );
        for (position, key) in ["a", "b", "c", "d"].iter().enumerate() {
            assert!(
                html.contains(&format!(
                    "name=\"choice\" value=\"{position}\" data-key=\"{key}\""
                )),
                "answer button {key}:\n{html}"
            );
        }
        assert!(
            html.contains("value=\"unknown\" data-key=\"?\""),
            "declining:\n{html}"
        );
        assert!(
            html.contains("&lt;b&gt;Mars&lt;/b&gt;"),
            "answers are escaped:\n{html}"
        );
        assert!(
            !html.contains("<b>Mars</b>"),
            "no markup from a card reaches the page"
        );
        assert!(
            html.contains(
                "<p>Which planet is closest to the Sun?<br>Second line.</p><p>Second paragraph.</p>"
            ),
            "question paragraphs:\n{html}"
        );
        assert!(
            html.contains("aria-current=\"page\" href=\"/sets/solar/study\"")
                || html.contains("href=\"/sets/solar/study\" aria-current=\"page\""),
            "the current scope is marked:\n{html}"
        );
        assert!(
            html.contains("<svg class=\"chart\""),
            "the chart is shown for all cards:\n{html}"
        );
    }

    /// Answers the sample card with `answer`, chooses `retries` after it, and
    /// renders the study page.
    ///
    /// The choices of the sample card are "Mercury", which is correct, at
    /// position 0, then "Venus", "Earth", and "<b>Mars</b>".
    fn review_page(answer: Choice, retries: &[Choice]) -> String {
        let set = sample();
        let mut progress = prepared(&set);
        let view = progress.view(&set, &Scope::All);
        let View::Question { card, .. } = view else {
            panic!("expected a question, found {view:?}");
        };
        let card = card.id().clone();
        progress
            .answer(
                &set,
                &Scope::All,
                &card,
                answer,
                &mut fastrand::Rng::with_seed(2),
            )
            .expect("the answer is accepted");
        for &retry in retries {
            progress
                .retry(&set, &Scope::All, &card, retry)
                .expect("the retry is accepted");
        }
        study(&set, &progress, &Scope::All)
    }

    /// Returns the texts of the listed answers that carry exactly the classes
    /// `marks`, in the order of the list and escaped as on the page.
    fn marked<'a>(page: &'a str, marks: &str) -> Vec<&'a str> {
        page.split(&format!("<li class=\"{marks}\"><span class=\"letter\">"))
            .skip(1)
            .map(|item| {
                let (_, text) = item
                    .split_once("</span><span class=\"text\">")
                    .expect("the letter is followed by the text");
                text.split_once("</span></li>")
                    .expect("the text ends the answer")
                    .0
            })
            .collect()
    }

    /// Returns the markup of the button that chooses the answer `text`.
    fn button_for<'a>(page: &'a str, text: &str) -> &'a str {
        let label = format!("<span class=\"text\">{text}</span></button>");
        page.split_inclusive("</button>")
            .filter(|piece| piece.ends_with(&label))
            .find_map(|piece| piece.rfind("<button").and_then(|start| piece.get(start..)))
            .unwrap_or_else(|| panic!("no button chooses {text:?}:\n{page}"))
    }

    /// Fails when the answered card on `page` says in words how the card was
    /// answered or which answer is correct. The marks in the list of answers
    /// are all that tells either.
    fn assert_no_verdict_in_words(page: &str) {
        assert!(
            page.contains(
                "<section class=\"card answered\"><p class=\"topic\">Planets &amp; moons</p>\
                 <div class=\"question\">"
            ),
            "the question follows the topic directly:\n{page}"
        );
        for words in [">Correct<", "Incorrect", "The correct answer", "verdict"] {
            assert!(!page.contains(words), "the page holds {words:?}:\n{page}");
        }
    }

    #[test]
    fn a_correct_answer_is_confirmed() {
        let html = review_page(Choice::Answer(Card::CORRECT_CHOICE), &[]);
        assert_eq!(
            marked(&html, "answer chosen right"),
            ["Mercury"],
            "the chosen answer is marked as correct:\n{html}"
        );
        assert_no_verdict_in_words(&html);
        assert!(
            !html.contains("Show correct answer"),
            "nothing is left to reveal:\n{html}"
        );
        assert!(
            html.contains(
                "<summary data-key=\"x\">Explanation</summary><p>It orbits at 0.39 AU.</p>"
            ),
            "explanation:\n{html}"
        );
        assert!(
            html.contains("action=\"/sets/solar/study/next\""),
            "next form:\n{html}"
        );
        assert!(
            html.contains("autofocus"),
            "the next button takes focus:\n{html}"
        );
        assert!(
            html.contains("<dd>63%</dd><dt>Aptitude</dt>"),
            "aptitude after one correct answer:\n{html}"
        );
        assert!(
            html.contains("<dd>100%</dd><dt>Seen</dt>"),
            "seen after one answer:\n{html}"
        );
    }

    #[test]
    fn a_wrong_answer_keeps_the_correct_one_hidden_and_offers_the_others_again() {
        let set = sample();
        let question = study(&set, &prepared(&set), &Scope::All);
        let html = review_page(Choice::Answer(1), &[]);
        assert_no_verdict_in_words(&html);
        assert_eq!(
            marked(&html, "answer chosen wrong"),
            ["Venus"],
            "the chosen answer is marked wrong:\n{html}"
        );
        assert!(
            html.contains(
                "<form method=\"post\" action=\"/sets/solar/study/retry\" class=\"retry\">\
                 <input type=\"hidden\" name=\"card\" value=\"closest\"><ol class=\"answers\">"
            ),
            "the list of answers is a form for retries:\n{html}"
        );
        for left in ["Mercury", "Earth", "&lt;b&gt;Mars&lt;/b&gt;"] {
            let button = button_for(&question, left);
            assert!(
                html.contains(&format!("<li>{button}</li>")),
                "{left} keeps the button it had in the question, {button}:\n{html}"
            );
        }
        assert!(
            !html.contains(button_for(&question, "Venus")),
            "the chosen answer cannot be chosen again:\n{html}"
        );
        assert!(
            html.contains(
                "</ol><button type=\"submit\" name=\"choice\" value=\"unknown\" data-key=\"s\" \
                 class=\"reveal\">Show correct answer</button></form>"
            ),
            "the form offers to show the correct answer:\n{html}"
        );
        assert!(
            html.contains(
                "<p class=\"hint\">Choose another answer or show the correct one. \
                 Only your first answer counts.</p>"
            ),
            "hint:\n{html}"
        );
        assert!(
            !html.contains("class=\"answer right\"") && !html.contains("chosen right"),
            "the correct answer is not marked in the list:\n{html}"
        );
        assert!(
            html.contains("action=\"/sets/solar/study/next\""),
            "the learner can move on without the correct answer:\n{html}"
        );
    }

    #[test]
    fn a_wrong_retry_is_marked_like_a_wrong_answer() {
        let html = review_page(Choice::Answer(1), &[Choice::Answer(3)]);
        assert_eq!(
            marked(&html, "answer chosen wrong").len(),
            2,
            "the answer and the retry are marked wrong:\n{html}"
        );
        for chosen in ["Venus", "&lt;b&gt;Mars&lt;/b&gt;"] {
            assert!(
                marked(&html, "answer chosen wrong").contains(&chosen),
                "{chosen} is marked wrong:\n{html}"
            );
        }
        assert_eq!(
            html.matches("<li><button type=\"submit\" name=\"choice\"")
                .count(),
            2,
            "the two other answers can be chosen:\n{html}"
        );
        assert_no_verdict_in_words(&html);
        assert!(
            html.contains("Show correct answer"),
            "the correct answer can be asked for:\n{html}"
        );
    }

    #[test]
    fn choosing_the_correct_answer_in_a_retry_ends_the_retries() {
        let correct = Choice::Answer(Card::CORRECT_CHOICE);
        let html = review_page(Choice::Answer(1), &[Choice::Answer(3), correct]);
        assert_eq!(
            marked(&html, "answer chosen right"),
            ["Mercury"],
            "the correct answer is marked as chosen:\n{html}"
        );
        assert_no_verdict_in_words(&html);
        assert_eq!(
            marked(&html, "answer chosen wrong").len(),
            2,
            "the wrong choices stay marked:\n{html}"
        );
        assert_eq!(
            marked(&html, "answer"),
            ["Earth"],
            "the answer that was not chosen has no mark:\n{html}"
        );
        assert!(
            !html.contains("name=\"choice\"") && !html.contains("/study/retry"),
            "a review that shows the correct answer has no answer to choose:\n{html}"
        );
        assert!(
            !html.contains("Only your first answer counts."),
            "the hint for retries is gone:\n{html}"
        );
        assert!(
            html.contains("action=\"/sets/solar/study/next\""),
            "next form:\n{html}"
        );
        assert!(
            html.contains("<dd>13%</dd><dt>Aptitude</dt>"),
            "the aptitude is that of one wrong answer:\n{html}"
        );
    }

    #[test]
    fn asking_for_the_correct_answer_after_a_wrong_one_shows_it() {
        let html = review_page(Choice::Answer(1), &[Choice::DontKnow]);
        assert_eq!(
            marked(&html, "answer right"),
            ["Mercury"],
            "the correct answer is marked in the list:\n{html}"
        );
        assert_no_verdict_in_words(&html);
        assert_eq!(
            marked(&html, "answer chosen wrong"),
            ["Venus"],
            "the wrong answer stays marked:\n{html}"
        );
        assert!(
            !html.contains("chosen right"),
            "the correct answer was not chosen:\n{html}"
        );
        assert!(
            !html.contains("name=\"choice\"") && !html.contains("/study/retry"),
            "a review that shows the correct answer has no answer to choose:\n{html}"
        );
    }

    #[test]
    fn declining_shows_the_correct_answer() {
        let html = review_page(Choice::DontKnow, &[]);
        assert_eq!(
            marked(&html, "answer right"),
            ["Mercury"],
            "the correct answer is marked in the list:\n{html}"
        );
        assert_no_verdict_in_words(&html);
        assert!(!html.contains("chosen"), "no answer was chosen:\n{html}");
        assert!(
            !html.contains("Show correct answer"),
            "nothing is left to reveal:\n{html}"
        );
    }

    #[test]
    fn declining_names_the_correct_answer_when_the_list_does_not_hold_it() {
        let set = sample();
        let topic = &set.topics()[0];
        let card = &topic.cards()[0];
        // An order from a damaged progress file, without the correct answer.
        let declined = Review {
            topic,
            card,
            order: &[2, 1],
            choice: Choice::DontKnow,
            outcome: Outcome::DontKnow,
            retries: &[],
        };
        let html = answered(set.id(), &Scope::All, &declined).to_document();
        assert!(
            !html.contains("class=\"answer right\""),
            "no answer in the list is the correct one:\n{html}"
        );
        assert!(
            html.contains("</ol><p>The correct answer is: Mercury</p>"),
            "the answer is given in words below the list:\n{html}"
        );
    }

    #[test]
    fn a_list_without_the_correct_answer_still_leads_to_it() {
        let set = sample();
        let topic = &set.topics()[0];
        let card = &topic.cards()[0];
        // An order from a damaged progress file, without the correct answer.
        let missed = Review {
            topic,
            card,
            order: &[2, 1],
            choice: Choice::Answer(2),
            outcome: Outcome::Wrong,
            retries: &[],
        };
        let open = answered(set.id(), &Scope::All, &missed).to_document();
        assert_eq!(
            marked(&open, "answer chosen wrong"),
            ["Earth"],
            "the wrong answer is marked:\n{open}"
        );
        assert!(
            open.contains("Show correct answer"),
            "the correct answer can be asked for:\n{open}"
        );
        assert!(
            !open.contains("The correct answer"),
            "the correct answer stays hidden until it is asked for:\n{open}"
        );

        let asked = Review {
            retries: &[Choice::DontKnow],
            ..missed
        };
        let html = answered(set.id(), &Scope::All, &asked).to_document();
        assert!(
            html.contains("</ol><p>The correct answer is: Mercury</p>"),
            "the answer is given in words below the list:\n{html}"
        );
    }

    #[test]
    fn a_topic_without_cards_shows_an_empty_state() {
        let set = sample();
        let scope = Scope::Topic(set.topics()[1].id().clone());
        let mut progress = Progress::new();
        progress
            .prepare(&set, &scope, &mut fastrand::Rng::with_seed(3))
            .expect("the topic exists");
        let html = study(&set, &progress, &scope);
        assert!(
            html.contains("There are no cards to study here."),
            "empty state:\n{html}"
        );
        assert!(
            !html.contains("<svg"),
            "no chart for a single topic:\n{html}"
        );
        assert!(
            html.contains("<h2>Stars</h2>"),
            "the panel names the topic:\n{html}"
        );
    }

    #[test]
    fn the_editor_offers_every_change() {
        let set = sample();
        let html = editor(&set);
        for action in [
            "/sets/solar/edit/title",
            "/sets/solar/edit/topics",
            "/sets/solar/edit/topics/planets-moons/rename",
            "/sets/solar/edit/topics/planets-moons/delete",
            "/sets/solar/edit/reset",
            "/sets/solar/edit/delete",
        ] {
            assert!(
                html.contains(&format!("action=\"{action}\"")),
                "form for {action}:\n{html}"
            );
        }
        assert!(
            html.contains("href=\"/sets/solar/edit/cards/closest\""),
            "card link:\n{html}"
        );
        assert!(
            html.contains("href=\"/sets/solar/edit/topics/stars/cards/new\""),
            "add-card link:\n{html}"
        );
        assert!(
            html.contains("value=\"Solar &lt;system&gt;\""),
            "the title field is escaped:\n{html}"
        );
        assert!(
            html.contains("Delete this topic and its 1 card<"),
            "singular count:\n{html}"
        );
        assert!(
            html.contains("Delete this topic and its 0 cards<"),
            "plural count:\n{html}"
        );
    }

    #[test]
    fn the_card_form_keeps_what_was_typed() {
        let set = sample();
        let draft = CardDraft {
            question: "Why <is> the sky blue?".to_owned(),
            answer: "Scattering".to_owned(),
            wrong: vec!["Reflection".to_owned()],
            explanation: None,
        };
        let html = card_form(
            &set,
            CardTarget::New(&set.topics()[1]),
            &draft,
            Some("the question is empty"),
        );
        assert!(
            html.contains("action=\"/sets/solar/edit/topics/stars/cards/new\""),
            "form action:\n{html}"
        );
        assert!(
            html.contains("Why &lt;is&gt; the sky blue?</textarea>"),
            "the question is kept and escaped:\n{html}"
        );
        assert!(
            html.contains("name=\"answer\" value=\"Scattering\""),
            "the answer is kept:\n{html}"
        );
        assert!(
            html.contains("name=\"wrong\" value=\"Reflection\""),
            "wrong answers are kept:\n{html}"
        );
        assert_eq!(
            html.matches("name=\"wrong\"").count(),
            5,
            "five wrong-answer fields"
        );
        assert!(
            html.contains("role=\"alert\">the question is empty</p>"),
            "the problem is shown:\n{html}"
        );
        assert!(
            !html.contains("Delete card"),
            "a new card cannot be deleted:\n{html}"
        );

        let card = set.topics()[0].cards()[0].id();
        let existing = card_form(&set, CardTarget::Existing(card), &draft, None);
        assert!(
            existing.contains("action=\"/sets/solar/edit/cards/closest\""),
            "form action:\n{existing}"
        );
        assert!(
            existing.contains("action=\"/sets/solar/edit/cards/closest/delete\""),
            "delete form:\n{existing}"
        );
        assert!(
            !existing.contains("role=\"alert\""),
            "no problem is shown:\n{existing}"
        );
    }

    #[test]
    fn notices_escape_their_text() {
        let html = notice("Not found", "there is no study set \"<x>\"", "/");
        assert!(
            html.contains("there is no study set \"&lt;x&gt;\""),
            "escaped message:\n{html}"
        );
        assert!(
            html.contains("<a href=\"/\" class=\"button\">Go back</a>"),
            "back link:\n{html}"
        );
    }
}
