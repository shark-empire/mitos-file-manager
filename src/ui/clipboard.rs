//! The system clipboard, so Copy / Cut / Paste work between this app and
//! any other -- not just inside one window. (Before this, "the clipboard"
//! was a private list in memory: copy a file here, switch to another file
//! manager, and there was nothing to paste.)

use crate::operations::PendingOp;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::path::{Path, PathBuf};

/// Put `paths` on the clipboard, in every format other programs look for:
///
/// * a file list (`text/uri-list`) -- what file managers and most GTK/Qt
///   apps paste from;
/// * `x-special/gnome-copied-files` -- "copy" or "cut" followed by the URIs,
///   which is how Nautilus, Thunar and PCManFM tell a cut from a copy;
/// * plain text of the paths, so pasting into a terminal or editor works.
pub fn set_files(widget: &impl IsA<gtk::Widget>, operation: PendingOp, paths: &[PathBuf]) {
    if paths.is_empty() {
        return;
    }

    let files: Vec<gio::File> = paths.iter().map(|path| gio::File::for_path(path)).collect();

    let file_list = gdk::FileList::from_array(&files);
    let list_provider = gdk::ContentProvider::for_value(&file_list.to_value());

    let gnome_provider = gdk::ContentProvider::for_bytes(
        "x-special/gnome-copied-files",
        &glib::Bytes::from(gnome_copied_files(operation, &files).as_bytes()),
    );

    let text = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let text_provider = gdk::ContentProvider::for_value(&text.to_value());

    let union = gdk::ContentProvider::new_union(&[list_provider, gnome_provider, text_provider]);

    let _ = widget.clipboard().set_content(Some(&union));
}

/// The body of an `x-special/gnome-copied-files` clipboard entry.
fn gnome_copied_files(operation: PendingOp, files: &[gio::File]) -> String {
    let verb = match operation {
        PendingOp::Copy => "copy",
        PendingOp::Move => "cut",
    };

    let mut payload = String::from(verb);

    for file in files {
        payload.push('\n');
        payload.push_str(&file.uri());
    }

    payload
}

/// Put plain text (a path, say) on the clipboard.
pub fn set_text(widget: &impl IsA<gtk::Widget>, text: &str) {
    widget.clipboard().set_text(text);
}

/// Empty the clipboard -- after a "cut" has been pasted, its files have
/// moved and there's nothing left to paste.
pub fn clear(widget: &impl IsA<gtk::Widget>) {
    let _ = widget
        .clipboard()
        .set_content(None::<&gdk::ContentProvider>);
}

/// Read a list of files off the clipboard. `done` is called (on the GTK
/// thread, a moment later) with the paths, or an empty list if what's on
/// the clipboard isn't files -- reading a clipboard is asynchronous, since
/// the program that owns it may have to produce the data first.
pub fn read_files(widget: &impl IsA<gtk::Widget>, done: impl FnOnce(Vec<PathBuf>) + 'static) {
    let clipboard = widget.clipboard();

    glib::MainContext::default().spawn_local(async move {
        let paths: Vec<PathBuf> = match clipboard
            .read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT)
            .await
        {
            Ok(value) => value
                .get::<gdk::FileList>()
                .map(|list| list.files().iter().filter_map(|file| file.path()).collect())
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };

        done(paths);
    });
}

/// Do `a` and `b` name the same set of paths, regardless of order? Used to
/// tell "the clipboard still holds what *we* copied" (so a remembered
/// cut-vs-copy applies) from "another program has since put something else
/// there".
pub fn same_files(a: &[PathBuf], b: &[PathBuf]) -> bool {
    let mut left: Vec<&Path> = a.iter().map(|path| path.as_path()).collect();
    let mut right: Vec<&Path> = b.iter().map(|path| path.as_path()).collect();

    left.sort();
    right.sort();

    left == right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_files_in_any_order_match() {
        let a = vec![PathBuf::from("/x/one"), PathBuf::from("/x/two")];
        let b = vec![PathBuf::from("/x/two"), PathBuf::from("/x/one")];

        assert!(same_files(&a, &b));
        assert!(same_files(&[], &[]));
    }

    #[test]
    fn different_files_do_not_match() {
        let a = vec![PathBuf::from("/x/one")];

        assert!(!same_files(&a, &[PathBuf::from("/x/other")]));
        assert!(!same_files(&a, &[]));
        assert!(!same_files(
            &a,
            &[PathBuf::from("/x/one"), PathBuf::from("/x/two")]
        ));
    }

    #[test]
    fn gnome_copied_files_names_the_operation_then_each_uri() {
        let files = vec![
            gio::File::for_path("/tmp/a b.txt"),
            gio::File::for_path("/tmp/c.txt"),
        ];

        assert_eq!(
            gnome_copied_files(PendingOp::Copy, &files),
            "copy\nfile:///tmp/a%20b.txt\nfile:///tmp/c.txt"
        );
        assert!(gnome_copied_files(PendingOp::Move, &files).starts_with("cut\n"));
    }
}
