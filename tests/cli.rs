//! Runs the `studycards` program and checks what it prints and how it exits.
//!
//! The commands that end by themselves are run here: `--version`, `--help`,
//! and `check`. The unit tests of `studycards::cli` cover `serve`.

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    /// Runs the program with `arguments` and waits for it to end.
    fn studycards<I, S>(arguments: I) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Command::new(env!("CARGO_BIN_EXE_studycards"))
            .args(arguments)
            .output()
            .expect("the program can be run")
    }

    fn text(output: &[u8]) -> &str {
        str::from_utf8(output).expect("UTF-8 output")
    }

    /// Returns the path of a file in the directory of study sets that the
    /// repository holds.
    fn set_file(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join("sets")
            .join(name)
    }

    #[test]
    fn the_version_is_printed() {
        let output = studycards(["--version"]);
        assert!(output.status.success(), "the program succeeds");
        assert_eq!(
            text(&output.stdout),
            format!("studycards {}\n", env!("CARGO_PKG_VERSION")),
            "standard output"
        );
        assert_eq!(text(&output.stderr), "", "standard error");
    }

    #[test]
    fn help_shows_how_to_call_the_program() {
        let output = studycards(["--help"]);
        assert!(output.status.success(), "the program succeeds");
        assert!(
            text(&output.stdout).contains("studycards [serve] [--port PORT] [--data DIR]"),
            "the usage is printed:\n{}",
            text(&output.stdout)
        );
        assert_eq!(text(&output.stderr), "", "standard error");
    }

    #[test]
    fn a_mistake_is_explained_on_standard_error() {
        let output = studycards(["--bogus"]);
        assert!(!output.status.success(), "the program fails");
        assert_eq!(text(&output.stdout), "", "standard output");
        assert_eq!(
            text(&output.stderr),
            "studycards: unexpected argument \"--bogus\"\nTry 'studycards --help'.\n",
            "standard error"
        );
    }

    #[test]
    fn the_example_set_passes_the_check() {
        let example = set_file("example.toml");
        let output = studycards([OsStr::new("check"), example.as_os_str()]);
        assert!(
            output.status.success(),
            "the example set is valid: {}",
            text(&output.stdout)
        );
        assert!(
            text(&output.stdout).starts_with(&format!("ok: {}: \"", example.display())),
            "the report names the file and its title: {}",
            text(&output.stdout)
        );
        assert_eq!(text(&output.stderr), "", "standard error");
    }

    #[test]
    fn check_fails_when_a_file_is_not_a_study_set() {
        let missing = set_file("no-such-set.toml");
        let output = studycards([OsStr::new("check"), missing.as_os_str()]);
        assert!(!output.status.success(), "the program fails");
        assert!(
            text(&output.stdout).starts_with(&format!("invalid: {}: ", missing.display())),
            "the report names the file: {}",
            text(&output.stdout)
        );
        assert_eq!(
            text(&output.stderr),
            "studycards: 1 file is not a valid study set\n",
            "standard error"
        );
    }
}
