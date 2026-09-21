//! Drag-and-drop decisions shared by every place files can be dropped: the
//! icon and list views, the sidebar, tab headers and the split pane.

use crate::operations::PendingOp;
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// A drop target that accepts files and is willing to either copy or move
/// them -- the choice is made in `plan_drop`.
pub fn new_file_drop_target() -> gtk::DropTarget {
    gtk::DropTarget::new(
        gdk::FileList::static_type(),
        gdk::DragAction::COPY | gdk::DragAction::MOVE,
    )
}

/// What dropping `value` onto `destination` should do: the files to
/// transfer and whether to copy or move them. `None` if it isn't a list of
/// local files.
pub fn plan_drop(
    target: &gtk::DropTarget,
    value: &glib::Value,
    destination: &Path,
) -> Option<(PendingOp, Vec<PathBuf>)> {
    let file_list = value.get::<gdk::FileList>().ok()?;

    let sources: Vec<PathBuf> = file_list
        .files()
        .iter()
        .filter_map(|file| file.path())
        .collect();

    if sources.is_empty() {
        return None;
    }

    // What the drag has settled on. A modifier held down (Ctrl = copy,
    // Shift = move) narrows it to one action; otherwise both are offered.
    let offered = target
        .current_drop()
        .map(|drop| drop.actions())
        .unwrap_or(gdk::DragAction::COPY | gdk::DragAction::MOVE);

    Some((choose_operation(offered, &sources, destination), sources))
}

/// Copy or move? If the user forced one, that; otherwise -- like most file
/// managers -- move within one filesystem (instant, and what dragging a file
/// into a folder usually means) but copy across filesystems (so dragging
/// from a USB stick doesn't quietly empty it).
pub fn choose_operation(
    offered: gdk::DragAction,
    sources: &[PathBuf],
    destination: &Path,
) -> PendingOp {
    let can_copy = offered.contains(gdk::DragAction::COPY);
    let can_move = offered.contains(gdk::DragAction::MOVE);

    match (can_copy, can_move) {
        (false, true) => PendingOp::Move,
        (true, false) => PendingOp::Copy,
        _ => {
            if sources
                .iter()
                .all(|source| same_filesystem(source, destination))
            {
                PendingOp::Move
            } else {
                PendingOp::Copy
            }
        }
    }
}

/// Are both paths on the same filesystem? (When that can't be worked out,
/// "no": the safe default is a copy.)
fn same_filesystem(a: &Path, b: &Path) -> bool {
    match (fs::symlink_metadata(a), fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::scratch_dir;

    fn is_move(operation: PendingOp) -> bool {
        matches!(operation, PendingOp::Move)
    }

    #[test]
    fn a_forced_action_wins() {
        let dir = scratch_dir("dnd-forced");
        let file = dir.join("f");
        fs::write(&file, "x").unwrap();

        // Same filesystem, which would default to move -- but Ctrl was held.
        assert!(!is_move(choose_operation(
            gdk::DragAction::COPY,
            &[file.clone()],
            &dir
        )));
        assert!(is_move(choose_operation(gdk::DragAction::MOVE, &[file], &dir)));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn with_no_modifier_the_same_filesystem_moves() {
        let dir = scratch_dir("dnd-same-fs");
        let file = dir.join("f");
        fs::write(&file, "x").unwrap();
        fs::create_dir(dir.join("dest")).unwrap();

        let both = gdk::DragAction::COPY | gdk::DragAction::MOVE;

        assert!(is_move(choose_operation(both, &[file], &dir.join("dest"))));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn with_no_modifier_a_different_filesystem_copies() {
        let dir = scratch_dir("dnd-other-fs");
        let both = gdk::DragAction::COPY | gdk::DragAction::MOVE;

        // /proc is its own filesystem, so this stands in for a USB stick.
        assert!(!is_move(choose_operation(
            both,
            &[PathBuf::from("/proc/self")],
            &dir
        )));

        // Something that can't be inspected at all is copied, not moved.
        assert!(!is_move(choose_operation(
            both,
            &[PathBuf::from("/no/such/thing")],
            &dir
        )));

        let _ = fs::remove_dir_all(&dir);
    }
}
