use gtk::prelude::*;
use gtk::{ApplicationWindow, Dialog, Entry, Label, ResponseType};

pub fn show_text_dialog<F>(
    parent: &ApplicationWindow,
    title: &str,
    initial: &str,
    ok_label: &str,
    on_accept: F,
) where
    F: Fn(String) + 'static,
{
    let dialog = Dialog::builder()
        .title(title)
        .transient_for(parent)
        .modal(true)
        .build();

    dialog.add_button("Cancel", ResponseType::Cancel);
    dialog.add_button(ok_label, ResponseType::Accept);

    let content = dialog.content_area();

    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let entry = Entry::new();
    entry.set_text(initial);

    content.append(&entry);

    dialog.connect_response(move |dialog, response| {
        if response == ResponseType::Accept {
            let text = entry.text().to_string();
            on_accept(text.trim().to_string());
        }

        dialog.close();
    });

    dialog.present();
}

pub fn show_error(parent: &impl IsA<gtk::Window>, message: &str) {
    let dialog = Dialog::builder()
        .title("Error")
        .transient_for(parent)
        .modal(true)
        .build();

    dialog.add_button("OK", ResponseType::Close);

    let label = Label::new(Some(message));
    label.set_wrap(true);

    label.set_margin_top(12);
    label.set_margin_bottom(12);
    label.set_margin_start(12);
    label.set_margin_end(12);

    dialog.content_area().append(&label);

    dialog.connect_response(|dialog, _| {
        dialog.close();
    });

    dialog.present();
}

pub fn show_info(parent: &ApplicationWindow, title: &str, message: &str) {
    let dialog = Dialog::builder()
        .title(title)
        .transient_for(parent)
        .modal(true)
        .build();

    dialog.add_button("OK", ResponseType::Close);

    let label = Label::new(Some(message));
    label.set_wrap(true);

    label.set_margin_top(12);
    label.set_margin_bottom(12);
    label.set_margin_start(12);
    label.set_margin_end(12);

    dialog.content_area().append(&label);

    dialog.connect_response(|dialog, _| {
        dialog.close();
    });

    dialog.present();
}

pub fn choose_conflict_policy(
    parent: &ApplicationWindow,
    conflict_count: usize,
) -> Option<crate::operations::jobs::ConflictPolicy> {
    use crate::operations::jobs::ConflictPolicy;
    use gtk::glib;
    use std::cell::Cell;
    use std::rc::Rc;

    let dialog = Dialog::builder()
        .title("File Conflict")
        .transient_for(parent)
        .modal(true)
        .build();

    dialog.add_button("Cancel", ResponseType::Cancel);
    dialog.add_button("Skip Existing", ResponseType::Reject);
    dialog.add_button("Replace", ResponseType::Yes);
    dialog.add_button("Keep Both", ResponseType::Accept);

    let content = dialog.content_area();

    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let noun = if conflict_count == 1 { "file" } else { "files" };

    let label = Label::new(Some(&format!(
        "{conflict_count} {noun} already {} in the destination.\n\nWhat should MITOS Files do? This choice applies to all of them.",
        if conflict_count == 1 { "exists" } else { "exist" }
    )));

    label.set_wrap(true);
    content.append(&label);

    let loop_ = glib::MainLoop::new(None, false);
    let result = Rc::new(Cell::new(None));

    let result_clone = result.clone();
    let loop_clone = loop_.clone();

    dialog.connect_response(move |dialog, response| {
        let chosen = match response {
            ResponseType::Yes => Some(ConflictPolicy::Replace),
            ResponseType::Accept => Some(ConflictPolicy::KeepBoth),
            ResponseType::Reject => Some(ConflictPolicy::SkipExisting),
            _ => None,
        };

        result_clone.set(chosen);
        dialog.close();
        loop_clone.quit();
    });

    dialog.present();
    loop_.run();

    result.get()
}

/// "Connect to Server" -- prompts for a network address (`smb://`,
/// `sftp://`, `ftp://`, ...) and mounts it via GIO/GVfs. On success,
/// `on_connected` is called on the GTK main thread with the local path
/// GVfs mounted the share at, so the caller can navigate straight there.
/// Already-mounted shares (and anything GVfs mounts locally) then show up
/// in the sidebar automatically via the existing `VolumeMonitor` hookup.
pub fn show_connect_to_server<F>(parent: &ApplicationWindow, on_connected: F)
where
    F: Fn(std::path::PathBuf) + 'static,
{
    use gtk::gio;
    use std::rc::Rc;

    let on_connected = Rc::new(on_connected);

    let dialog = Dialog::builder()
        .title("Connect to Server")
        .transient_for(parent)
        .modal(true)
        .build();

    dialog.add_button("Cancel", ResponseType::Cancel);
    dialog.add_button("Connect", ResponseType::Accept);

    let content = dialog.content_area();

    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);

    let label = Label::new(Some(
        "Enter a network address:\nsmb://server/share · sftp://user@host/path · ftp://host/path",
    ));
    label.set_wrap(true);
    label.set_halign(gtk::Align::Start);

    let entry = Entry::new();
    entry.set_placeholder_text(Some("smb://server/share"));
    entry.set_activates_default(true);

    inner.append(&label);
    inner.append(&entry);
    content.append(&inner);

    dialog.set_default_widget(Some(&entry));

    let parent = parent.clone();

    dialog.connect_response(move |dialog, response| {
        if response != ResponseType::Accept {
            dialog.close();
            return;
        }

        let uri = entry.text().trim().to_string();
        dialog.close();

        if uri.is_empty() {
            return;
        }

        let file = gio::File::for_uri(&uri);
        let file_for_result = file.clone();
        let mount_op = gtk::MountOperation::new(Some(&parent));
        let parent_for_error = parent.clone();
        let on_connected = on_connected.clone();

        file.mount_enclosing_volume(
            gio::MountMountFlags::NONE,
            Some(&mount_op),
            gio::Cancellable::NONE,
            move |result| {
                // Already being mounted (e.g. a second "Connect" while the
                // first is still in flight) isn't a real failure -- fall
                // through and try to resolve a local path regardless.
                if let Err(err) = result {
                    if !err.matches(gio::IOErrorEnum::AlreadyMounted) {
                        show_error(&parent_for_error, &format!("Couldn't connect: {err}"));
                        return;
                    }
                }

                if let Some(path) = file_for_result.path() {
                    on_connected(path);
                } else {
                    show_error(
                        &parent_for_error,
                        "Connected, but MITOS Files couldn't resolve a local path for it.",
                    );
                }
            },
        );
    });

    dialog.present();
}
