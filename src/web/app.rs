//! The application: requests, routes, and the actions behind them.

use std::str::FromStr;
use std::sync::Mutex;

use super::pages::{self, CardTarget, SetState, SetSummary};
use crate::deck::{CardDraft, CardId, DeckError, SetId, StudySet, TopicId};
use crate::http::{Fields, Handler, Method, Request, Response, Status};
use crate::picker::RandomSource;
use crate::store::{Store, StoreError};
use crate::study::{Choice, Progress, Scope, StudyError, View};

/// The style sheet served at `/assets/app.css`.
const STYLE_SHEET: &str = include_str!("../../assets/app.css");
/// The icon served at `/assets/icon.svg`.
const ICON: &str = include_str!("../../assets/icon.svg");
/// The script served at `/assets/app.js`, compiled from `web/src/app.ts`.
const SCRIPT: &str = include_str!("../../assets/app.js");

/// The application, ready to handle requests.
///
/// Requests are handled one at a time, so the changes of two requests to the
/// files never interleave. If handling a request panics, the application
/// answers every later request with status 500 until it is restarted.
#[derive(Debug)]
pub struct App {
    /// The files the application works on.
    store: Store,
    /// The source of randomness. Its lock is held while a request is
    /// handled, so requests change the files one at a time.
    random: Mutex<fastrand::Rng>,
}

impl App {
    /// Creates the application for the sets and progress in `store`.
    #[must_use]
    pub fn new(store: Store) -> Self {
        Self {
            store,
            random: Mutex::new(fastrand::Rng::new()),
        }
    }

    /// Creates the application with a fixed random seed, so that the cards
    /// chosen are the same on every run.
    #[must_use]
    pub const fn with_seed(store: Store, seed: u64) -> Self {
        Self {
            store,
            random: Mutex::new(fastrand::Rng::with_seed(seed)),
        }
    }
}

impl Handler for App {
    fn handle(&self, request: &Request) -> Response {
        // A poisoned lock means that a request panicked while it worked on the
        // files. What the lock guards is left alone from then on: every
        // request is refused until the application is restarted.
        let Ok(mut random) = self.random.lock() else {
            return Response::text(Status::InternalServerError, STOPPED);
        };
        route(&self.store, &mut *random, request).unwrap_or_else(Failure::into_response)
    }
}

/// The reply to every request once handling a request has panicked.
const STOPPED: &str = "A request failed unexpectedly. Restart studycards to continue.";

/// Why a request could not be fulfilled.
#[derive(Debug)]
enum Failure {
    /// Nothing lives at the requested path.
    NotFound,
    /// The submitted form cannot be used.
    Rejected {
        /// Why the form was rejected, as a sentence for the learner.
        reason: String,
        /// The path of the page that holds the form.
        back: String,
    },
    /// Reading or writing files failed.
    Store(StoreError),
}

impl Failure {
    /// Renders the failure as a page.
    fn into_response(self) -> Response {
        let (status, heading, text, back) = match self {
            Self::NotFound => (
                Status::NotFound,
                "Not found",
                "There is nothing at this address.".to_owned(),
                "/".to_owned(),
            ),
            Self::Rejected { reason, back } => {
                (Status::BadRequest, "That did not work", reason, back)
            }
            Self::Store(StoreError::UnknownSet(id)) => (
                Status::NotFound,
                "Not found",
                StoreError::UnknownSet(id).to_string(),
                "/".to_owned(),
            ),
            Self::Store(error) => (
                Status::InternalServerError,
                "Something went wrong",
                error.to_string(),
                "/".to_owned(),
            ),
        };
        Response::html(status, pages::notice(heading, &text, &back))
    }
}

impl From<StoreError> for Failure {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

/// Parses a path segment as an identifier; a segment that is not one cannot
/// name anything.
fn id<Id: FromStr>(segment: &str) -> Result<Id, Failure> {
    segment.parse().map_err(|_unusable| Failure::NotFound)
}

/// Dispatches a request to its action.
fn route(
    store: &Store,
    random: &mut dyn RandomSource,
    request: &Request,
) -> Result<Response, Failure> {
    let reading = matches!(request.method(), Method::Get | Method::Head);
    match (reading, request.segments().as_slice()) {
        (true, []) => home(store),
        (true, ["assets", "app.css"]) => Ok(asset("text/css; charset=utf-8", STYLE_SHEET)),
        (true, ["assets", "app.js"]) => Ok(asset("text/javascript; charset=utf-8", SCRIPT)),
        (true, ["assets", "icon.svg"]) => Ok(asset("image/svg+xml", ICON)),
        (false, ["sets"]) => {
            let form = posted_form(request, "/")?;
            let set = store
                .create_set(form.get("title").unwrap_or_default())
                .map_err(|error| rejected_or_failed(error, "/"))?;
            Ok(Response::see_other(&pages::edit_path(set.id())))
        }
        (_, ["sets", set, rest @ ..]) => {
            let set: SetId = id(set)?;
            match rest {
                ["study", scope @ ..] => study_route(store, random, request, &set, scope),
                ["edit", action @ ..] => edit_route(store, request, &set, action),
                [] if reading => {
                    let loaded = store.load_set(&set)?;
                    let progress = store.load_progress(&set)?;
                    Ok(Response::html(
                        Status::Ok,
                        pages::overview(&loaded, &progress),
                    ))
                }
                _ => Err(Failure::NotFound),
            }
        }
        _ => Err(Failure::NotFound),
    }
}

/// Creates the response that serves a file compiled into the program.
fn asset(media_type: &'static str, content: &str) -> Response {
    Response::new(Status::Ok, media_type, content.as_bytes().to_vec())
}

/// Reads the form of a post, rejecting a body that is not one.
fn posted_form(request: &Request, back: &str) -> Result<Fields, Failure> {
    request.form().ok_or_else(|| Failure::Rejected {
        reason: "The form could not be read.".to_owned(),
        back: back.to_owned(),
    })
}

/// Turns a rejected change into a message for the form at `back`, and any
/// other failure into a store failure.
fn rejected_or_failed(error: StoreError, back: &str) -> Failure {
    match error {
        StoreError::InvalidSet { source, .. } => rejected(&source, back),
        other @ (StoreError::Io { .. }
        | StoreError::InvalidProgress { .. }
        | StoreError::UnknownSet(_)) => Failure::Store(other),
    }
}

/// Turns a rejected change into a message for the form at `back`.
fn rejected(error: &DeckError, back: &str) -> Failure {
    let mut reason = error.to_string();
    if let Some(first) = reason.get(..1).map(str::to_uppercase) {
        reason.replace_range(..1, &first);
    }
    reason.push('.');
    Failure::Rejected {
        reason,
        back: back.to_owned(),
    }
}

/// Renders the home page.
fn home(store: &Store) -> Result<Response, Failure> {
    let listing = store.list()?;
    let sets: Vec<SetSummary> = listing
        .sets
        .into_iter()
        .map(|id| {
            let loaded = store.load_set(&id).and_then(|set| {
                let progress = store.load_progress(&id)?;
                Ok(SetState::Usable {
                    title: set.title().to_owned(),
                    tally: pages::set_tally(&set, &progress),
                })
            });
            let state = loaded.unwrap_or_else(|error| SetState::Broken {
                problem: error.to_string(),
            });
            SetSummary { id, state }
        })
        .collect();
    Ok(Response::html(
        Status::Ok,
        pages::home(&sets, &listing.unusable),
    ))
}

/// What a form of the study page asks for.
#[derive(Clone, Copy, Debug)]
enum StudyAction {
    /// Answer the card being asked.
    Answer,
    /// Choose again after a wrong answer.
    Retry,
    /// Move on from the answered card.
    Next,
}

impl StudyAction {
    /// Returns the action that the last segment of a form's path names.
    fn from_segment(segment: &str) -> Result<Self, Failure> {
        match segment {
            "answer" => Ok(Self::Answer),
            "retry" => Ok(Self::Retry),
            "next" => Ok(Self::Next),
            _ => Err(Failure::NotFound),
        }
    }
}

/// Handles the study page of a set and the forms on it.
fn study_route(
    store: &Store,
    random: &mut dyn RandomSource,
    request: &Request,
    set: &SetId,
    rest: &[&str],
) -> Result<Response, Failure> {
    let reading = matches!(request.method(), Method::Get | Method::Head);
    let (scope, action) = match (reading, rest) {
        (true, []) => (Scope::All, None),
        (true, [topic]) => (Scope::Topic(id(topic)?), None),
        (false, [action]) => (Scope::All, Some(StudyAction::from_segment(action)?)),
        (false, [topic, action]) => (
            Scope::Topic(id(topic)?),
            Some(StudyAction::from_segment(action)?),
        ),
        _ => return Err(Failure::NotFound),
    };
    let loaded = store.load_set(set)?;
    let mut progress = store.load_progress(set)?;
    let before = progress.clone();
    progress
        .prepare(&loaded, &scope, random)
        .map_err(|_unknown_topic| Failure::NotFound)?;

    let response = match action {
        None => Response::html(Status::Ok, pages::study(&loaded, &progress, &scope)),
        Some(action) => {
            let here = pages::study_path(set, &scope);
            // A choice that does not fit the card on the page comes from a
            // page that is out of date; showing the current page is the
            // useful reply.
            match action {
                StudyAction::Answer => {
                    let form = posted_form(request, &here)?;
                    drop(record_answer(&loaded, &mut progress, &scope, &form, random));
                }
                StudyAction::Retry => {
                    let form = posted_form(request, &here)?;
                    drop(record_retry(&loaded, &mut progress, &scope, &form));
                }
                StudyAction::Next => progress.advance(&scope),
            }
            Response::see_other(&here)
        }
    };
    if progress != before {
        store.save_progress(set, &progress)?;
    }
    Ok(response)
}

/// Applies the choice in `form` as the answer to the card being asked.
fn record_answer(
    set: &StudySet,
    progress: &mut Progress,
    scope: &Scope,
    form: &Fields,
    random: &mut dyn RandomSource,
) -> Result<(), StudyError> {
    let View::Question { order, .. } = progress.view(set, scope) else {
        return Err(StudyError::NoQuestion);
    };
    let (card, choice) = posted_choice(form, order).ok_or(StudyError::NoQuestion)?;
    progress.answer(set, scope, &card, choice, random).map(drop)
}

/// Applies the choice in `form` as a retry on the card under review.
fn record_retry(
    set: &StudySet,
    progress: &mut Progress,
    scope: &Scope,
    form: &Fields,
) -> Result<(), StudyError> {
    let View::Answered(review) = progress.view(set, scope) else {
        return Err(StudyError::NoRetry);
    };
    let (card, choice) = posted_choice(form, review.order).ok_or(StudyError::NoRetry)?;
    progress.retry(set, scope, &card, choice)
}

/// Reads the card and the choice of a study form, or `None` when the form
/// lacks one of them.
///
/// A form names an answer by its place on the page; `order` is the order in
/// which the page shows the answers of the card.
fn posted_choice(form: &Fields, order: &[usize]) -> Option<(CardId, Choice)> {
    let card: CardId = form.get("card")?.parse().ok()?;
    let choice = match form.get("choice")? {
        "unknown" => Choice::DontKnow,
        place => {
            let place: usize = place.parse().ok()?;
            Choice::Answer(*order.get(place)?)
        }
    };
    Some((card, choice))
}

/// Handles the editor of a set and the forms on it.
fn edit_route(
    store: &Store,
    request: &Request,
    set_id: &SetId,
    rest: &[&str],
) -> Result<Response, Failure> {
    let set = store.load_set(set_id)?;
    if matches!(request.method(), Method::Get | Method::Head) {
        edit_page(&set, rest)
    } else {
        edit_action(store, request, set, rest)
    }
}

/// Renders the editor or one of its card forms.
fn edit_page(set: &StudySet, rest: &[&str]) -> Result<Response, Failure> {
    let page = match rest {
        [] => pages::editor(set),
        ["topics", topic, "cards", "new"] => {
            let topic: TopicId = id(topic)?;
            let topic = set.topic(&topic).ok_or(Failure::NotFound)?;
            pages::card_form(set, CardTarget::New(topic), &CardDraft::default(), None)
        }
        ["cards", card] => {
            let card: CardId = id(card)?;
            let (_, existing) = set.card(&card).ok_or(Failure::NotFound)?;
            pages::card_form(set, CardTarget::Existing(&card), &existing.to_draft(), None)
        }
        _ => return Err(Failure::NotFound),
    };
    Ok(Response::html(Status::Ok, page))
}

/// Applies a change posted from the editor.
fn edit_action(
    store: &Store,
    request: &Request,
    mut set: StudySet,
    rest: &[&str],
) -> Result<Response, Failure> {
    let set_id = set.id().clone();
    let editor = pages::edit_path(&set_id);
    let form = posted_form(request, &editor)?;
    let text = |name: &str| form.get(name).unwrap_or_default();
    let reject = |error: DeckError| rejected(&error, &editor);
    let confirmed = || {
        if form.get("confirm") == Some("yes") {
            Ok(())
        } else {
            Err(Failure::Rejected {
                reason: "Tick the box to confirm.".to_owned(),
                back: editor.clone(),
            })
        }
    };
    match rest {
        ["title"] => set.set_title(text("title")).map_err(reject)?,
        ["topics"] => set.add_topic(text("name")).map(drop).map_err(reject)?,
        ["topics", topic, "rename"] => {
            let topic: TopicId = id(topic)?;
            set.topic(&topic).ok_or(Failure::NotFound)?;
            set.rename_topic(&topic, text("name")).map_err(reject)?;
        }
        ["topics", topic, "delete"] => {
            let topic: TopicId = id(topic)?;
            set.topic(&topic).ok_or(Failure::NotFound)?;
            confirmed()?;
            set.remove_topic(&topic).map_err(reject)?;
        }
        ["topics", topic, "cards", "new"] => {
            let topic: TopicId = id(topic)?;
            let draft = card_draft(&form);
            if let Err(error) = set.add_card(&topic, &draft) {
                let topic = set.topic(&topic).ok_or(Failure::NotFound)?;
                return Ok(card_form_again(
                    &set,
                    CardTarget::New(topic),
                    &draft,
                    &error,
                ));
            }
        }
        ["cards", card] => {
            let card: CardId = id(card)?;
            set.card(&card).ok_or(Failure::NotFound)?;
            let draft = card_draft(&form);
            if let Err(error) = set.update_card(&card, &draft) {
                return Ok(card_form_again(
                    &set,
                    CardTarget::Existing(&card),
                    &draft,
                    &error,
                ));
            }
        }
        ["cards", card, "delete"] => {
            let card: CardId = id(card)?;
            set.card(&card).ok_or(Failure::NotFound)?;
            confirmed()?;
            set.remove_card(&card).map_err(reject)?;
        }
        ["reset"] => {
            confirmed()?;
            store.reset_progress(&set_id)?;
            return Ok(Response::see_other(&pages::set_path(&set_id)));
        }
        ["delete"] => {
            confirmed()?;
            store.delete_set(&set_id)?;
            return Ok(Response::see_other("/"));
        }
        _ => return Err(Failure::NotFound),
    }
    store.save_set(&set)?;
    Ok(Response::see_other(&editor))
}

/// Reads the fields of a card form.
fn card_draft(form: &Fields) -> CardDraft {
    CardDraft {
        question: form.get("question").unwrap_or_default().to_owned(),
        answer: form.get("answer").unwrap_or_default().to_owned(),
        wrong: form.all("wrong").map(str::to_owned).collect(),
        explanation: form.get("explanation").map(str::to_owned),
    }
}

/// Shows a card form again with what was typed and why it was rejected.
fn card_form_again(
    set: &StudySet,
    target: CardTarget<'_>,
    draft: &CardDraft,
    error: &DeckError,
) -> Response {
    let reason = if let DeckError::InvalidCard { problem, .. } = error {
        problem.to_string()
    } else {
        error.to_string()
    };
    let problem = format!("This card cannot be saved: {reason}.");
    Response::html(
        Status::BadRequest,
        pages::card_form(set, target, draft, Some(&problem)),
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::thread;

    use super::{App, STYLE_SHEET};
    use crate::http::{Request, Response, Status};
    use crate::scratch::Scratch;
    use crate::store::Store;

    /// An application on a scratch data directory.
    struct Fixture {
        app: App,
        scratch: Scratch,
    }

    impl Fixture {
        fn new() -> Self {
            let scratch = Scratch::new();
            let store = Store::open(scratch.path()).expect("the data directory can be created");
            Self {
                app: App::with_seed(store, 7),
                scratch,
            }
        }

        /// Returns the data directory.
        fn root(&self) -> &Path {
            self.scratch.path()
        }

        /// Hands the application the request written in `text`.
        fn send(&self, text: &str) -> Response {
            use crate::http::Handler as _;

            let request = Request::read(text.as_bytes()).expect("a valid request");
            self.app.handle(&request)
        }

        fn get(&self, path: &str) -> Response {
            self.send(&format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n"))
        }

        fn post(&self, path: &str, body: &str) -> Response {
            self.send(&format!(
                "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ))
        }

        /// Posts a form and returns the path it redirects to.
        fn submit(&self, path: &str, body: &str) -> String {
            let response = self.post(path, body);
            assert_eq!(
                response.status(),
                Status::SeeOther,
                "posting {body:?} to {path} redirects; got:\n{}",
                text(&response)
            );
            response
                .header("location")
                .expect("a redirect names its target")
                .to_owned()
        }

        /// Creates the set "Demo" with the single card `which`, whose answers
        /// are "Right", "Wrong A", "Wrong B", and "Wrong C".
        fn one_card(&self) {
            self.demo(0);
            self.submit(
                "/sets/demo/edit/topics/basics/cards/new",
                "question=Which%3F&answer=Right&wrong=Wrong+A&wrong=Wrong+B&wrong=Wrong+C\
                 &explanation=",
            );
        }

        /// Returns the study page of the set "Demo".
        fn study_page(&self) -> String {
            text(&self.get("/sets/demo/study"))
        }

        /// Returns the progress file of the set "Demo".
        fn stored_progress(&self) -> String {
            fs::read_to_string(self.root().join("progress").join("demo.json"))
                .expect("progress file")
        }

        /// Creates the set "Demo" with one topic and `cards` cards.
        fn demo(&self, cards: usize) {
            assert_eq!(
                self.submit("/sets", "title=Demo"),
                "/sets/demo/edit",
                "the set is created"
            );
            self.submit("/sets/demo/edit/topics", "name=Basics");
            for number in 0..cards {
                self.submit(
                    "/sets/demo/edit/topics/basics/cards/new",
                    &format!(
                        "question=Question+{number}&answer=Right+{number}&wrong=Wrong+{number}\
                         &wrong=&explanation="
                    ),
                );
            }
        }
    }

    fn text(response: &Response) -> String {
        String::from_utf8(response.body().to_vec()).expect("pages are UTF-8")
    }

    /// Returns the value of the first attribute `name="…"` after `marker`.
    fn attribute_after<'a>(page: &'a str, marker: &str, name: &str) -> &'a str {
        let (_, rest) = page
            .split_once(marker)
            .unwrap_or_else(|| panic!("{marker:?} not found in:\n{page}"));
        let (_, rest) = rest
            .split_once(&format!("{name}=\""))
            .unwrap_or_else(|| panic!("no {name} attribute after {marker:?}"));
        rest.split_once('"')
            .expect("the attribute value is closed")
            .0
    }

    /// Returns the `value` of the answer button that shows `answer`.
    fn shown_position<'a>(page: &'a str, answer: &str) -> &'a str {
        let button = page
            .split("<button")
            .find(|button| {
                button
                    .split("</button>")
                    .next()
                    .is_some_and(|inner| inner.contains(answer))
            })
            .unwrap_or_else(|| panic!("no button shows {answer:?} in:\n{page}"));
        attribute_after(button, "name=\"choice\"", "value")
    }

    #[test]
    fn the_home_page_starts_empty() {
        let fixture = Fixture::new();
        let response = fixture.get("/");
        assert_eq!(response.status(), Status::Ok, "status");
        assert_eq!(
            response.header("content-type"),
            Some("text/html; charset=utf-8"),
            "media type"
        );
        assert!(text(&response).contains("No study sets yet"), "empty state");
    }

    #[test]
    fn assets_are_served_with_their_media_types() {
        let fixture = Fixture::new();
        let css = fixture.get("/assets/app.css");
        assert_eq!(
            css.header("content-type"),
            Some("text/css; charset=utf-8"),
            "style sheet type"
        );
        assert!(!css.body().is_empty(), "the style sheet has content");
        let script = fixture.get("/assets/app.js");
        assert_eq!(
            script.header("content-type"),
            Some("text/javascript; charset=utf-8"),
            "script type"
        );
        assert!(
            text(&script).contains("data-key"),
            "the script reads key bindings from the page"
        );
        let icon = fixture.get("/assets/icon.svg");
        assert_eq!(
            icon.header("content-type"),
            Some("image/svg+xml"),
            "icon type"
        );
        assert!(text(&icon).starts_with("<svg "), "the icon is an SVG image");
    }

    #[test]
    fn the_style_sheet_marks_an_answer_with_a_selector_that_outranks_the_plain_answer() {
        // The rule for every answer is `ol.answers li.answer`. A rule for a
        // right or wrong answer that began with `li.answer` would lose to it,
        // and the mark would not be shown.
        let marks: Vec<&str> = STYLE_SHEET
            .lines()
            .filter(|line| line.contains("li.answer."))
            .collect();
        assert!(
            marks.iter().any(|line| line.contains("li.answer.right"))
                && marks.iter().any(|line| line.contains("li.answer.wrong")),
            "the style sheet marks right and wrong answers"
        );
        for line in marks {
            assert!(
                line.starts_with("ol.answers li.answer."),
                "a selector that names the list: {line}"
            );
        }
    }

    #[test]
    fn unknown_addresses_are_not_found() {
        let fixture = Fixture::new();
        for path in [
            "/nope",
            "/assets/other.css",
            "/sets",
            "/sets/missing",
            "/sets/missing/study",
            "/sets/missing/edit",
            "/sets/Not%20An%20Id",
            "/sets/..%2f..%2fetc/edit",
        ] {
            assert_eq!(fixture.get(path).status(), Status::NotFound, "GET {path}");
        }
        fixture.demo(1);
        for path in [
            "/sets/demo/other",
            "/sets/demo/study/missing-topic",
            "/sets/demo/study/basics/extra",
            "/sets/demo/edit/cards/missing",
            "/sets/demo/edit/topics/missing/cards/new",
            "/sets/demo/edit/title",
        ] {
            assert_eq!(fixture.get(path).status(), Status::NotFound, "GET {path}");
        }
        for path in [
            "/",
            "/sets/demo",
            "/sets/demo/study/skip",
            "/sets/demo/study/basics/skip",
            "/sets/demo/study/missing-topic/retry",
            "/sets/demo/study/extra/answer/more",
            "/sets/demo/study/extra/retry/more",
            "/sets/demo/edit/unknown",
        ] {
            assert_eq!(
                fixture.post(path, "").status(),
                Status::NotFound,
                "POST {path}"
            );
        }
    }

    #[test]
    fn a_set_is_built_in_the_editor_and_stored_as_a_file() {
        let fixture = Fixture::new();
        fixture.demo(2);
        fixture.submit("/sets/demo/edit/title", "title=Demo+%26+more");
        fixture.submit("/sets/demo/edit/topics/basics/rename", "name=The+basics");

        let file =
            fs::read_to_string(fixture.root().join("sets").join("demo.toml")).expect("set file");
        assert!(
            file.contains("title = \"Demo & more\""),
            "the title is stored:\n{file}"
        );
        assert!(
            file.contains("name = \"The basics\""),
            "the topic name is stored:\n{file}"
        );
        assert!(
            file.contains("question = \"Question 1\""),
            "cards are stored:\n{file}"
        );
        assert!(
            file.contains("wrong = [\"Wrong 1\"]"),
            "blank wrong answers are dropped:\n{file}"
        );

        let editor = text(&fixture.get("/sets/demo/edit"));
        assert!(
            editor.contains("Demo &amp; more"),
            "the editor shows the new title:\n{editor}"
        );
        assert!(
            editor.contains("href=\"/sets/demo/edit/cards/question-0\""),
            "cards are listed:\n{editor}"
        );
        let home = text(&fixture.get("/"));
        assert!(
            home.contains("2 cards · 0% seen · 0% aptitude"),
            "the home page summarises the set:\n{home}"
        );
        let overview = text(&fixture.get("/sets/demo"));
        assert!(
            overview.contains("The basics"),
            "the overview lists the topic:\n{overview}"
        );
    }

    #[test]
    fn rejected_forms_explain_why() {
        let fixture = Fixture::new();
        let blank = fixture.post("/sets", "title=+++");
        assert_eq!(
            blank.status(),
            Status::BadRequest,
            "a blank set title is rejected"
        );
        assert!(
            text(&blank).contains("The set title is empty."),
            "the reason is shown:\n{}",
            text(&blank)
        );

        fixture.demo(1);
        let topic = fixture.post("/sets/demo/edit/topics", "name=");
        assert_eq!(
            topic.status(),
            Status::BadRequest,
            "a blank topic name is rejected"
        );
        assert!(
            text(&topic).contains("A topic name is empty."),
            "the reason is shown"
        );
        assert!(
            text(&topic).contains("href=\"/sets/demo/edit\""),
            "the page links back to the editor"
        );

        let card = fixture.post(
            "/sets/demo/edit/topics/basics/cards/new",
            "question=Why%3F&answer=Because&wrong=Because&explanation=",
        );
        assert_eq!(
            card.status(),
            Status::BadRequest,
            "a duplicate answer is rejected"
        );
        let page = text(&card);
        assert!(
            page.contains("This card cannot be saved: the answer \"Because\" appears twice."),
            "reason:\n{page}"
        );
        assert!(
            page.contains("Why?</textarea>"),
            "the question is kept:\n{page}"
        );
        assert!(
            page.contains("name=\"answer\" value=\"Because\""),
            "the answer is kept:\n{page}"
        );

        let unconfirmed = fixture.post("/sets/demo/edit/delete", "");
        assert_eq!(
            unconfirmed.status(),
            Status::BadRequest,
            "deleting needs confirmation"
        );
        assert!(
            text(&unconfirmed).contains("Tick the box to confirm."),
            "the reason is shown"
        );
        assert_eq!(
            fixture.get("/sets/demo").status(),
            Status::Ok,
            "the set still exists"
        );
    }

    #[test]
    fn a_rejected_change_to_a_card_shows_its_form_again() {
        let fixture = Fixture::new();
        fixture.demo(1);
        let update = fixture.post(
            "/sets/demo/edit/cards/question-0",
            "question=Reworded&answer=Yes&wrong=&explanation=",
        );
        assert_eq!(
            update.status(),
            Status::BadRequest,
            "a card without a wrong answer is rejected"
        );
        let again = text(&update);
        assert!(
            again.contains(
                "This card cannot be saved: it has 0 wrong answers; a card needs 1 to 5."
            ),
            "reason:\n{again}"
        );
        assert!(
            again.contains("action=\"/sets/demo/edit/cards/question-0\"")
                && again.contains("Reworded</textarea>"),
            "the form of the card is shown again with what was typed:\n{again}"
        );
        assert!(
            text(&fixture.get("/sets/demo/edit/cards/question-0"))
                .contains("Question 0</textarea>"),
            "the rejected update did not reach the card"
        );
    }

    #[test]
    fn a_body_that_is_not_a_form_is_rejected() {
        let fixture = Fixture::new();
        fixture.demo(0);
        let unreadable = fixture.post("/sets/demo/edit/title", "title=%");
        assert_eq!(
            unreadable.status(),
            Status::BadRequest,
            "a body that is not a form is rejected"
        );
        assert!(
            text(&unreadable).contains("The form could not be read."),
            "the reason is shown"
        );
        assert!(
            text(&unreadable).contains("href=\"/sets/demo/edit\""),
            "the page links back to the editor"
        );
        let file =
            fs::read_to_string(fixture.root().join("sets").join("demo.toml")).expect("set file");
        assert!(
            file.contains("title = \"Demo\""),
            "the title is as it was:\n{file}"
        );
    }

    #[test]
    fn cards_and_topics_are_edited_and_removed() {
        let fixture = Fixture::new();
        fixture.demo(2);
        let form = text(&fixture.get("/sets/demo/edit/cards/question-0"));
        assert!(
            form.contains("Question 0</textarea>"),
            "the form shows the card:\n{form}"
        );
        let blank = text(&fixture.get("/sets/demo/edit/topics/basics/cards/new"));
        assert!(
            blank.contains("action=\"/sets/demo/edit/topics/basics/cards/new\"")
                && blank.contains("<h1>New card in Basics</h1>"),
            "the form for a card to add names its topic:\n{blank}"
        );

        fixture.submit(
            "/sets/demo/edit/cards/question-0",
            "question=Reworded&answer=Yes&wrong=No&wrong=Maybe&explanation=Because.",
        );
        let file =
            fs::read_to_string(fixture.root().join("sets").join("demo.toml")).expect("set file");
        assert!(
            file.contains("id = \"question-0\"\nquestion = \"Reworded\""),
            "the id is kept:\n{file}"
        );
        assert!(
            file.contains("explanation = \"Because.\""),
            "the explanation is stored:\n{file}"
        );

        assert_eq!(
            fixture
                .post("/sets/demo/edit/cards/question-1/delete", "")
                .status(),
            Status::BadRequest,
            "deleting a card needs confirmation"
        );
        fixture.submit("/sets/demo/edit/cards/question-1/delete", "confirm=yes");
        assert_eq!(
            fixture.get("/sets/demo/edit/cards/question-1").status(),
            Status::NotFound,
            "the card is gone"
        );

        fixture.submit("/sets/demo/edit/topics/basics/delete", "confirm=yes");
        let editor = text(&fixture.get("/sets/demo/edit"));
        assert!(
            !editor.contains("Reworded"),
            "the topic and its cards are gone:\n{editor}"
        );

        assert_eq!(
            fixture.submit("/sets/demo/edit/delete", "confirm=yes"),
            "/",
            "deleting leads home"
        );
        assert_eq!(
            fixture.get("/sets/demo").status(),
            Status::NotFound,
            "the set is gone"
        );
    }

    #[test]
    fn studying_asks_judges_and_moves_on() {
        let fixture = Fixture::new();
        fixture.demo(12);
        let page = text(&fixture.get("/sets/demo/study"));
        let card = attribute_after(&page, "name=\"card\"", "value").to_owned();
        assert!(
            page.contains("Type the letter or click an answer."),
            "a question is shown:\n{page}"
        );

        let number = card.trim_start_matches("question-");
        let right = shown_position(&page, &format!("Right {number}"));
        let target = fixture.submit(
            "/sets/demo/study/answer",
            &format!("card={card}&choice={right}"),
        );
        assert_eq!(
            target, "/sets/demo/study",
            "answering leads back to the study page"
        );
        let reviewed = text(&fixture.get("/sets/demo/study"));
        assert!(
            reviewed.contains("<li class=\"answer chosen right\">"),
            "the chosen answer is marked as correct:\n{reviewed}"
        );
        assert!(
            reviewed.contains("<dd>8%</dd><dt>Seen</dt>"),
            "one of twelve cards is seen:\n{reviewed}"
        );

        // Reloading or repeating the answer changes nothing.
        fixture.submit(
            "/sets/demo/study/answer",
            &format!("card={card}&choice={right}"),
        );
        assert_eq!(
            text(&fixture.get("/sets/demo/study")),
            reviewed,
            "a repeated answer is ignored"
        );

        fixture.submit("/sets/demo/study/next", "");
        let next = text(&fixture.get("/sets/demo/study"));
        let next_card = attribute_after(&next, "name=\"card\"", "value");
        assert_ne!(next_card, card, "the next card is asked");

        fixture.submit(
            "/sets/demo/study/answer",
            &format!("card={next_card}&choice=unknown"),
        );
        let declined = text(&fixture.get("/sets/demo/study"));
        assert!(
            declined.contains("<li class=\"answer right\">"),
            "declining marks the correct answer in the list:\n{declined}"
        );

        let progress = fs::read_to_string(fixture.root().join("progress").join("demo.json"))
            .expect("progress file");
        assert!(
            progress.contains(&format!("\"{card}\":[\"correct\"]")),
            "the answer is stored:\n{progress}"
        );
        assert!(
            progress.contains(&format!("\"{next_card}\":[\"dont_know\"]")),
            "declining is stored:\n{progress}"
        );
    }

    #[test]
    fn a_wrong_answer_is_retried_until_the_correct_one_is_chosen() {
        let fixture = Fixture::new();
        fixture.one_card();
        let question = fixture.study_page();
        let choosing =
            |answer: &str| format!("card=which&choice={}", shown_position(&question, answer));

        fixture.submit("/sets/demo/study/answer", &choosing("Wrong A"));
        let missed = fixture.study_page();
        assert_eq!(
            missed.matches("class=\"answer chosen wrong\"").count(),
            1,
            "the chosen answer is marked wrong:\n{missed}"
        );
        for words in [">Correct<", "Incorrect", "The correct answer"] {
            assert!(
                !missed.contains(words),
                "only the mark tells how the card was answered, not {words:?}:\n{missed}"
            );
        }
        assert!(
            missed.contains("action=\"/sets/demo/study/retry\""),
            "the page offers retries:\n{missed}"
        );
        assert_eq!(
            shown_position(&missed, "Wrong B"),
            shown_position(&question, "Wrong B"),
            "an answer keeps its place for a retry"
        );

        assert_eq!(
            fixture.submit("/sets/demo/study/retry", &choosing("Wrong B")),
            "/sets/demo/study",
            "a retry leads back to the study page"
        );
        let narrowed = fixture.study_page();
        assert_eq!(
            narrowed.matches("class=\"answer chosen wrong\"").count(),
            2,
            "the answer and the retry are marked wrong:\n{narrowed}"
        );
        for mark in ["class=\"answer right\"", "class=\"answer chosen right\""] {
            assert!(
                !narrowed.contains(mark),
                "the correct answer is still hidden, without {mark}:\n{narrowed}"
            );
        }

        fixture.submit("/sets/demo/study/retry", &choosing("Right"));
        let found = fixture.study_page();
        assert!(
            found.contains("class=\"answer chosen right\""),
            "the correct answer is marked:\n{found}"
        );
        assert_eq!(
            found.matches("class=\"answer chosen wrong\"").count(),
            2,
            "the wrong choices stay marked:\n{found}"
        );
        assert!(
            !found.contains("/study/retry"),
            "a review that shows the correct answer offers no retry:\n{found}"
        );
        fixture.submit("/sets/demo/study/retry", &choosing("Wrong C"));
        assert_eq!(
            fixture.study_page(),
            found,
            "a retry after the correct answer is ignored"
        );

        let progress = fixture.stored_progress();
        assert!(
            progress.contains("\"which\":[\"wrong\"]"),
            "only the answer is in the history:\n{progress}"
        );
        assert!(
            found.contains("<dd>13%</dd><dt>Aptitude</dt>"),
            "the aptitude is that of one wrong answer:\n{found}"
        );

        fixture.submit("/sets/demo/study/next", "");
        assert!(
            fixture
                .study_page()
                .contains("Type the letter or click an answer."),
            "the card is asked again after the review"
        );
        let moved_on = fixture.stored_progress();
        assert!(
            !moved_on.contains("retries"),
            "the retries are forgotten with the review:\n{moved_on}"
        );
    }

    #[test]
    fn the_correct_answer_is_shown_on_request_after_a_wrong_one() {
        let fixture = Fixture::new();
        fixture.one_card();
        let question = fixture.study_page();
        let wrong = shown_position(&question, "Wrong A");
        fixture.submit(
            "/sets/demo/study/answer",
            &format!("card=which&choice={wrong}"),
        );
        assert!(
            fixture.study_page().contains(
                "<button type=\"submit\" name=\"choice\" value=\"unknown\" data-key=\"s\" \
                 class=\"reveal\">Show correct answer</button>"
            ),
            "the page offers to show the correct answer"
        );

        fixture.submit("/sets/demo/study/retry", "card=which&choice=unknown");
        let shown = fixture.study_page();
        assert!(
            shown.contains("<li class=\"answer chosen wrong\">"),
            "the wrong answer stays marked:\n{shown}"
        );
        assert!(
            shown.contains("<li class=\"answer right\">"),
            "the correct answer is marked in the list:\n{shown}"
        );
        assert!(
            !shown.contains("/study/retry"),
            "a review that shows the correct answer offers no retry:\n{shown}"
        );
        let progress = fixture.stored_progress();
        assert!(
            progress.contains("\"which\":[\"wrong\"]"),
            "the answer stays a wrong one in the history:\n{progress}"
        );
    }

    #[test]
    fn a_form_that_does_not_fit_the_page_changes_nothing() {
        let fixture = Fixture::new();
        fixture.one_card();
        let question = fixture.study_page();
        let choosing =
            |answer: &str| format!("card=which&choice={}", shown_position(&question, answer));

        fixture.submit("/sets/demo/study/retry", &choosing("Right"));
        assert_eq!(
            fixture.study_page(),
            question,
            "a retry is not taken for the answer to the card being asked"
        );
        assert!(
            fixture.stored_progress().contains("\"cards\":{}"),
            "the retry left no answer in the history"
        );

        fixture.submit("/sets/demo/study/answer", &choosing("Wrong A"));
        let missed = fixture.study_page();
        let stored = fixture.stored_progress();
        fixture.submit("/sets/demo/study/answer", &choosing("Right"));
        assert_eq!(
            fixture.study_page(),
            missed,
            "an answer is not taken for a retry on the card under review"
        );
        for body in [
            "card=other&choice=0",
            "card=Not+An+Id&choice=0",
            "choice=0",
            "card=which&choice=4",
            "card=which&choice=-1",
            "card=which&choice=first",
            "card=which",
            "",
        ] {
            fixture.submit("/sets/demo/study/retry", body);
            assert_eq!(
                fixture.study_page(),
                missed,
                "the retry {body:?} is ignored"
            );
        }
        assert_eq!(
            fixture.stored_progress(),
            stored,
            "nothing was written for the forms that were ignored"
        );
    }

    #[test]
    fn a_topic_is_studied_on_its_own_page() {
        let fixture = Fixture::new();
        fixture.demo(3);
        fixture.submit("/sets/demo/edit/topics", "name=Other");
        fixture.submit(
            "/sets/demo/edit/topics/other/cards/new",
            "question=Elsewhere&answer=Yes&wrong=No&explanation=",
        );
        let page = text(&fixture.get("/sets/demo/study/other"));
        assert_eq!(
            attribute_after(&page, "name=\"card\"", "value"),
            "elsewhere",
            "the topic's card is asked"
        );
        assert!(
            page.contains("action=\"/sets/demo/study/other/answer\""),
            "answers go to the topic:\n{page}"
        );
        let target = fixture.submit(
            "/sets/demo/study/other/answer",
            "card=elsewhere&choice=unknown",
        );
        assert_eq!(
            target, "/sets/demo/study/other",
            "answering stays on the topic"
        );
        fixture.submit("/sets/demo/study/other/next", "");
        let again = text(&fixture.get("/sets/demo/study/other"));
        assert_eq!(
            attribute_after(&again, "name=\"card\"", "value"),
            "elsewhere",
            "the only card returns"
        );

        fixture.submit(
            "/sets/demo/study/other/answer",
            &format!("card=elsewhere&choice={}", shown_position(&again, "No")),
        );
        let missed = text(&fixture.get("/sets/demo/study/other"));
        assert!(
            missed.contains("action=\"/sets/demo/study/other/retry\""),
            "retries go to the topic:\n{missed}"
        );
        assert_eq!(
            fixture.submit(
                "/sets/demo/study/other/retry",
                &format!("card=elsewhere&choice={}", shown_position(&again, "Yes")),
            ),
            "/sets/demo/study/other",
            "a retry stays on the topic"
        );
        let found = text(&fixture.get("/sets/demo/study/other"));
        assert!(
            found.contains("class=\"answer chosen right\""),
            "the retry is applied to the review of the topic:\n{found}"
        );
    }

    #[test]
    fn progress_can_be_reset() {
        let fixture = Fixture::new();
        fixture.demo(2);
        let page = text(&fixture.get("/sets/demo/study"));
        let card = attribute_after(&page, "name=\"card\"", "value").to_owned();
        fixture.submit(
            "/sets/demo/study/answer",
            &format!("card={card}&choice=unknown"),
        );
        assert!(
            text(&fixture.get("/sets/demo")).contains("50% seen"),
            "one of two cards is seen"
        );
        assert_eq!(
            fixture.submit("/sets/demo/edit/reset", "confirm=yes"),
            "/sets/demo",
            "leads to the overview"
        );
        assert!(
            text(&fixture.get("/sets/demo")).contains("0% seen"),
            "nothing is seen after a reset"
        );
    }

    #[test]
    fn a_broken_set_file_is_reported_without_hiding_the_others() {
        let fixture = Fixture::new();
        fixture.demo(1);
        fs::write(fixture.root().join("sets").join("broken.toml"), "title = ")
            .expect("file written");
        let home = text(&fixture.get("/"));
        assert!(
            home.contains("broken.toml"),
            "the broken file is named:\n{home}"
        );
        assert!(
            home.contains("href=\"/sets/demo\""),
            "the working set is still listed:\n{home}"
        );
        let direct = fixture.get("/sets/broken");
        assert_eq!(
            direct.status(),
            Status::InternalServerError,
            "opening the broken set fails"
        );
        assert!(
            text(&direct).contains("broken.toml"),
            "the failure names the file"
        );
    }

    #[test]
    fn nothing_is_answered_after_a_request_panicked() {
        let fixture = Fixture::new();
        fixture.demo(1);
        let outcome = thread::scope(|scope| {
            let request = scope.spawn(|| {
                let _guard = fixture.app.random.lock();
                panic!("a request panics while it holds the lock");
            });
            request.join()
        });
        assert!(outcome.is_err(), "the thread holding the lock panicked");

        for path in ["/", "/sets/demo", "/sets/demo/study", "/assets/app.css"] {
            let response = fixture.get(path);
            assert_eq!(
                response.status(),
                Status::InternalServerError,
                "GET {path} is refused"
            );
            assert_eq!(
                text(&response),
                "A request failed unexpectedly. Restart studycards to continue.",
                "GET {path} says what to do"
            );
        }
        assert_eq!(
            fixture
                .post("/sets/demo/edit/title", "title=Changed")
                .status(),
            Status::InternalServerError,
            "changes are refused"
        );
        let file =
            fs::read_to_string(fixture.root().join("sets").join("demo.toml")).expect("set file");
        assert!(
            file.contains("title = \"Demo\""),
            "the refused change did not reach the file:\n{file}"
        );
    }
}
