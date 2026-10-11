//! Moving a whole library to another folder or disk.
//!
//! Everything inside a library is stored with paths relative to its folder,
//! so moving the folder keeps every book, note and link working. On the
//! same disk the folder is renamed (instant); to another disk it is copied
//! file by file with progress, checked, and the old folder goes to the
//! Trash. The library must be closed while it moves.

use crate::{Error, Progress, Result};
use libreri_core::LibraryLayout;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// What moving a library to a folder would do, for the confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovePlan {
    /// The library's new folder (a folder of the same name in the place
    /// chosen).
    pub target: PathBuf,
    /// The place chosen is on the same disk, so the move is a rename.
    pub same_disk: bool,
    pub files: u64,
    pub bytes: u64,
}

/// How a move ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoveReport {
    /// The old folder could not be moved to the Trash (after a copy); it is
    /// still there and should be deleted by hand.
    pub old_left_behind: Option<String>,
}

/// Checks that the library in `root` can move into `parent` and says how.
pub fn plan_move(root: &Path, parent: &Path) -> Result<MovePlan> {
    let layout = LibraryLayout::new(root);
    if !layout.info_path().is_file() {
        return Err(Error::NotALibrary(root.to_path_buf()));
    }
    if !parent.is_dir() {
        return Err(Error::InvalidInput(format!(
            "the folder {} does not exist",
            parent.display()
        )));
    }
    let root_c = fs::canonicalize(root)?;
    let parent_c = fs::canonicalize(parent)?;
    if parent_c.starts_with(&root_c) {
        return Err(Error::InvalidInput(
            "a library cannot move into its own folder".into(),
        ));
    }
    let name = root_c
        .file_name()
        .ok_or_else(|| Error::InvalidInput("the library folder has no name".into()))?;
    let target = parent_c.join(name);
    if root_c.parent() == Some(parent_c.as_path()) {
        return Err(Error::InvalidInput("the library is already there".into()));
    }
    if target.exists() {
        return Err(Error::NameTaken(name.to_string_lossy().into_owned()));
    }
    let (files, bytes) = measure(&root_c)?;
    Ok(MovePlan {
        target,
        same_disk: same_disk(&root_c, &parent_c),
        files,
        bytes,
    })
}

/// Moves the (closed) library in `root` to `plan.target`. A rename when
/// possible; otherwise a checked copy, after which the old folder goes to
/// the Trash. A cancelled or failed copy removes what it wrote and leaves
/// the library where it was.
pub fn move_closed(root: &Path, plan: &MovePlan, progress: &dyn Progress) -> Result<MoveReport> {
    let root = fs::canonicalize(root)?;
    if plan.target.exists() {
        let name = plan.target.file_name().unwrap_or_default();
        return Err(Error::NameTaken(name.to_string_lossy().into_owned()));
    }
    if plan.same_disk {
        match fs::rename(&root, &plan.target) {
            Ok(()) => {
                progress.report(plan.bytes, plan.bytes, "");
                return Ok(MoveReport::default());
            }
            // Same device by its id, but the system still says no (some
            // network and synced folders): copy instead.
            Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {}
            Err(e) => return Err(e.into()),
        }
    }
    if let Err(e) = copy_tree(&root, &plan.target, plan.bytes, progress) {
        let _ = fs::remove_dir_all(&plan.target);
        return Err(e);
    }
    let (files, bytes) = measure(&plan.target)?;
    let (want_files, want_bytes) = measure(&root)?;
    if files != want_files || bytes != want_bytes {
        let _ = fs::remove_dir_all(&plan.target);
        return Err(Error::InvalidInput(
            "the copy did not match the library (a file may have changed while it was copied); nothing was moved".into(),
        ));
    }
    let old_left_behind = trash::delete(&root).err().map(|e| e.to_string());
    Ok(MoveReport { old_left_behind })
}

/// Files and bytes in a folder (the library lock is not counted).
fn measure(root: &Path) -> Result<(u64, u64)> {
    let lock = LibraryLayout::new(root).lock_path();
    let mut files = 0;
    let mut bytes = 0;
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|e| Error::Io(e.into()))?;
        if entry.file_type().is_file() && entry.path() != lock {
            files += 1;
            bytes += entry.metadata().map_err(|e| Error::Io(e.into()))?.len();
        }
    }
    Ok((files, bytes))
}

fn copy_tree(from: &Path, to: &Path, total: u64, progress: &dyn Progress) -> Result<()> {
    let lock = LibraryLayout::new(from).lock_path();
    let mut done = 0u64;
    fs::create_dir_all(to)?;
    for entry in WalkDir::new(from).follow_links(false).min_depth(1) {
        if progress.cancelled() {
            return Err(Error::Cancelled);
        }
        let entry = entry.map_err(|e| Error::Io(e.into()))?;
        let rel = entry
            .path()
            .strip_prefix(from)
            .expect("walkdir stays inside");
        let dest = to.join(rel);
        let kind = entry.file_type();
        if kind.is_dir() {
            fs::create_dir_all(&dest)?;
        } else if kind.is_file() {
            if entry.path() == lock {
                continue;
            }
            fs::copy(entry.path(), &dest)?;
            // Keep the time the file was changed, so the next scan does not
            // read every book again.
            if let Some(modified) = entry.metadata().ok().and_then(|m| m.modified().ok()) {
                if let Ok(f) = fs::File::options().write(true).open(&dest) {
                    let _ = f.set_modified(modified);
                }
            }
            done += entry.metadata().map(|m| m.len()).unwrap_or(0);
            progress.report(done, total, &rel.to_string_lossy());
        } else if kind.is_symlink() {
            copy_link(entry.path(), &dest)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_link(from: &Path, to: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(fs::read_link(from)?, to)
}

#[cfg(not(unix))]
fn copy_link(from: &Path, to: &Path) -> io::Result<()> {
    // Links are rare in a library; copy what they point to.
    if from.is_file() {
        fs::copy(from, to).map(|_| ())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn same_disk(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev(),
        _ => false,
    }
}

#[cfg(windows)]
fn same_disk(a: &Path, b: &Path) -> bool {
    use std::path::Component;
    let prefix = |p: &Path| match p.components().next() {
        Some(Component::Prefix(x)) => Some(x.as_os_str().to_ascii_lowercase()),
        _ => None,
    };
    prefix(a).is_some() && prefix(a) == prefix(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Library, NoProgress};

    fn library(dir: &Path) -> PathBuf {
        let root = dir.join("My Library");
        let lib = Library::create(&root, Some("Mine"), "test").unwrap();
        fs::write(root.join("Books/a.txt"), "hello").unwrap();
        fs::create_dir_all(root.join("Notes/Ada")).unwrap();
        fs::write(root.join("Notes/Ada/n.md"), "# note").unwrap();
        lib.shutdown().unwrap();
        root
    }

    #[test]
    fn plans_a_move_and_refuses_bad_places() {
        let dir = tempfile::tempdir().unwrap();
        let root = library(dir.path());
        let dest = dir.path().join("Elsewhere");
        fs::create_dir_all(&dest).unwrap();
        let plan = plan_move(&root, &dest).unwrap();
        assert_eq!(
            plan.target,
            fs::canonicalize(&dest).unwrap().join("My Library")
        );
        assert!(plan.same_disk);
        assert!(plan.files >= 2);
        // Into itself, where it already is, onto a taken name, nowhere.
        assert!(plan_move(&root, &root.join("Books")).is_err());
        assert!(plan_move(&root, dir.path()).is_err());
        fs::create_dir_all(dest.join("My Library")).unwrap();
        assert!(matches!(plan_move(&root, &dest), Err(Error::NameTaken(_))));
        assert!(plan_move(&root, &dir.path().join("missing")).is_err());
    }

    #[test]
    fn renames_on_the_same_disk_and_the_library_opens_there() {
        let dir = tempfile::tempdir().unwrap();
        let root = library(dir.path());
        let dest = dir.path().join("Elsewhere");
        fs::create_dir_all(&dest).unwrap();
        let plan = plan_move(&root, &dest).unwrap();
        let report = move_closed(&root, &plan, &NoProgress).unwrap();
        assert_eq!(report, MoveReport::default());
        assert!(!root.exists());
        assert_eq!(
            fs::read_to_string(plan.target.join("Notes/Ada/n.md")).unwrap(),
            "# note"
        );
        let lib = Library::open(&plan.target, Default::default()).unwrap();
        assert_eq!(lib.info().name, "Mine");
        lib.shutdown().unwrap();
    }

    #[test]
    fn copies_with_times_kept_and_cleans_up_when_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let root = library(dir.path());
        let target = dir.path().join("Copy");
        let (_, bytes) = measure(&root).unwrap();
        copy_tree(&root, &target, bytes, &NoProgress).unwrap();
        assert_eq!(measure(&target).unwrap(), measure(&root).unwrap());
        let t = |p: &Path| fs::metadata(p).unwrap().modified().unwrap();
        assert_eq!(t(&target.join("Books/a.txt")), t(&root.join("Books/a.txt")));

        struct Stop;
        impl Progress for Stop {
            fn report(&self, _: u64, _: u64, _: &str) {}
            fn cancelled(&self) -> bool {
                true
            }
        }
        let plan = MovePlan {
            target: dir.path().join("Cancelled"),
            same_disk: false,
            files: 0,
            bytes,
        };
        assert!(matches!(
            move_closed(&root, &plan, &Stop),
            Err(Error::Cancelled)
        ));
        assert!(!plan.target.exists());
        assert!(root.join("Books/a.txt").exists());
    }
}
