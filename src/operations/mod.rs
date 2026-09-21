pub mod archive;
pub mod batch_rename;
pub mod copy;
pub mod create;
pub mod jobs;
pub mod link;
pub mod move_op;
pub mod privileged;
pub mod rename;
pub mod trash;

use crate::error::FileManagerError;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
pub enum PendingOp {
    Copy,
    Move,
}

/// Check that `name` can be used as the name of a single file or folder and
/// hand it back trimmed. Rejects the empty string, `.` / `..`, and anything
/// containing a path separator (which would silently create or move things
/// somewhere other than where the user is looking) or a NUL byte.
///
/// Every place that turns user-typed text into a file name -- New Folder,
/// New File, Rename, Batch Rename -- goes through this.
pub fn validate_name(name: &str) -> Result<&str, FileManagerError> {
    let name = name.trim();

    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(FileManagerError::InvalidName);
    }

    Ok(name)
}

/// Copy or move `sources` into `destination_dir` synchronously, picking a
/// free "name (1)" style name for anything that would collide instead of
/// overwriting it. Returns how many items were actually transferred.
///
/// This is the lightweight sibling of the job engine in `jobs.rs`: no
/// progress dialog, no conflict prompt -- which is what makes it the right
/// tool for "Duplicate" (the job engine deliberately skips same-folder
/// pastes) and for the quick copy/move between the two split panes. It does
/// block, so callers run it on a worker thread (see `run_quick_transfer` in
/// `main.rs`).
pub fn paste_pending(
    destination_dir: &Path,
    operation: PendingOp,
    sources: &[PathBuf],
) -> Result<usize, FileManagerError> {
    if !destination_dir.is_dir() {
        return Err(FileManagerError::NotADirectory);
    }

    // A move removes the original, so it obeys the same protected-path
    // rules as delete and rename. (Copying a system folder is harmless.)
    if matches!(operation, PendingOp::Move) {
        crate::filesystem::protection::ensure_modifiable(sources)?;
    }

    let mut pasted = 0;

    for source in sources {
        let Some(file_name) = source.file_name() else {
            continue;
        };

        // A folder can't go inside itself (or one of its own subfolders):
        // for a copy that would recurse until the disk fills up.
        if destination_dir.starts_with(source) {
            return Err(FileManagerError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "\"{}\" can't be placed inside itself",
                    file_name.to_string_lossy()
                ),
            )));
        }

        // Moving something into the folder it's already in changes nothing;
        // without this it would be "renamed" to "name (1)".
        if matches!(operation, PendingOp::Move) && source.parent() == Some(destination_dir) {
            continue;
        }

        let destination = unique_destination(&destination_dir.join(file_name));

        match operation {
            PendingOp::Copy => copy::copy_path(source, &destination)?,
            PendingOp::Move => move_op::move_path(source, &destination)?,
        }

        pasted += 1;
    }

    Ok(pasted)
}

/// `true` if *something* is already at `path` -- including a dangling
/// symlink, which `Path::exists` (it follows links) would call "free".
pub fn occupied(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Extensions made of two parts, which must stay together: an archive that
/// collides becomes "backup (1).tar.gz", not "backup.tar (1).gz".
const COMPOUND_EXTENSIONS: &[&str] = &[
    ".tar.gz",
    ".tar.bz2",
    ".tar.xz",
    ".tar.zst",
    ".tar.lz",
    ".tar.lzma",
];

/// Split "photo.jpg" into ("photo", Some("jpg")). A dotfile like ".bashrc"
/// has no extension, and "x.tar.gz" splits as ("x", Some("tar.gz")).
fn split_name(file_name: &str) -> (String, Option<String>) {
    let bytes = file_name.as_bytes();

    for suffix in COMPOUND_EXTENSIONS {
        if bytes.len() > suffix.len() {
            let cut = bytes.len() - suffix.len();

            // The tail is pure ASCII, so `cut` (where the '.' is) is always
            // on a character boundary.
            if bytes[cut..].eq_ignore_ascii_case(suffix.as_bytes()) {
                return (
                    file_name[..cut].to_string(),
                    Some(file_name[cut + 1..].to_string()),
                );
            }
        }
    }

    if file_name.starts_with('.') && file_name.matches('.').count() == 1 {
        return (file_name.to_string(), None);
    }

    match file_name.rsplit_once('.') {
        Some((stem, extension)) => (stem.to_string(), Some(extension.to_string())),
        None => (file_name.to_string(), None),
    }
}

pub fn unique_destination(destination: &Path) -> PathBuf {
    if !occupied(destination) {
        return destination.to_path_buf();
    }

    let parent = destination.parent().unwrap_or_else(|| Path::new("."));

    let file_name = destination
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let (stem, extension) = split_name(&file_name);

    let mut counter = 1;

    loop {
        let candidate = match &extension {
            Some(extension) => parent.join(format!("{stem} ({counter}).{extension}")),
            None => parent.join(format!("{stem} ({counter})")),
        };

        if !occupied(&candidate) {
            return candidate;
        }

        counter += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::scratch_dir;

    #[test]
    fn validate_name_accepts_ordinary_names_and_trims() {
        assert_eq!(validate_name("notes.txt").unwrap(), "notes.txt");
        assert_eq!(validate_name("  padded name  ").unwrap(), "padded name");
        assert_eq!(validate_name(".hidden").unwrap(), ".hidden");
    }

    #[test]
    fn validate_name_rejects_names_that_are_not_a_single_component() {
        for bad in ["", "   ", ".", "..", "a/b", "/abs", "nul\0byte"] {
            assert!(validate_name(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn unique_destination_counts_up_and_keeps_extensions() {
        let dir = scratch_dir("unique-dest");
        let file = dir.join("report.txt");

        // Free name: unchanged.
        assert_eq!(unique_destination(&file), file);

        fs::write(&file, "x").unwrap();
        assert_eq!(unique_destination(&file), dir.join("report (1).txt"));

        fs::write(dir.join("report (1).txt"), "x").unwrap();
        assert_eq!(unique_destination(&file), dir.join("report (2).txt"));

        // No extension, and a dotfile (whose "extension" is its whole name).
        let plain = dir.join("README");
        fs::write(&plain, "x").unwrap();
        assert_eq!(unique_destination(&plain), dir.join("README (1)"));

        let dotfile = dir.join(".bashrc");
        fs::write(&dotfile, "x").unwrap();
        assert_eq!(unique_destination(&dotfile), dir.join(".bashrc (1)"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn archives_keep_their_double_extension_when_renamed_to_avoid_a_clash() {
        let dir = scratch_dir("unique-compound");
        let archive = dir.join("backup.tar.gz");
        fs::write(&archive, "x").unwrap();

        assert_eq!(unique_destination(&archive), dir.join("backup (1).tar.gz"));

        assert_eq!(
            split_name("photos.TAR.GZ"),
            ("photos".to_string(), Some("TAR.GZ".to_string()))
        );
        assert_eq!(
            split_name("notes.txt"),
            ("notes".to_string(), Some("txt".to_string()))
        );
        assert_eq!(split_name(".bashrc"), (".bashrc".to_string(), None));
        assert_eq!(split_name("Makefile"), ("Makefile".to_string(), None));
        // Non-ASCII names are split on a character boundary, not mid-letter.
        assert_eq!(
            split_name("résumé.tar.gz"),
            ("résumé".to_string(), Some("tar.gz".to_string()))
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn move_never_overwrites_and_ignores_same_folder_moves() {
        let root = scratch_dir("paste-move");
        let (src, dst) = (root.join("src"), root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();

        fs::write(src.join("a.txt"), "new").unwrap();
        fs::write(dst.join("a.txt"), "old").unwrap();

        let moved = paste_pending(&dst, PendingOp::Move, &[src.join("a.txt")]).unwrap();

        assert_eq!(moved, 1);
        assert!(!src.join("a.txt").exists());
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "old");
        assert_eq!(fs::read_to_string(dst.join("a (1).txt")).unwrap(), "new");

        // Moving something into the folder it's already in is a no-op --
        // not a rename to "a (2).txt".
        let again = paste_pending(&dst, PendingOp::Move, &[dst.join("a.txt")]).unwrap();

        assert_eq!(again, 0);
        assert!(dst.join("a.txt").exists());
        assert!(!dst.join("a (2).txt").exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn paste_refuses_a_missing_destination_and_a_folder_inside_itself() {
        let root = scratch_dir("paste-refuse");

        assert!(matches!(
            paste_pending(&root.join("missing"), PendingOp::Move, &[]),
            Err(FileManagerError::NotADirectory)
        ));

        let folder = root.join("f");
        fs::create_dir_all(folder.join("inner")).unwrap();

        assert!(paste_pending(&folder.join("inner"), PendingOp::Copy, &[folder.clone()]).is_err());
        assert!(paste_pending(&folder.join("inner"), PendingOp::Move, &[folder.clone()]).is_err());

        let _ = fs::remove_dir_all(&root);
    }
}
