//! Study sets and progress as files in a data directory.
//!
//! A [`Store`] keeps one TOML file per study set in `sets/` and one JSON file
//! of progress per set in `progress/`, both named after the set's identifier.
//! Files are replaced by writing a temporary file and renaming it, so an
//! interrupted write leaves the previous content in place.
//!
//! The files of the data directory are treated as untrusted input. Only
//! regular files are read, and a temporary file is created under a name that
//! did not exist, so a symbolic link or a named pipe put into the directory is
//! neither written through nor waited on. A file is read whole, whatever its
//! size.
//!
//! # Examples
//!
//! Creating a set in a data directory that holds none. The example is not run
//! as a test, because it writes to the directory `data`.
//!
//! ```no_run
//! use std::path::Path;
//!
//! use studycards::store::Store;
//! use studycards::study::Progress;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let store = Store::open(Path::new("data"))?;
//! let set = store.create_set("Solar system")?;
//! assert_eq!(set.id().as_str(), "solar-system");
//! assert_eq!(store.list()?.sets, [set.id().clone()]);
//! assert_eq!(store.load_progress(set.id())?, Progress::new());
//! # Ok(())
//! # }
//! ```

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind};
use std::iter;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::deck::{self, DeckError, SetId, StudySet};
use crate::study::Progress;

/// Why a file operation failed.
#[derive(Debug)]
pub enum StoreError {
    /// Reading or writing `path` failed.
    Io {
        /// The file or directory involved.
        path: PathBuf,
        /// The underlying error.
        source: io::Error,
    },
    /// The file of a set does not hold a valid study set.
    InvalidSet {
        /// The set's file.
        path: PathBuf,
        /// What is wrong with it.
        source: DeckError,
    },
    /// The progress file of a set cannot be understood.
    InvalidProgress {
        /// The progress file.
        path: PathBuf,
        /// The parser's description of the problem.
        message: String,
    },
    /// There is no set with this identifier.
    UnknownSet(SetId),
}

impl Display for StoreError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::InvalidSet { path, source } => write!(f, "{}: {source}", path.display()),
            Self::InvalidProgress { path, message } => {
                write!(f, "{}: not a progress file: {message}", path.display())
            }
            Self::UnknownSet(id) => write!(f, "there is no study set \"{id}\""),
        }
    }
}

impl Error for StoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidSet { source, .. } => Some(source),
            Self::InvalidProgress { .. } | Self::UnknownSet(_) => None,
        }
    }
}

/// The study sets found in a data directory.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Listing {
    /// Identifiers of the sets, sorted.
    pub sets: Vec<SetId>,
    /// Names of `.toml` files whose stem cannot be used as an identifier.
    pub unusable: Vec<String>,
}

/// A data directory holding study sets and progress.
#[derive(Clone, Debug)]
pub struct Store {
    /// The directory of set files.
    sets: PathBuf,
    /// The directory of progress files.
    progress: PathBuf,
}

impl Store {
    /// Opens the data directory `root`, creating it and its `sets` and
    /// `progress` subdirectories when missing.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] when a directory cannot be created.
    pub fn open(root: &Path) -> Result<Self, StoreError> {
        let store = Self {
            sets: root.join("sets"),
            progress: root.join("progress"),
        };
        for directory in [&store.sets, &store.progress] {
            fs::create_dir_all(directory).map_err(io_error(directory))?;
        }
        Ok(store)
    }

    /// Lists the study sets in the directory.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] when the directory cannot be read.
    pub fn list(&self) -> Result<Listing, StoreError> {
        let mut listing = Listing::default();
        for entry in fs::read_dir(&self.sets).map_err(io_error(&self.sets))? {
            let path = entry.map_err(io_error(&self.sets))?.path();
            if path.extension().is_none_or(|extension| extension != "toml") || !path.is_file() {
                continue;
            }
            let id: Option<SetId> = path
                .file_stem()
                .and_then(|stem| stem.to_str()?.parse().ok());
            match id {
                Some(id) => listing.sets.push(id),
                None => listing.unusable.push(
                    path.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                ),
            }
        }
        listing.sets.sort();
        listing.unusable.sort();
        Ok(listing)
    }

    /// Reads a study set.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::UnknownSet`] when the set has no file,
    /// [`StoreError::Io`] when the file cannot be read, and
    /// [`StoreError::InvalidSet`] when its content is not a valid set.
    pub fn load_set(&self, id: &SetId) -> Result<StudySet, StoreError> {
        let path = self.set_path(id);
        let text = match read_text(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(StoreError::UnknownSet(id.clone()));
            }
            Err(error) => {
                return Err(StoreError::Io {
                    path,
                    source: error,
                });
            }
        };
        StudySet::from_toml(id.clone(), &text)
            .map_err(|source| StoreError::InvalidSet { path, source })
    }

    /// Writes a study set, replacing its file.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::InvalidSet`] when the set cannot be written as
    /// TOML and [`StoreError::Io`] when the file cannot be written.
    pub fn save_set(&self, set: &StudySet) -> Result<(), StoreError> {
        let path = self.set_path(set.id());
        let text = set.to_toml().map_err(|source| StoreError::InvalidSet {
            path: path.clone(),
            source,
        })?;
        replace_file(&path, text.as_bytes())
    }

    /// Creates an empty study set titled `title`, writes its file, and
    /// returns it. Its identifier is derived from the title and differs from
    /// the identifier of every set in the store.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::InvalidSet`] when the title is unusable and
    /// [`StoreError::Io`] when the file cannot be written.
    pub fn create_set(&self, title: &str) -> Result<StudySet, StoreError> {
        let invalid = |source| StoreError::InvalidSet {
            path: self.sets.clone(),
            source,
        };
        let id = deck::set_id_for_title(title, |candidate| {
            self.sets.join(format!("{candidate}.toml")).exists()
        })
        .map_err(invalid)?;
        let set = StudySet::try_new(id, title).map_err(invalid)?;
        self.save_set(&set)?;
        Ok(set)
    }

    /// Deletes a study set and its progress.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::UnknownSet`] when the set has no file and
    /// [`StoreError::Io`] when a file cannot be removed.
    pub fn delete_set(&self, id: &SetId) -> Result<(), StoreError> {
        let path = self.set_path(id);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(StoreError::UnknownSet(id.clone()));
            }
            Err(error) => {
                return Err(StoreError::Io {
                    path,
                    source: error,
                });
            }
        }
        self.reset_progress(id)
    }

    /// Reads the progress of a set. A set without a progress file has not
    /// been studied.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] when the file cannot be read and
    /// [`StoreError::InvalidProgress`] when its content cannot be understood.
    pub fn load_progress(&self, id: &SetId) -> Result<Progress, StoreError> {
        let path = self.progress_path(id);
        let text = match read_text(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Progress::new()),
            Err(error) => {
                return Err(StoreError::Io {
                    path,
                    source: error,
                });
            }
        };
        serde_json::from_str(&text).map_err(|error| StoreError::InvalidProgress {
            path,
            message: progress_problem(&error),
        })
    }

    /// Writes the progress of a set, replacing its file.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::InvalidProgress`] when the progress cannot be
    /// written as JSON and [`StoreError::Io`] when the file cannot be written.
    pub fn save_progress(&self, id: &SetId, progress: &Progress) -> Result<(), StoreError> {
        let path = self.progress_path(id);
        let text =
            serde_json::to_string(progress).map_err(|error| StoreError::InvalidProgress {
                path: path.clone(),
                message: progress_problem(&error),
            })?;
        replace_file(&path, text.as_bytes())
    }

    /// Forgets the progress of a set.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] when the progress file cannot be removed.
    pub fn reset_progress(&self, id: &SetId) -> Result<(), StoreError> {
        let path = self.progress_path(id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(StoreError::Io {
                path,
                source: error,
            }),
        }
    }

    /// Returns the file of a set. Identifiers contain no path separators, so
    /// the file is always directly inside the sets directory.
    fn set_path(&self, id: &SetId) -> PathBuf {
        self.sets.join(format!("{id}.toml"))
    }

    /// Returns the progress file of a set.
    fn progress_path(&self, id: &SetId) -> PathBuf {
        self.progress.join(format!("{id}.json"))
    }
}

/// Returns a closure that wraps an I/O error with the path it concerns.
fn io_error(path: &Path) -> impl Fn(io::Error) -> StoreError + '_ {
    move |source| StoreError::Io {
        path: path.to_owned(),
        source,
    }
}

/// The number of names tried for a temporary file.
const TEMPORARY_NAMES: usize = 16;
/// The most characters of the JSON parser's message that an error keeps. The
/// message can quote the file.
const MESSAGE_CHARS: usize = 300;

/// Describes why progress could not be read or written, in a text that stays
/// short whatever the file holds.
fn progress_problem(error: &serde_json::Error) -> String {
    let message = error.to_string();
    if message.chars().nth(MESSAGE_CHARS).is_none() {
        return message;
    }
    let start: String = message.chars().take(MESSAGE_CHARS).collect();
    format!(
        "{start}… at line {} column {}",
        error.line(),
        error.column()
    )
}

/// Reads a file of the data directory as text.
///
/// Only a regular file is read. Reading a named pipe would never end, and a
/// directory or a device holds nothing that the program wrote.
fn read_text(path: &Path) -> io::Result<String> {
    if !fs::metadata(path)?.is_file() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    fs::read_to_string(path)
}

/// Replaces the content of `path` without leaving a partly written file.
/// The directory is created when it is missing.
///
/// The content goes to a temporary file beside `path`, which is then renamed
/// over `path`. Renaming replaces a symbolic link at `path` instead of
/// writing to the file that the link names.
fn replace_file(path: &Path, content: &[u8]) -> Result<(), StoreError> {
    use std::io::Write as _;

    let write = || {
        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory)?;
        }
        let (mut file, temporary) = create_exclusive(temporary_names(path))?;
        let replaced = file
            .write_all(content)
            .and_then(|()| file.sync_all())
            .and_then(|()| fs::rename(&temporary, path));
        if replaced.is_err() {
            // Best effort: the temporary file is this call's own, and a
            // leftover one is never read.
            drop(fs::remove_file(&temporary));
        }
        replaced
    };
    write().map_err(io_error(path))
}

/// Returns names for a temporary file beside `path`. The names differ between
/// the processes that run at one time and between the calls of one process.
fn temporary_names(path: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    static NAMED: AtomicU64 = AtomicU64::new(0);
    let name = move || {
        let serial = NAMED.fetch_add(1, Ordering::Relaxed);
        let mut name = path.as_os_str().to_owned();
        name.push(format!(".{}.{serial}.tmp", process::id()));
        PathBuf::from(name)
    };
    iter::repeat_with(name).take(TEMPORARY_NAMES)
}

/// Creates the first of `candidates` that does not exist and returns it, open
/// for writing, with its path.
///
/// A candidate that exists is passed over, whatever it is: nothing is written
/// to a file, or through a symbolic link, that was there before.
fn create_exclusive(candidates: impl IntoIterator<Item = PathBuf>) -> io::Result<(File, PathBuf)> {
    let mut taken = None;
    for candidate in candidates {
        let created = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate);
        match created {
            Ok(file) => return Ok((file, candidate)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => taken = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(taken.unwrap_or_else(|| ErrorKind::AlreadyExists.into()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::ErrorKind;

    use super::{Listing, Store, StoreError, create_exclusive};
    use crate::deck::{CardDraft, DeckError, SetId, TextProblem};
    use crate::scratch::Scratch;
    use crate::study::{Choice, Progress, Scope, View};

    fn id(text: &str) -> SetId {
        text.parse().expect("valid set id")
    }

    #[test]
    fn open_creates_the_directories() {
        let scratch = Scratch::new();
        let root = scratch.path().join("nested").join("data");
        let store = Store::open(&root).expect("the directories can be created");
        assert!(root.join("sets").is_dir(), "the sets directory exists");
        assert!(
            root.join("progress").is_dir(),
            "the progress directory exists"
        );
        assert_eq!(
            store.list().expect("listing works"),
            Listing::default(),
            "a new store is empty"
        );
        Store::open(&root).expect("opening an existing directory works");
    }

    #[test]
    fn sets_are_created_saved_and_loaded() {
        let scratch = Scratch::new();
        let store = Store::open(scratch.path()).expect("store opens");
        let mut set = store
            .create_set("Solar System")
            .expect("the set is created");
        assert_eq!(set.id(), &id("solar-system"), "the id comes from the title");
        let again = store
            .create_set("Solar system")
            .expect("a second set is created");
        assert_eq!(
            again.id(),
            &id("solar-system-2"),
            "an existing id is not reused"
        );

        let topic = set.add_topic("Planets").expect("valid topic");
        let draft = CardDraft {
            question: "Closest to the Sun?".to_owned(),
            answer: "Mercury".to_owned(),
            wrong: vec!["Venus".to_owned()],
            explanation: None,
        };
        set.add_card(&topic, &draft).expect("valid card");
        store.save_set(&set).expect("the set is saved");

        assert_eq!(
            store.load_set(set.id()).expect("the set loads"),
            set,
            "the saved set is read back"
        );
        assert_eq!(
            store.list().expect("listing works").sets,
            [id("solar-system"), id("solar-system-2")],
            "both sets are listed in order"
        );
        let mut files: Vec<String> = fs::read_dir(scratch.path().join("sets"))
            .expect("the directory can be read")
            .map(|entry| {
                let name = entry.expect("an entry can be read").file_name();
                name.to_string_lossy().into_owned()
            })
            .collect();
        files.sort();
        assert_eq!(
            files,
            ["solar-system-2.toml", "solar-system.toml"],
            "no temporary file is left behind"
        );
    }

    #[test]
    fn an_unusable_title_creates_nothing() {
        let scratch = Scratch::new();
        let store = Store::open(scratch.path()).expect("store opens");
        let error = store
            .create_set("   ")
            .expect_err("a blank title is rejected");
        assert!(
            matches!(
                error,
                StoreError::InvalidSet {
                    source: DeckError::Title(TextProblem::Empty),
                    ..
                }
            ),
            "the title problem is reported: {error}"
        );
        assert!(
            store.list().expect("listing works").sets.is_empty(),
            "no set was created"
        );
    }

    #[test]
    fn listing_reports_files_it_cannot_use() {
        let scratch = Scratch::new();
        let store = Store::open(scratch.path()).expect("store opens");
        let sets = scratch.path().join("sets");
        fs::write(sets.join("good.toml"), "title = \"Good\"").expect("file written");
        fs::write(sets.join("Bad Name.toml"), "title = \"Bad\"").expect("file written");
        fs::write(sets.join("notes.txt"), "not a set").expect("file written");
        fs::create_dir_all(sets.join("folder.toml")).expect("directory created");
        let listing = store.list().expect("listing works");
        assert_eq!(
            listing.sets,
            [id("good")],
            "only usable set files are listed"
        );
        assert_eq!(
            listing.unusable,
            ["Bad Name.toml"],
            "unusable file names are reported"
        );
    }

    #[test]
    fn loading_reports_missing_and_broken_sets() {
        let scratch = Scratch::new();
        let store = Store::open(scratch.path()).expect("store opens");
        let missing = store.load_set(&id("missing"));
        assert!(
            matches!(&missing, Err(StoreError::UnknownSet(set)) if *set == id("missing")),
            "a missing set is reported as unknown"
        );
        fs::write(scratch.path().join("sets").join("broken.toml"), "title = ")
            .expect("file written");
        let error = store
            .load_set(&id("broken"))
            .expect_err("a broken file is rejected");
        assert!(
            matches!(error, StoreError::InvalidSet { .. }),
            "the file is reported as invalid"
        );
        assert!(
            error.to_string().contains("broken.toml"),
            "the message names the file: {error}"
        );
    }

    #[test]
    fn an_error_names_the_error_behind_it() {
        use std::error::Error as _;

        let scratch = Scratch::new();
        let store = Store::open(scratch.path()).expect("store opens");
        let sets = scratch.path().join("sets");
        fs::create_dir_all(sets.join("folder.toml")).expect("directory created");
        fs::write(sets.join("blank.toml"), "title = \" \"").expect("file written");
        let behind = |set: &str| {
            let error = store
                .load_set(&id(set))
                .expect_err("the set cannot be loaded");
            error.source().map(ToString::to_string)
        };

        assert_eq!(
            behind("folder").as_deref(),
            Some("not a regular file"),
            "the I/O error is behind an unreadable file"
        );
        assert_eq!(
            behind("blank").as_deref(),
            Some("the set title is empty"),
            "the problem of the set is behind an invalid file"
        );
        assert_eq!(behind("missing"), None, "nothing is behind an unknown set");
    }

    #[test]
    fn only_regular_files_are_read() {
        let scratch = Scratch::new();
        let store = Store::open(scratch.path()).expect("store opens");
        fs::create_dir_all(scratch.path().join("sets").join("folder.toml"))
            .expect("directory created");
        fs::create_dir_all(scratch.path().join("progress").join("folder.json"))
            .expect("directory created");

        let set = store
            .load_set(&id("folder"))
            .expect_err("a directory is not a set");
        let refused = matches!(
            &set,
            StoreError::Io { source, .. } if source.kind() == ErrorKind::InvalidInput
        );
        assert!(refused, "the set is refused before it is read: {set}");
        assert!(
            set.to_string().ends_with("folder.toml: not a regular file"),
            "the message says why: {set}"
        );
        let progress = store
            .load_progress(&id("folder"))
            .expect_err("a directory is not a progress file");
        assert!(
            progress
                .to_string()
                .ends_with("folder.json: not a regular file"),
            "the message says why: {progress}"
        );
    }

    #[test]
    fn a_temporary_file_never_takes_a_name_that_exists() {
        let scratch = Scratch::new();
        let stale = scratch.path().join("first.tmp");
        fs::write(&stale, "left by an earlier run").expect("file written");
        let folder = scratch.path().join("second.tmp");
        fs::create_dir_all(&folder).expect("directory created");
        let free = scratch.path().join("third.tmp");

        let (file, chosen) = create_exclusive([stale.clone(), folder.clone(), free.clone()])
            .expect("one name is free");
        drop(file);
        assert_eq!(chosen, free, "names that exist are passed over");
        assert_eq!(
            fs::read_to_string(&stale).expect("file read"),
            "left by an earlier run",
            "a file that was there is left as it was"
        );

        let none = create_exclusive([stale, folder, free]);
        assert_eq!(
            none.map(|(_, path)| path).map_err(|error| error.kind()),
            Err(ErrorKind::AlreadyExists),
            "with every name taken, nothing is created"
        );
    }

    #[cfg(unix)]
    #[test]
    fn nothing_is_written_through_a_symbolic_link() {
        use std::os::unix::fs as unix;

        let scratch = Scratch::new();
        let store = Store::open(scratch.path()).expect("store opens");
        let mut set = store.create_set("Demo").expect("the set is created");
        let sets = scratch.path().join("sets");
        let victim = scratch.path().join("victim.txt");
        fs::write(&victim, "untouched").expect("file written");
        let untouched = || fs::read_to_string(&victim).expect("file read");

        let planted = scratch.path().join("planted.tmp");
        unix::symlink(&victim, &planted).expect("link created");
        let dangling = scratch.path().join("dangling.tmp");
        unix::symlink(scratch.path().join("absent"), &dangling).expect("link created");
        let created = create_exclusive([planted, dangling]);
        assert_eq!(
            created.map(|(_, path)| path).map_err(|error| error.kind()),
            Err(ErrorKind::AlreadyExists),
            "a link is a name that exists, wherever it leads"
        );
        assert!(
            !scratch.path().join("absent").exists(),
            "nothing was created through the dangling link"
        );

        // Links beside the set file, and one in place of the set file itself.
        unix::symlink(&victim, sets.join("demo.toml.tmp")).expect("link created");
        fs::remove_file(sets.join("demo.toml")).expect("file removed");
        unix::symlink(&victim, sets.join("demo.toml")).expect("link created");
        set.set_title("Changed").expect("valid title");
        store.save_set(&set).expect("the set is saved");

        assert_eq!(
            untouched(),
            "untouched",
            "the file the links name is not written"
        );
        let saved = fs::symlink_metadata(sets.join("demo.toml")).expect("the set file exists");
        assert!(
            !saved.file_type().is_symlink(),
            "saving replaced the link with a file of the set's own"
        );
        assert_eq!(
            store.load_set(set.id()).expect("the set loads"),
            set,
            "the saved set is read back"
        );
    }

    #[test]
    fn progress_is_saved_loaded_and_reset() {
        let scratch = Scratch::new();
        let store = Store::open(scratch.path()).expect("store opens");
        let mut set = store.create_set("Demo").expect("the set is created");
        let topic = set.add_topic("Topic").expect("valid topic");
        for number in 0_u8..3 {
            let draft = CardDraft {
                question: format!("Question {number}"),
                answer: "right".to_owned(),
                wrong: vec!["wrong".to_owned()],
                explanation: None,
            };
            set.add_card(&topic, &draft).expect("valid card");
        }
        assert_eq!(
            store
                .load_progress(set.id())
                .expect("missing progress is fine"),
            Progress::new(),
            "a set without a progress file has not been studied"
        );

        let mut progress = Progress::new();
        let mut random = fastrand::Rng::with_seed(1);
        progress
            .prepare(&set, &Scope::All, &mut random)
            .expect("the scope exists");
        let view = progress.view(&set, &Scope::All);
        let View::Question { card, .. } = view else {
            panic!("expected a question, found {view:?}");
        };
        let card = card.id().clone();
        progress
            .answer(&set, &Scope::All, &card, Choice::DontKnow, &mut random)
            .expect("accepted");
        store
            .save_progress(set.id(), &progress)
            .expect("progress is saved");
        assert_eq!(
            store.load_progress(set.id()).expect("progress loads"),
            progress,
            "saved progress is read back"
        );

        store.reset_progress(set.id()).expect("progress is reset");
        store
            .reset_progress(set.id())
            .expect("resetting twice is fine");
        assert_eq!(
            store.load_progress(set.id()).expect("progress loads"),
            Progress::new(),
            "reset progress is empty"
        );
    }

    #[test]
    fn saving_recreates_a_directory_removed_while_running() {
        let scratch = Scratch::new();
        let store = Store::open(scratch.path()).expect("store opens");
        let set = store.create_set("Demo").expect("the set is created");
        fs::remove_dir_all(scratch.path().join("progress")).expect("directory removed");
        store
            .save_progress(set.id(), &Progress::new())
            .expect("saving recreates the directory");
        assert!(
            scratch.path().join("progress").join("demo.json").is_file(),
            "the progress file exists"
        );
    }

    #[test]
    fn a_broken_progress_file_is_reported() {
        let scratch = Scratch::new();
        let store = Store::open(scratch.path()).expect("store opens");
        fs::write(
            scratch.path().join("progress").join("demo.json"),
            "{ not json",
        )
        .expect("written");
        let error = store
            .load_progress(&id("demo"))
            .expect_err("broken progress is rejected");
        assert!(
            matches!(error, StoreError::InvalidProgress { .. }),
            "reported as invalid: {error}"
        );
        assert!(
            error.to_string().contains("demo.json"),
            "the message names the file: {error}"
        );
    }

    #[test]
    fn messages_about_broken_files_stay_short() {
        let scratch = Scratch::new();
        let store = Store::open(scratch.path()).expect("store opens");
        let long = "x".repeat(1_000_000);
        fs::write(
            scratch.path().join("progress").join("demo.json"),
            format!("{{\"cards\": {{\"card\": [\"{long}\"]}}}}"),
        )
        .expect("file written");
        fs::write(
            scratch.path().join("sets").join("demo.toml"),
            format!("title = {long}"),
        )
        .expect("file written");

        let progress = store
            .load_progress(&id("demo"))
            .expect_err("an unknown answer is rejected")
            .to_string();
        assert!(
            progress.chars().count() < 1000,
            "a message of {} characters",
            progress.chars().count()
        );
        assert!(
            progress.contains("… at line 1 column 10000"),
            "the message still says where the problem is: {progress}"
        );
        let set = store
            .load_set(&id("demo"))
            .expect_err("an unquoted title is rejected")
            .to_string();
        assert!(
            set.chars().count() < 3000,
            "a message of {} characters",
            set.chars().count()
        );
        assert!(
            set.contains("TOML parse error at line 1"),
            "the message still says where the problem is"
        );
    }

    #[test]
    fn deleting_a_set_removes_its_progress() {
        let scratch = Scratch::new();
        let store = Store::open(scratch.path()).expect("store opens");
        let set = store.create_set("Demo").expect("the set is created");
        store
            .save_progress(set.id(), &Progress::new())
            .expect("progress is saved");
        store.delete_set(set.id()).expect("the set is deleted");
        assert!(
            store.list().expect("listing works").sets.is_empty(),
            "the set is gone"
        );
        assert!(
            !scratch.path().join("progress").join("demo.json").exists(),
            "its progress is gone"
        );
        assert!(
            matches!(store.delete_set(set.id()), Err(StoreError::UnknownSet(_))),
            "deleting twice reports an unknown set"
        );
    }
}
