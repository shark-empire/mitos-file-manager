use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// How much is in a folder (or a whole selection): total bytes, plus how many
/// files and subfolders that's spread over.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FolderSize {
    /// Sum of the sizes of every file, in bytes. A symlink counts as the few
    /// bytes it takes up itself, never as whatever it points at.
    pub bytes: u64,
    pub files: u64,
    pub folders: u64,
}

/// Total up everything under `path` (a single file just reports its own
/// size). `None` means `cancel` was raised part-way through, so there's no
/// trustworthy answer; anything unreadable (a folder without permission, a
/// broken link) is skipped rather than aborting the whole count.
///
/// This walks the tree with an explicit stack and never collects the paths
/// it visits, so memory stays flat no matter how large the tree is, and it
/// does not follow symlinked folders, so a link loop can't run forever.
pub fn calculate_folder_size(path: &Path, cancel: &AtomicBool) -> Option<FolderSize> {
    // The top-level path is followed (asking for the size of a symlink to a
    // folder means the folder); nothing below it is.
    let Ok(root) = fs::metadata(path) else {
        return Some(FolderSize::default());
    };

    if !root.is_dir() {
        return Some(FolderSize {
            bytes: root.len(),
            files: 1,
            folders: 0,
        });
    }

    let mut total = FolderSize::default();
    let mut pending: Vec<PathBuf> = vec![path.to_path_buf()];

    while let Some(dir) = pending.pop() {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }

        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };

            if file_type.is_dir() {
                total.folders += 1;
                pending.push(entry.path());
            } else {
                total.files += 1;

                // `DirEntry::metadata` doesn't traverse symlinks.
                if let Ok(metadata) = entry.metadata() {
                    total.bytes += metadata.len();
                }
            }
        }
    }

    Some(total)
}

/// Run `calculate_folder_size` over every path in `paths` on a background
/// thread and deliver the combined total through the returned channel, so a
/// dialog can show "Calculating..." right away and fill the number in when
/// it's ready. If `cancel` is raised the thread stops and the channel simply
/// closes without a value.
pub fn spawn_folder_size_job(
    paths: Vec<PathBuf>,
    cancel: Arc<AtomicBool>,
) -> async_channel::Receiver<FolderSize> {
    let (sender, receiver) = async_channel::bounded(1);

    std::thread::spawn(move || {
        let mut total = FolderSize::default();

        for path in &paths {
            let Some(part) = calculate_folder_size(path, &cancel) else {
                return;
            };

            total.bytes += part.bytes;
            total.files += part.files;
            total.folders += part.folders;
        }

        let _ = sender.send_blocking(total);
    });

    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::scratch_dir;

    #[test]
    fn totals_files_and_folders_without_following_symlinks() {
        let dir = scratch_dir("traversal-totals");
        fs::write(dir.join("a.bin"), vec![0u8; 100]).unwrap();
        fs::create_dir(dir.join("sub")).unwrap();
        fs::write(dir.join("sub").join("b.bin"), vec![0u8; 50]).unwrap();

        // A link back up to the root: following it would never finish.
        std::os::unix::fs::symlink(&dir, dir.join("sub").join("loop")).unwrap();

        let size = calculate_folder_size(&dir, &AtomicBool::new(false)).unwrap();

        // a.bin, b.bin and the link itself count as files; `sub` is the folder.
        assert_eq!(size.files, 3);
        assert_eq!(size.folders, 1);
        assert!(size.bytes >= 150);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_single_file_reports_its_own_size() {
        let dir = scratch_dir("traversal-file");
        fs::write(dir.join("one.bin"), vec![0u8; 42]).unwrap();

        let size = calculate_folder_size(&dir.join("one.bin"), &AtomicBool::new(false)).unwrap();

        assert_eq!(size.bytes, 42);
        assert_eq!(size.files, 1);
        assert_eq!(size.folders, 0);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_raised_cancel_flag_gives_no_answer() {
        let dir = scratch_dir("traversal-cancel");
        fs::write(dir.join("a.bin"), vec![0u8; 10]).unwrap();

        assert!(calculate_folder_size(&dir, &AtomicBool::new(true)).is_none());

        let _ = fs::remove_dir_all(&dir);
    }
}
