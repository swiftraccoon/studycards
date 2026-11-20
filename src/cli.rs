//! The command line of the `studycards` program.
//!
//! ```text
//! studycards [serve] [--port PORT] [--data DIR]
//! studycards check FILE...
//! studycards --help | --version
//! ```
//!
//! `serve`, the default, starts the web application on a loopback port and
//! runs until interrupted. `check` reads study set files and reports whether
//! each is valid, without starting anything.
//!
//! The report of `check` has one line for each file, beginning with `ok:` or
//! `invalid:`; a problem that takes several lines continues on indented
//! lines. A file can come from someone else, so the text that the report takes
//! from a file, or from its name, is printed with control characters as
//! escapes such as `\u{1b}`.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::deck::{SetId, StudySet};
use crate::http::Server;
use crate::store::Store;
use crate::web::App;

/// The port used when none is given.
pub const DEFAULT_PORT: u16 = 8717;
/// The data directory used when none is given, relative to the working
/// directory.
pub const DEFAULT_DATA_DIR: &str = "data";

/// The text printed for `--help`.
const USAGE: &str = "\
Study your own flash-card sets in the browser.

Usage:
  studycards [serve] [--port PORT] [--data DIR]
  studycards check FILE...
  studycards --help | --version

Commands:
  serve   Start the application (the default) and print its address.
  check   Report whether each study set file is valid.

Options:
  --port PORT   Port to listen on, 0 for any free port [default: 8717]
  --data DIR    Directory holding sets/ and progress/ [default: data]
";

/// What the program was asked to do.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    /// Start the web application.
    Serve {
        /// The port to listen on; 0 picks a free one.
        port: u16,
        /// The data directory.
        data: PathBuf,
    },
    /// Validate study set files.
    Check(Vec<PathBuf>),
    /// Print the usage text.
    Help,
    /// Print the version.
    Version,
}

/// Why the arguments could not be understood.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UsageError {
    /// An option that takes a value came last.
    MissingValue(&'static str),
    /// The value of `--port` is not a port number.
    InvalidPort(String),
    /// An argument is not a known command or option.
    Unexpected(String),
    /// `check` was given no files.
    NothingToCheck,
}

impl Display for UsageError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingValue(option) => write!(f, "{option} needs a value"),
            Self::InvalidPort(value) => write!(f, "\"{value}\" is not a port number (0 to 65535)"),
            Self::Unexpected(argument) => write!(f, "unexpected argument \"{argument}\""),
            Self::NothingToCheck => f.write_str("check needs at least one file"),
        }
    }
}

impl Error for UsageError {}

impl Command {
    /// Interprets the arguments that follow the program name.
    ///
    /// # Errors
    ///
    /// Returns a [`UsageError`] describing the first argument that cannot be
    /// understood.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::PathBuf;
    ///
    /// use studycards::cli::{Command, UsageError};
    ///
    /// let serve = ["--port", "0", "--data", "cards"].map(String::from);
    /// assert_eq!(
    ///     Command::parse(serve),
    ///     Ok(Command::Serve { port: 0, data: PathBuf::from("cards") })
    /// );
    ///
    /// let check = ["check", "planets.toml"].map(String::from);
    /// assert_eq!(
    ///     Command::parse(check),
    ///     Ok(Command::Check(vec![PathBuf::from("planets.toml")]))
    /// );
    ///
    /// let mistake = ["--port"].map(String::from);
    /// assert_eq!(
    ///     Command::parse(mistake),
    ///     Err(UsageError::MissingValue("--port"))
    /// );
    /// ```
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, UsageError> {
        let mut arguments = arguments.into_iter().peekable();
        match arguments.peek().map(String::as_str) {
            Some("check") => {
                let files: Vec<PathBuf> = arguments.skip(1).map(PathBuf::from).collect();
                return if files.is_empty() {
                    Err(UsageError::NothingToCheck)
                } else {
                    Ok(Self::Check(files))
                };
            }
            Some("serve") => drop(arguments.next()),
            _ => {}
        }
        let mut port = DEFAULT_PORT;
        let mut data = PathBuf::from(DEFAULT_DATA_DIR);
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--help" | "-h" => return Ok(Self::Help),
                "--version" | "-V" => return Ok(Self::Version),
                "--port" => {
                    let value = arguments.next().ok_or(UsageError::MissingValue("--port"))?;
                    port = value
                        .parse()
                        .map_err(|_not_a_port| UsageError::InvalidPort(value))?;
                }
                "--data" => {
                    data = arguments
                        .next()
                        .map(PathBuf::from)
                        .ok_or(UsageError::MissingValue("--data"))?;
                }
                _ => return Err(UsageError::Unexpected(argument)),
            }
        }
        Ok(Self::Serve { port, data })
    }
}

/// Runs the program with the arguments that follow its name and returns its
/// exit status: success, or failure after a message on standard error.
pub fn main(arguments: impl IntoIterator<Item = String>) -> ExitCode {
    let mut out = io::stdout().lock();
    let outcome = match Command::parse(arguments) {
        Ok(command) => run(command, &mut out),
        Err(error) => Err(format!("{error}\nTry 'studycards --help'.")),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            // Standard error is the last place a failure can be reported.
            drop(writeln!(io::stderr().lock(), "studycards: {message}"));
            ExitCode::FAILURE
        }
    }
}

/// Carries out a command, writing its normal output to `out`.
fn run(command: Command, out: &mut dyn Write) -> Result<(), String> {
    let written =
        |result: io::Result<()>| result.map_err(|error| format!("cannot write output: {error}"));
    match command {
        Command::Help => written(out.write_all(USAGE.as_bytes())),
        Command::Version => written(writeln!(out, "studycards {}", env!("CARGO_PKG_VERSION"))),
        Command::Check(files) => {
            let mut invalid = 0_usize;
            for file in &files {
                // File names, like file content, can come from someone else.
                let name = one_line(&file.display().to_string());
                match check(file) {
                    Ok(summary) => written(writeln!(out, "ok: {name}: {summary}"))?,
                    Err(problem) => {
                        invalid = invalid.saturating_add(1);
                        written(writeln!(
                            out,
                            "invalid: {name}: {}",
                            indented(problem.trim_end())
                        ))?;
                    }
                }
            }
            match invalid {
                0 => Ok(()),
                1 => Err("1 file is not a valid study set".to_owned()),
                _ => Err(format!("{invalid} files are not valid study sets")),
            }
        }
        Command::Serve { port, data } => {
            let store = Store::open(&data).map_err(|error| error.to_string())?;
            let server = Server::bind(port)
                .map_err(|error| format!("cannot listen on port {port}: {error}"))?;
            written(writeln!(
                out,
                "Study cards is running at http://127.0.0.1:{}/\n\
                 Sets and progress are kept in {}\n\
                 Press Ctrl+C to stop.",
                server.port(),
                data.display()
            ))?;
            written(out.flush())?;
            server
                .run(App::new(store))
                .map_err(|error| format!("the server stopped: {error}"))
        }
    }
}

/// Reads a study set file and describes it on one line, or says what is wrong
/// with it.
///
/// The problem is returned as the file gave it and may hold any character.
fn check(file: &Path) -> Result<String, String> {
    let id: SetId = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| "the file has no name".to_owned())?
        .parse()
        .map_err(|problem| {
            format!("the file name cannot be used as a set identifier: {problem}")
        })?;
    let kind = fs::metadata(file).map_err(|error| error.to_string())?;
    if !kind.is_file() {
        return Err("not a regular file".to_owned());
    }
    let text = fs::read_to_string(file).map_err(|error| error.to_string())?;
    let set = StudySet::from_toml(id, &text).map_err(|error| error.to_string())?;
    Ok(format!(
        "\"{}\", {}, {}",
        one_line(set.title()),
        counted(set.topics().len(), "topic"),
        counted(set.card_count(), "card")
    ))
}

/// Writes a number with the noun it counts, as in "1 card" and "2 cards".
fn counted(number: usize, noun: &str) -> String {
    let ending = if number == 1 { "" } else { "s" };
    format!("{number} {noun}{ending}")
}

/// Makes text from a file safe to print on a terminal, on one line.
///
/// A control character, which a terminal would obey, and a character that
/// reorders the text around it are shown as escapes such as `\u{1b}`.
fn one_line(text: &str) -> String {
    printable(text, "\\u{a}")
}

/// Makes text from a file safe to print on a terminal, keeping its lines.
///
/// The lines after the first are indented, so that none of them can pass for
/// a line of the report. Other characters are treated as by [`one_line`].
fn indented(text: &str) -> String {
    printable(text, "\n    ")
}

/// Copies `text`, writing `line_break` for a line feed and an escape for
/// every other character that must not reach a terminal.
fn printable(text: &str, line_break: &str) -> String {
    let mut shown = String::with_capacity(text.len());
    for character in text.chars() {
        if character == '\n' {
            shown.push_str(line_break);
        } else if character.is_control() || is_reordering(character) {
            shown.extend(character.escape_unicode());
        } else {
            shown.push(character);
        }
    }
    shown
}

/// Returns `true` for the characters that change the direction in which the
/// text around them is displayed.
const fn is_reordering(character: char) -> bool {
    matches!(
        character,
        '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{self, Write};
    use std::net::{Ipv4Addr, TcpStream};
    use std::path::PathBuf;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread;

    use super::{Command, DEFAULT_DATA_DIR, DEFAULT_PORT, UsageError, counted, run};
    use crate::scratch::Scratch;

    fn parse(arguments: &[&str]) -> Result<Command, UsageError> {
        Command::parse(arguments.iter().map(|&argument| argument.to_owned()))
    }

    #[test]
    fn serving_is_the_default() {
        let default = Command::Serve {
            port: DEFAULT_PORT,
            data: PathBuf::from(DEFAULT_DATA_DIR),
        };
        assert_eq!(parse(&[]), Ok(default.clone()), "no arguments");
        assert_eq!(parse(&["serve"]), Ok(default), "the command alone");
        assert_eq!(
            parse(&["--port", "0", "--data", "/tmp/cards"]),
            Ok(Command::Serve {
                port: 0,
                data: PathBuf::from("/tmp/cards")
            }),
            "options without the command"
        );
        assert_eq!(
            parse(&["serve", "--data", "x", "--port", "9000"]),
            Ok(Command::Serve {
                port: 9000,
                data: PathBuf::from("x")
            }),
            "options in any order"
        );
    }

    #[test]
    fn help_version_and_check() {
        assert_eq!(parse(&["--help"]), Ok(Command::Help), "long help");
        assert_eq!(
            parse(&["serve", "-h"]),
            Ok(Command::Help),
            "short help after the command"
        );
        assert_eq!(parse(&["--version"]), Ok(Command::Version), "long version");
        assert_eq!(parse(&["-V"]), Ok(Command::Version), "short version");
        assert_eq!(
            parse(&["check", "a.toml", "--port"]),
            Ok(Command::Check(vec![
                PathBuf::from("a.toml"),
                PathBuf::from("--port")
            ])),
            "everything after check is a file"
        );
    }

    #[test]
    fn mistakes_are_explained() {
        let cases = [
            (
                vec!["--port"],
                UsageError::MissingValue("--port"),
                "--port needs a value",
            ),
            (
                vec!["--data"],
                UsageError::MissingValue("--data"),
                "--data needs a value",
            ),
            (
                vec!["--port", "70000"],
                UsageError::InvalidPort("70000".to_owned()),
                "\"70000\" is not a port number (0 to 65535)",
            ),
            (
                vec!["--port", "http"],
                UsageError::InvalidPort("http".to_owned()),
                "\"http\" is not a port number (0 to 65535)",
            ),
            (
                vec!["start"],
                UsageError::Unexpected("start".to_owned()),
                "unexpected argument \"start\"",
            ),
            (
                vec!["check"],
                UsageError::NothingToCheck,
                "check needs at least one file",
            ),
        ];
        for (arguments, error, message) in cases {
            assert_eq!(
                parse(&arguments),
                Err(error.clone()),
                "arguments {arguments:?}"
            );
            assert_eq!(error.to_string(), message, "message for {arguments:?}");
        }
    }

    #[test]
    fn help_and_version_are_printed() {
        let mut out = Vec::new();
        run(Command::Help, &mut out).expect("help succeeds");
        let help = String::from_utf8(out).expect("UTF-8 output");
        assert!(
            help.contains("studycards check FILE..."),
            "usage is shown:\n{help}"
        );
        assert!(
            help.contains(&format!("[default: {DEFAULT_PORT}]")),
            "the default port is documented"
        );
        assert!(
            help.contains(&format!("[default: {DEFAULT_DATA_DIR}]")),
            "the default directory is documented"
        );

        let mut version = Vec::new();
        run(Command::Version, &mut version).expect("version succeeds");
        assert_eq!(
            String::from_utf8(version).expect("UTF-8 output"),
            format!("studycards {}\n", env!("CARGO_PKG_VERSION")),
            "the version line"
        );
    }

    #[test]
    fn check_reports_each_file() {
        let scratch = Scratch::new();
        let directory = scratch.path();
        let good = directory.join("good.toml");
        let bad = directory.join("bad.toml");
        let misnamed = directory.join("Not An Id.toml");
        fs::write(
            &good,
            "title = \"Good\"\n[[topics]]\nname = \"T\"\n[[topics.cards]]\n\
             question = \"q\"\nanswer = \"a\"\nwrong = [\"b\"]\n",
        )
        .expect("file written");
        fs::write(&bad, "title = \"Bad\"\n[[topics]]\nname = \"\"\n").expect("file written");
        fs::write(&misnamed, "title = \"Fine\"\n").expect("file written");

        let mut out = Vec::new();
        run(Command::Check(vec![good.clone()]), &mut out).expect("a valid file passes");
        assert_eq!(
            String::from_utf8(out).expect("UTF-8 output"),
            format!("ok: {}: \"Good\", 1 topic, 1 card\n", good.display()),
            "report for a valid file"
        );

        let mut mixed = Vec::new();
        let missing = directory.join("missing.toml");
        let outcome = run(
            Command::Check(vec![good, bad.clone(), misnamed.clone(), missing.clone()]),
            &mut mixed,
        );
        let report = String::from_utf8(mixed).expect("UTF-8 output");
        assert_eq!(
            outcome,
            Err("3 files are not valid study sets".to_owned()),
            "the failure counts files"
        );
        assert!(
            report.contains(&format!(
                "invalid: {}: a topic name is empty",
                bad.display()
            )),
            "bad content:\n{report}"
        );
        assert!(
            report.contains(&format!(
                "invalid: {}: the file name cannot be used as a set identifier",
                misnamed.display()
            )),
            "bad name:\n{report}"
        );
        assert!(
            report.contains(&format!("invalid: {}: ", missing.display())),
            "missing file:\n{report}"
        );
    }

    #[test]
    fn counts_are_written_with_a_noun_in_the_right_number() {
        assert_eq!(counted(0, "card"), "0 cards", "none");
        assert_eq!(counted(1, "card"), "1 card", "one");
        assert_eq!(counted(2, "topic"), "2 topics", "several");
    }

    #[test]
    fn check_reads_regular_files_only() {
        let scratch = Scratch::new();
        let folder = scratch.path().join("folder.toml");
        fs::create_dir_all(&folder).expect("directory created");
        let mut out = Vec::new();
        let outcome = run(Command::Check(vec![folder.clone()]), &mut out);
        assert_eq!(
            outcome,
            Err("1 file is not a valid study set".to_owned()),
            "a directory is not a valid set"
        );
        assert_eq!(
            String::from_utf8(out).expect("UTF-8 output"),
            format!("invalid: {}: not a regular file\n", folder.display()),
            "the report says why"
        );
    }

    #[test]
    fn text_from_a_file_cannot_control_the_terminal_or_forge_a_report_line() {
        let scratch = Scratch::new();
        let titled = scratch.path().join("titled.toml");
        fs::write(
            &titled,
            "title = \"a\\u001b]0;owned\\u0007\\nok: /other.toml: fine\\u202e\"\n",
        )
        .expect("file written");
        let broken = scratch.path().join("broken.toml");
        fs::write(&broken, "title = \u{1b}[31m oops\r\n").expect("file written");
        let card = scratch.path().join("card.toml");
        fs::write(
            &card,
            "title = \"x\"\n[[topics]]\nname = \"t\"\n[[topics.cards]]\n\
             question = \"q\\nok: /other.toml: forged\\u009b31m\"\nanswer = \"a\"\nwrong = []\n",
        )
        .expect("file written");

        let mut out = Vec::new();
        let outcome = run(Command::Check(vec![titled.clone(), broken, card]), &mut out);
        let report = String::from_utf8(out).expect("UTF-8 output");
        assert_eq!(
            outcome,
            Err("2 files are not valid study sets".to_owned()),
            "the files are judged as before"
        );
        assert!(
            report
                .chars()
                .all(|character| character == '\n' || !character.is_control()),
            "no control character is printed:\n{}",
            report.escape_debug()
        );
        assert!(
            !report.contains('\u{202e}'),
            "no character that reorders text is printed"
        );
        assert!(
            report.starts_with(&format!(
                "ok: {}: \"a\\u{{1b}}]0;owned\\u{{7}}\\u{{a}}ok: /other.toml: fine\\u{{202e}}\", \
                 0 topics, 0 cards\n",
                titled.display()
            )),
            "the title is shown on one line with its escapes:\n{report}"
        );
        let lines: Vec<&str> = report.lines().collect();
        assert_eq!(
            lines.iter().filter(|line| line.starts_with("ok: ")).count(),
            1,
            "text from a file starts no report line:\n{report}"
        );
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.starts_with("invalid: "))
                .count(),
            2,
            "each invalid file has one report line:\n{report}"
        );
        assert!(
            lines.iter().all(|line| {
                line.starts_with("ok: ")
                    || line.starts_with("invalid: ")
                    || line.starts_with("    ")
            }),
            "every other line is indented:\n{report}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_file_name_cannot_control_the_terminal() {
        let scratch = Scratch::new();
        let named = scratch.path().join("a\u{1b}[2J\nok: forged.toml");
        fs::write(&named, "title = \"x\"\n").expect("file written");
        let mut out = Vec::new();
        let outcome = run(Command::Check(vec![named]), &mut out);
        let report = String::from_utf8(out).expect("UTF-8 output");
        assert!(outcome.is_err(), "the name is not a set identifier");
        assert!(
            report.starts_with("invalid: ") && report.ends_with('\n'),
            "one report line:\n{report}"
        );
        assert_eq!(report.matches('\n').count(), 1, "the name adds no line");
        assert!(
            report.contains("a\\u{1b}[2J\\u{a}ok: forged.toml"),
            "the name is shown with its escapes:\n{report}"
        );
        assert!(
            !report.contains('\u{1b}'),
            "no control character is printed"
        );
    }

    /// Passes on to a channel what is written to it.
    struct Relay(Sender<Vec<u8>>);

    impl Write for Relay {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            // Nobody listens once the test that made the channel has ended.
            drop(self.0.send(buf.to_vec()));
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn serving_announces_the_address_and_answers_requests() {
        use std::io::Read as _;

        let scratch = Scratch::new();
        let data = scratch.path().join("data");
        let command = Command::Serve {
            port: 0,
            data: data.clone(),
        };
        let (output, announced): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel();
        // The server thread runs until the test process exits.
        drop(thread::spawn(move || run(command, &mut Relay(output))));

        let mut announcement = String::new();
        while !announcement.ends_with("Press Ctrl+C to stop.\n") {
            let written = announced.recv().expect("the server announces itself");
            announcement.push_str(&String::from_utf8(written).expect("UTF-8 output"));
        }
        let (address, rest) = announcement
            .split_once('\n')
            .expect("the announcement has several lines");
        let port: u16 = address
            .strip_prefix("Study cards is running at http://127.0.0.1:")
            .and_then(|port| port.strip_suffix('/'))
            .and_then(|port| port.parse().ok())
            .unwrap_or_else(|| panic!("the first line gives the address: {address}"));
        assert_eq!(
            rest,
            format!(
                "Sets and progress are kept in {}\nPress Ctrl+C to stop.\n",
                data.display()
            ),
            "the announcement names the data directory and says how to stop"
        );

        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connects");
        write!(stream, "GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").expect("request sent");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("response read");
        assert!(
            response.starts_with("HTTP/1.1 200 OK\r\n"),
            "the announced address answers: {response}"
        );
        assert!(
            response.contains("No study sets yet"),
            "the home page of an empty data directory is served: {response}"
        );
        assert!(data.join("sets").is_dir(), "the data directory was created");
    }

    #[test]
    fn serving_fails_cleanly_when_the_data_directory_cannot_be_made() {
        let scratch = Scratch::new();
        let blocker = scratch.path().join("blocker");
        fs::write(&blocker, "a file, not a directory").expect("file written");
        let mut out = Vec::new();
        let outcome = run(
            Command::Serve {
                port: 0,
                data: blocker.join("data"),
            },
            &mut out,
        );
        assert!(
            outcome.is_err(),
            "a data directory under a file cannot be created"
        );
        assert!(out.is_empty(), "nothing is announced when starting fails");
    }
}
