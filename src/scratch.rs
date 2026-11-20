//! Scratch directories for tests.

use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU32, Ordering};

/// A directory that one test has to itself, removed on drop.
#[derive(Debug)]
#[expect(
    clippy::redundant_pub_crate,
    reason = "the tests of other modules use the type, and `unreachable_pub` forbids `pub`"
)]
pub(crate) struct Scratch(PathBuf);

impl Scratch {
    /// Creates a directory in the system's temporary directory.
    ///
    /// The temporary directory can be shared with other users, so the
    /// directory is one that did not exist: a directory or a symbolic link
    /// that someone else put at the same name is never used.
    pub(crate) fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        loop {
            let name = format!(
                "studycards-test-{}-{}",
                process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let path = env::temp_dir().join(name);
            #[expect(
                clippy::create_dir,
                reason = "`create_dir_all` would accept a directory that already exists"
            )]
            let created = fs::create_dir(&path);
            match created {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => panic!("no scratch directory at {}: {error}", path.display()),
            }
        }
    }

    /// Returns the directory.
    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Best effort: a leftover directory in the temporary directory is
        // harmless.
        drop(fs::remove_dir_all(&self.0));
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::Scratch;

    #[test]
    fn each_scratch_directory_is_new_and_removed_afterwards() {
        let first = Scratch::new();
        let second = Scratch::new();
        assert_ne!(first.path(), second.path(), "two directories differ");
        assert!(first.path().is_dir(), "the directory exists");
        let entries = fs::read_dir(first.path()).expect("the directory can be read");
        assert_eq!(entries.count(), 0, "the directory starts empty");

        let path = first.path().to_owned();
        fs::write(path.join("file"), "content").expect("file written");
        drop(first);
        assert!(!path.exists(), "the directory is removed with its content");
    }
}
