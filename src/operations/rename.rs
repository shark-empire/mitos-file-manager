use crate::error::FileManagerError;
use crate::operations::validate_name;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub fn rename_path(source: &Path, new_name: &str) -> Result<PathBuf, FileManagerError> {
    let new_name = validate_name(new_name)?;
    let destination = source.with_file_name(new_name);

    // Renaming something to the name it already has is a no-op, not a
    // conflict.
    if destination == source {
        return Ok(destination);
    }

    // `fs::rename` silently replaces whatever is at the destination, so
    // check first. (`symlink_metadata`, so a dangling symlink at the target
    // counts as "taken" too.) The same-inode check keeps case-only renames
    // ("photo.jpg" -> "Photo.jpg") working on case-insensitive filesystems
    // like FAT/exFAT USB sticks, where the "existing" target is the source
    // itself.
    if let Ok(existing) = fs::symlink_metadata(&destination) {
        let same_file = fs::symlink_metadata(source)
            .map(|original| original.dev() == existing.dev() && original.ino() == existing.ino())
            .unwrap_or(false);

        if !same_file {
            return Err(FileManagerError::Io(io::Error::from(
                io::ErrorKind::AlreadyExists,
            )));
        }
    }

    fs::rename(source, &destination)?;

    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::scratch_dir;

    #[test]
    fn rename_refuses_to_replace_an_existing_file() {
        let dir = scratch_dir("rename-clash");
        fs::write(dir.join("a.txt"), "A").unwrap();
        fs::write(dir.join("b.txt"), "B").unwrap();

        assert!(rename_path(&dir.join("a.txt"), "b.txt").is_err());

        assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "A");
        assert_eq!(fs::read_to_string(dir.join("b.txt")).unwrap(), "B");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_moves_the_file_and_treats_the_same_name_as_a_no_op() {
        let dir = scratch_dir("rename-ok");
        fs::write(dir.join("a.txt"), "A").unwrap();

        let renamed = rename_path(&dir.join("a.txt"), "c.txt").unwrap();

        assert_eq!(renamed, dir.join("c.txt"));
        assert!(renamed.exists());
        assert!(!dir.join("a.txt").exists());

        assert_eq!(rename_path(&renamed, "c.txt").unwrap(), renamed);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_rejects_names_with_path_separators() {
        let dir = scratch_dir("rename-invalid");
        fs::write(dir.join("a.txt"), "A").unwrap();

        assert!(matches!(
            rename_path(&dir.join("a.txt"), "x/y"),
            Err(FileManagerError::InvalidName)
        ));
        assert!(matches!(
            rename_path(&dir.join("a.txt"), "  "),
            Err(FileManagerError::InvalidName)
        ));
        assert!(dir.join("a.txt").exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
