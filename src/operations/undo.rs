//! Reversing the file operations this app performs -- Ctrl+Z / Ctrl+Shift+Z.
//!
//! Scope, stated plainly: an action is here because it reverses cleanly and
//! safely, full stop. **Permanent deletion is never undoable** -- that's
//! the entire point of the "this can't be undone" it's confirmed with.
//! Compress and Extract aren't (yet) either, nor is anything done through
//! *Retry as Administrator* (root's changes aren't this list's business).
//!
//! `undo()` and `redo()` are the two entry points, and they're each other's
//! mirror image: both take an `UndoableAction`, both do real (possibly
//! slow -- a big move, say) filesystem work, so both are meant to be called
//! from a background thread, and both hand back an `UndoResult` carrying a
//! human-readable outcome plus the *new* inverse action to push onto the
//! other stack. That inverse is usually just the same action cloned back
//! (undo a rename, and redoing it is "do that exact rename again"), but not
//! always: restoring a trashed item and re-trashing it produces a new
//! `.trashinfo` each time, so `redo()` on a `Trashed` action rebuilds fresh
//! `TrashItem`s for whatever it hands back, rather than replaying stale
//! ones that no longer point at anything.

use crate::error::FileManagerError;
use crate::filesystem::protection;
use crate::filesystem::trash::{self, TrashItem};
use crate::operations::jobs::JobMessage;
use crate::operations::{self, batch_rename, copy, move_op, occupied, PendingOp};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Not `Debug`: `TrashItem` (inside `Trashed`) doesn't derive it, and
/// nothing here needs to print an action -- `forward_description()` is the
/// human-readable form everything else uses.
#[derive(Clone)]
pub enum UndoableAction {
    Renamed {
        from: PathBuf,
        to: PathBuf,
    },
    CreatedFolder {
        path: PathBuf,
    },
    CreatedFile {
        path: PathBuf,
    },
    /// (link path, what it points at) -- a `Vec` because Create Link can
    /// be used on a multi-item selection.
    Linked {
        links: Vec<(PathBuf, PathBuf)>,
    },
    Trashed {
        items: Vec<TrashItem>,
    },
    Pasted {
        operation: PendingOp,
        /// (source, destination) exactly as written -- not the request, the
        /// result: for a `KeepBoth` conflict this is the "name (1)" path
        /// that was actually used, which is what undo must delete/move.
        pairs: Vec<(PathBuf, PathBuf)>,
    },
    BatchRenamed {
        renames: Vec<(PathBuf, PathBuf)>,
    },
}

impl UndoableAction {
    /// A short, human description for the confirmation the app shows after
    /// a *redo* -- i.e. "what this action does when performed forwards".
    /// (`undo()`/`redo()` build the message shown right after acting from
    /// their own result, which is more specific; this is for anything that
    /// wants to describe a pending action, such as a future "Redo: ..."
    /// menu label.)
    pub fn forward_description(&self) -> String {
        match self {
            UndoableAction::Renamed { to, .. } => format!("Rename to \"{}\"", display_name(to)),
            UndoableAction::CreatedFolder { path } => {
                format!("Create folder \"{}\"", display_name(path))
            }
            UndoableAction::CreatedFile { path } => {
                format!("Create file \"{}\"", display_name(path))
            }
            UndoableAction::Linked { links } => describe_count("Create link", links.len()),
            UndoableAction::Trashed { items } => describe_count("Trash", items.len()),
            UndoableAction::Pasted { operation, pairs } => describe_count(
                match operation {
                    PendingOp::Copy => "Copy",
                    PendingOp::Move => "Move",
                },
                pairs.len(),
            ),
            UndoableAction::BatchRenamed { renames } => describe_count("Rename", renames.len()),
        }
    }
}

fn display_name(path: &PathBuf) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn describe_count(verb: &str, count: usize) -> String {
    if count == 1 {
        format!("{verb} 1 item")
    } else {
        format!("{verb} {count} items")
    }
}

pub struct UndoResult {
    pub message: String,
    /// What to push onto the *other* stack. `None` only when literally
    /// nothing happened (so there's nothing to reverse back).
    pub inverse: Option<UndoableAction>,
}

fn ok(message: impl Into<String>, inverse: UndoableAction) -> Result<UndoResult, String> {
    Ok(UndoResult {
        message: message.into(),
        inverse: Some(inverse),
    })
}

/// Reverse `action`. Meant to be called off the GTK thread -- a large move
/// or a folder full of trashed items can take a moment.
pub fn undo(action: &UndoableAction) -> Result<UndoResult, String> {
    match action {
        UndoableAction::Renamed { from, to } => {
            rename_back(to, from)?;
            ok(
                format!("Undid rename \u{2014} back to \"{}\"", display_name(from)),
                action.clone(),
            )
        }

        UndoableAction::CreatedFolder { path } => {
            remove_if_still_empty_folder(path)?;
            ok(
                format!("Undid: removed \"{}\"", display_name(path)),
                action.clone(),
            )
        }

        UndoableAction::CreatedFile { path } => {
            remove_if_still_empty_file(path)?;
            ok(
                format!("Undid: removed \"{}\"", display_name(path)),
                action.clone(),
            )
        }

        UndoableAction::Linked { links } => {
            let (removed, failures) =
                delete_all(links.iter().map(|(link_path, _)| link_path.clone()));

            if removed == 0 && !links.is_empty() {
                return Err(too_stale_to_undo("remove the link", &failures));
            }

            ok(
                partial_message("Removed link", removed, links.len()),
                action.clone(),
            )
        }

        UndoableAction::Trashed { items } => {
            let (restored, failures) = restore_all(items);

            if restored.is_empty() {
                return Err(too_stale_to_undo("restore", &failures));
            }

            Ok(UndoResult {
                message: partial_message("Restored", restored.len(), items.len()),
                inverse: Some(UndoableAction::Trashed { items: restored }),
            })
        }

        UndoableAction::Pasted { operation, pairs } => match operation {
            PendingOp::Copy => {
                let (undone, failures) = delete_all(pairs.iter().map(|(_, dest)| dest.clone()));

                if undone == 0 && !pairs.is_empty() {
                    return Err(too_stale_to_undo("remove the copy of", &failures));
                }

                Ok(UndoResult {
                    message: partial_message("Removed", undone, pairs.len()),
                    inverse: Some(action.clone()),
                })
            }
            PendingOp::Move => {
                let (undone, failures) = move_all(pairs.iter().map(|(src, dest)| (dest.clone(), src.clone())));

                if undone == 0 && !pairs.is_empty() {
                    return Err(too_stale_to_undo("move back", &failures));
                }

                Ok(UndoResult {
                    message: partial_message("Moved back", undone, pairs.len()),
                    inverse: Some(action.clone()),
                })
            }
        },

        UndoableAction::BatchRenamed { renames } => {
            let reversed: Vec<(PathBuf, PathBuf)> = renames
                .iter()
                .map(|(from, to)| (to.clone(), from.clone()))
                .collect();

            run_reversible_batch(&reversed).map_err(|err| format!("Couldn't undo the rename: {err}"))?;

            ok(
                format!("Undid renaming {} items", renames.len()),
                action.clone(),
            )
        }
    }
}

/// Replay `action`. The mirror image of `undo` -- see the module docs for
/// why the returned inverse isn't always just `action.clone()`.
pub fn redo(action: &UndoableAction) -> Result<UndoResult, String> {
    match action {
        UndoableAction::Renamed { from, to } => {
            rename_back(from, to)?;
            ok(
                format!("Redid rename \u{2014} to \"{}\"", display_name(to)),
                action.clone(),
            )
        }

        UndoableAction::CreatedFolder { path } => {
            fs::create_dir(path).map_err(|err| describe_io_error("create the folder again", err))?;
            ok(
                format!("Redid: created \"{}\"", display_name(path)),
                action.clone(),
            )
        }

        UndoableAction::CreatedFile { path } => {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|err| describe_io_error("create the file again", err))?;
            ok(
                format!("Redid: created \"{}\"", display_name(path)),
                action.clone(),
            )
        }

        UndoableAction::Linked { links } => {
            let mut created = 0;
            let mut failures = Vec::new();

            for (link_path, target) in links {
                if occupied(link_path) {
                    failures.push(display_name(link_path));
                } else if std::os::unix::fs::symlink(target, link_path).is_ok() {
                    created += 1;
                } else {
                    failures.push(display_name(link_path));
                }
            }

            if created == 0 && !links.is_empty() {
                return Err(too_stale_to_undo("recreate the link", &failures));
            }

            ok(
                partial_message("Created link", created, links.len()),
                action.clone(),
            )
        }

        UndoableAction::Trashed { items } => {
            let mut fresh = Vec::with_capacity(items.len());
            let mut failures = Vec::new();

            for item in items {
                match re_trash(&item.original_path) {
                    Some(new_item) => fresh.push(new_item),
                    None => failures.push(display_name(&item.original_path)),
                }
            }

            if fresh.is_empty() {
                return Err(too_stale_to_undo("trash", &failures));
            }

            Ok(UndoResult {
                message: partial_message("Trashed", fresh.len(), items.len()),
                inverse: Some(UndoableAction::Trashed { items: fresh }),
            })
        }

        UndoableAction::Pasted { operation, pairs } => match operation {
            PendingOp::Copy => {
                let (redone, failures) = copy_all(pairs.iter().cloned());

                if redone == 0 && !pairs.is_empty() {
                    return Err(too_stale_to_undo("re-copy", &failures));
                }

                Ok(UndoResult {
                    message: partial_message("Copied", redone, pairs.len()),
                    inverse: Some(action.clone()),
                })
            }
            PendingOp::Move => {
                let (redone, failures) = move_all(pairs.iter().cloned());

                if redone == 0 && !pairs.is_empty() {
                    return Err(too_stale_to_undo("move", &failures));
                }

                Ok(UndoResult {
                    message: partial_message("Moved", redone, pairs.len()),
                    inverse: Some(action.clone()),
                })
            }
        },

        UndoableAction::BatchRenamed { renames } => {
            run_reversible_batch(renames).map_err(|err| format!("Couldn't redo the rename: {err}"))?;

            ok(
                format!("Redid renaming {} items", renames.len()),
                action.clone(),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Rename
// ---------------------------------------------------------------------------

/// Move `from` back to `to`, the same way `operations::rename::rename_path`
/// would, but taking two full paths instead of a (path, new base name)
/// pair, since undo already has both ends. Refuses to replace an existing
/// file, and honours protected paths.
fn rename_back(from: &PathBuf, to: &PathBuf) -> Result<(), String> {
    protection::ensure_modifiable(&[from.clone()]).map_err(|err| err.to_string())?;

    if occupied(to) {
        return Err(format!(
            "Can't undo \u{2014} something is already at \"{}\"",
            display_name(to)
        ));
    }

    fs::rename(from, to).map_err(|err| describe_io_error("rename", err))
}

// ---------------------------------------------------------------------------
// Create / Link
// ---------------------------------------------------------------------------

/// Undoing "New Folder" must never eat files the person has since dropped
/// into it -- so this only removes an *empty* folder, and reports plainly
/// when it declines.
fn remove_if_still_empty_folder(path: &PathBuf) -> Result<(), String> {
    match fs::read_dir(path) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Err(format!(
                    "\"{}\" has things in it now, so Undo won't remove it",
                    display_name(path)
                ));
            }
        }
        Err(err) => return Err(describe_io_error("find that folder", err)),
    }

    fs::remove_dir(path).map_err(|err| describe_io_error("remove the folder", err))
}

/// Same idea for "New File": only removed if it's still exactly as empty
/// as New File left it.
fn remove_if_still_empty_file(path: &PathBuf) -> Result<(), String> {
    let metadata = fs::metadata(path).map_err(|err| describe_io_error("find that file", err))?;

    if metadata.len() != 0 {
        return Err(format!(
            "\"{}\" has content now, so Undo won't remove it",
            display_name(path)
        ));
    }

    fs::remove_file(path).map_err(|err| describe_io_error("remove the file", err))
}

fn remove_if_still_a_symlink(path: &PathBuf) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|err| describe_io_error("find that link", err))?;

    if !metadata.file_type().is_symlink() {
        return Err(format!(
            "\"{}\" isn't the link Undo created anymore",
            display_name(path)
        ));
    }

    fs::remove_file(path).map_err(|err| describe_io_error("remove the link", err))
}

// ---------------------------------------------------------------------------
// Trash
// ---------------------------------------------------------------------------

/// Restore every item, returning the ones that actually came back (as fresh
/// `TrashItem`s are no longer meaningful once restored, this just echoes
/// the input for those) and the display names of any that failed.
fn restore_all(items: &[TrashItem]) -> (Vec<TrashItem>, Vec<String>) {
    let mut restored = Vec::new();
    let mut failures = Vec::new();

    for item in items {
        match trash::restore(item) {
            Ok(()) => restored.push(item.clone()),
            Err(_) => failures.push(display_name(&item.original_path)),
        }
    }

    (restored, failures)
}

/// Trash `original_path` again and look up the `TrashItem` that resulted.
fn re_trash(original_path: &PathBuf) -> Option<TrashItem> {
    operations::trash::delete(original_path).ok()?;

    find_recently_trashed(std::slice::from_ref(original_path))
        .into_iter()
        .next()
}

/// Match freshly trashed items back to the original paths that were just
/// sent to the trash, by `original_path` -- since neither the `trash` crate
/// nor the trash job hands back the `.trashinfo` it wrote. Used right after
/// a successful trash (here, and by `main.rs`'s job-completion handler) to
/// build a `Trashed` undo entry out of real `TrashItem`s. A path with no
/// match (trashed, then immediately restored or emptied by something else)
/// is simply left out -- the caller gets undo for whatever's left.
pub fn find_recently_trashed(original_paths: &[PathBuf]) -> Vec<TrashItem> {
    let all = trash::list(&crate::filesystem::mounts::removable_roots());

    original_paths
        .iter()
        .filter_map(|path| {
            all.iter()
                .filter(|candidate| candidate.original_path == *path)
                .max_by_key(|candidate| candidate.deletion_date.clone())
                .cloned()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Paste (copy / move)
// ---------------------------------------------------------------------------

fn delete_all(destinations: impl Iterator<Item = PathBuf>) -> (usize, Vec<String>) {
    let mut done = 0;
    let mut failures = Vec::new();

    for destination in destinations {
        if protection::ensure_modifiable(&[destination.clone()]).is_ok()
            && remove_any(&destination).is_ok()
        {
            done += 1;
        } else {
            failures.push(display_name(&destination));
        }
    }

    (done, failures)
}

fn move_all(pairs: impl Iterator<Item = (PathBuf, PathBuf)>) -> (usize, Vec<String>) {
    let mut done = 0;
    let mut failures = Vec::new();

    for (source, destination) in pairs {
        // The slot this is moving back into may have been reused for
        // something else since -- don't clobber it.
        if !occupied(&source) && move_op::move_path(&source, &destination).is_ok() {
            done += 1;
        } else {
            failures.push(display_name(&source));
        }
    }

    (done, failures)
}

fn copy_all(pairs: impl Iterator<Item = (PathBuf, PathBuf)>) -> (usize, Vec<String>) {
    let mut done = 0;
    let mut failures = Vec::new();

    for (source, destination) in pairs {
        if !occupied(&destination)
            && fs::symlink_metadata(&source).is_ok()
            && copy::copy_path(&source, &destination).is_ok()
        {
            done += 1;
        } else {
            failures.push(display_name(&source));
        }
    }

    (done, failures)
}

fn remove_any(path: &PathBuf) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;

    if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

// ---------------------------------------------------------------------------
// Batch rename
// ---------------------------------------------------------------------------

/// Run `pairs` through the same staged, rollback-safe engine the batch
/// rename job uses -- correct even for a set of renames that swap or chain
/// names. `sender` only matters to a live progress dialog, which undo/redo
/// don't have, so its messages are simply dropped.
fn run_reversible_batch(pairs: &[(PathBuf, PathBuf)]) -> Result<usize, String> {
    let (sender, _receiver) = async_channel::unbounded::<JobMessage>();

    batch_rename::run_batch_rename(
        pairs,
        &sender,
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    )
}

// ---------------------------------------------------------------------------
// Shared message building
// ---------------------------------------------------------------------------

fn describe_io_error(doing: &str, err: std::io::Error) -> String {
    format!("Couldn't {doing}: {}", FileManagerError::from(err))
}

fn too_stale_to_undo(verb: &str, failures: &[String]) -> String {
    format!(
        "Couldn't {verb} anything \u{2014} too much has changed since ({})",
        failures.join(", ")
    )
}

fn partial_message(verb: &str, done: usize, of: usize) -> String {
    if done == of {
        describe_count(verb, done)
    } else {
        format!("{} ({} of {} \u{2014} the rest had already changed)", describe_count(verb, done), done, of)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::scratch_dir;

    #[test]
    fn rename_undoes_and_redoes() {
        let dir = scratch_dir("undo-rename");
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        fs::write(&a, "hi").unwrap();

        let action = UndoableAction::Renamed { from: a.clone(), to: b.clone() };
        fs::rename(&a, &b).unwrap();

        let undone = undo(&action).unwrap();
        assert!(a.exists() && !b.exists());
        assert_eq!(fs::read_to_string(&a).unwrap(), "hi");

        redo(undone.inverse.as_ref().unwrap()).unwrap();
        assert!(!a.exists() && b.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_undo_refuses_to_clobber_something_already_there() {
        let dir = scratch_dir("undo-rename-clash");
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        fs::write(&b, "renamed").unwrap();
        fs::write(&a, "unrelated, appeared after the rename").unwrap();

        let action = UndoableAction::Renamed { from: a.clone(), to: b.clone() };
        assert!(undo(&action).is_err());
        assert_eq!(fs::read_to_string(&a).unwrap(), "unrelated, appeared after the rename");
        assert_eq!(fs::read_to_string(&b).unwrap(), "renamed");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn created_folder_undo_only_removes_it_while_still_empty() {
        let dir = scratch_dir("undo-created-folder");
        let folder = dir.join("New Folder");
        fs::create_dir(&folder).unwrap();

        let action = UndoableAction::CreatedFolder { path: folder.clone() };

        // Something was added before Undo was pressed: refuse.
        fs::write(folder.join("keep.txt"), "important").unwrap();
        assert!(undo(&action).is_err());
        assert!(folder.exists());

        fs::remove_file(folder.join("keep.txt")).unwrap();
        let result = undo(&action).unwrap();
        assert!(!folder.exists());

        redo(result.inverse.as_ref().unwrap()).unwrap();
        assert!(folder.is_dir());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn created_file_undo_only_removes_it_while_still_empty() {
        let dir = scratch_dir("undo-created-file");
        let file = dir.join("new-file.txt");
        fs::write(&file, "").unwrap();

        let action = UndoableAction::CreatedFile { path: file.clone() };

        fs::write(&file, "typed something").unwrap();
        assert!(undo(&action).is_err());

        fs::write(&file, "").unwrap();
        assert!(undo(&action).is_ok());
        assert!(!file.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn link_undo_and_redo_round_trip() {
        let dir = scratch_dir("undo-link");
        let target = dir.join("target.txt");
        fs::write(&target, "x").unwrap();
        let link_path = dir.join("Link to target.txt");
        std::os::unix::fs::symlink(&target, &link_path).unwrap();

        let action = UndoableAction::Linked {
            links: vec![(link_path.clone(), target.clone())],
        };
        let undone = undo(&action).unwrap();
        assert!(!link_path.exists());

        redo(undone.inverse.as_ref().unwrap()).unwrap();
        assert_eq!(fs::read_link(&link_path).unwrap(), target);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn paste_copy_undo_removes_the_copies_and_redo_makes_them_again() {
        let dir = scratch_dir("undo-paste-copy");
        let src = dir.join("src");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("a.txt"), "hello").unwrap();

        let dest = dir.join("a.txt");
        fs::copy(src.join("a.txt"), &dest).unwrap();

        let action = UndoableAction::Pasted {
            operation: PendingOp::Copy,
            pairs: vec![(src.join("a.txt"), dest.clone())],
        };

        let undone = undo(&action).unwrap();
        assert!(!dest.exists());

        redo(undone.inverse.as_ref().unwrap()).unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "hello");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn paste_move_undo_and_redo_round_trip() {
        let dir = scratch_dir("undo-paste-move");
        let (src_dir, dst_dir) = (dir.join("src"), dir.join("dst"));
        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&dst_dir).unwrap();

        let source = src_dir.join("a.txt");
        let destination = dst_dir.join("a.txt");
        fs::write(&source, "moved").unwrap();
        fs::rename(&source, &destination).unwrap();

        let action = UndoableAction::Pasted {
            operation: PendingOp::Move,
            pairs: vec![(source.clone(), destination.clone())],
        };

        undo(&action).unwrap();
        assert!(source.exists() && !destination.exists());

        redo(&action).unwrap();
        assert!(!source.exists() && destination.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn batch_rename_undo_handles_a_swap() {
        let dir = scratch_dir("undo-batch-swap");
        let (a, b) = (dir.join("a"), dir.join("b"));
        fs::write(&a, "was a").unwrap();
        fs::write(&b, "was b").unwrap();

        // The batch swapped a<->b.
        let action = UndoableAction::BatchRenamed {
            renames: vec![(a.clone(), b.clone()), (b.clone(), a.clone())],
        };

        let result = undo(&action).unwrap();
        assert_eq!(fs::read_to_string(&a).unwrap(), "was a");
        assert_eq!(fs::read_to_string(&b).unwrap(), "was b");

        redo(result.inverse.as_ref().unwrap()).unwrap();
        assert_eq!(fs::read_to_string(&a).unwrap(), "was b");
        assert_eq!(fs::read_to_string(&b).unwrap(), "was a");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn undoing_a_trash_action_with_a_since_emptied_item_reports_failure() {
        let dir = scratch_dir("undo-trash-stale");

        // A `TrashItem` pointing at files that were never really trashed
        // (standing in for "the user emptied the trash since").
        let fake = TrashItem {
            trash_name: "gone".to_string(),
            original_path: dir.join("gone.txt"),
            file_path: dir.join("nonexistent-trash-dir").join("gone.txt"),
            info_path: dir.join("nonexistent-trash-dir").join("gone.txt.trashinfo"),
            location_label: "Home".to_string(),
            deletion_date: None,
        };

        let action = UndoableAction::Trashed { items: vec![fake] };
        assert!(undo(&action).is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn forward_descriptions_name_the_item_or_count() {
        let path = PathBuf::from("/tmp/Report.docx");

        assert_eq!(
            UndoableAction::CreatedFile { path: path.clone() }.forward_description(),
            "Create file \"Report.docx\""
        );

        let renames = UndoableAction::BatchRenamed {
            renames: vec![
                (PathBuf::from("/a"), PathBuf::from("/b")),
                (PathBuf::from("/c"), PathBuf::from("/d")),
            ],
        };
        assert_eq!(renames.forward_description(), "Rename 2 items");
    }
}
