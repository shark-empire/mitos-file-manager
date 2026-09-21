use gtk::prelude::*;
use gtk::{ApplicationWindow, Entry, Label};

// `gtk::Dialog` / `DialogBuilder` / `DialogExt::{add_button, content_area,
// connect_response}` are all deprecated since GTK 4.10. Per the GTK team's
// own "Preparing for GTK 5" migration notes, there's no direct
// replacement widget -- "the recommended replacement is to just create
// your own window and add buttons as required" -- so every dialog in this
// file is a plain `gtk::Window` with a content box and a button row,
// built via `build_dialog`/`dialog_button` below. Each button gets its
// own `connect_clicked` handler instead of one `connect_response` keyed
// off a `ResponseType`.

/// The pieces of a `build_dialog` window a caller fills in: `content` for
/// whatever the dialog is asking about (a label, an entry, ...), and
/// `button_row` to add buttons to via `dialog_button`.
pub(crate) struct DialogScaffold {
    pub(crate) window: gtk::Window,
    pub(crate) content: gtk::Box,
    pub(crate) button_row: gtk::Box,
}

pub(crate) fn build_dialog(parent: &impl IsA<gtk::Window>, title: &str) -> DialogScaffold {
    let window = gtk::Window::builder()
        .title(title)
        .transient_for(parent)
        .modal(true)
        .resizable(false)
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let button_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    button_row.set_halign(gtk::Align::End);
    button_row.set_margin_start(12);
    button_row.set_margin_end(12);
    button_row.set_margin_bottom(12);

    root.append(&content);
    root.append(&button_row);
    window.set_child(Some(&root));

    DialogScaffold {
        window,
        content,
        button_row,
    }
}

/// Add a button to a scaffold's button row and return it for the caller
/// to `.connect_clicked()`.
pub(crate) fn dialog_button(row: &gtk::Box, label: &str) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    row.append(&button);
    button
}

pub fn show_text_dialog<F>(
    parent: &ApplicationWindow,
    title: &str,
    initial: &str,
    ok_label: &str,
    on_accept: F,
) where
    F: Fn(String) + 'static,
{
    let dialog = build_dialog(parent, title);

    let entry = Entry::new();
    entry.set_text(initial);
    dialog.content.append(&entry);

    let cancel_btn = dialog_button(&dialog.button_row, "Cancel");
    let ok_btn = dialog_button(&dialog.button_row, ok_label);
    ok_btn.add_css_class("suggested-action");

    {
        let window = dialog.window.clone();
        cancel_btn.connect_clicked(move |_| window.close());
    }

    {
        let window = dialog.window.clone();
        ok_btn.connect_clicked(move |_| {
            let text = entry.text().to_string();
            on_accept(text.trim().to_string());
            window.close();
        });
    }

    dialog.window.present();
}

/// Ask a yes/no question without blocking: `on_accept` runs only if the user
/// presses the confirm button, and Cancel (or closing the window) does
/// nothing. `destructive` paints the confirm button red -- for deleting
/// things, where the safe answer is the one that should look calm.
///
/// This deliberately doesn't spin a nested main loop (unlike
/// `choose_conflict_policy`): it's called from inside signal handlers and
/// async callbacks, where re-entering the main loop is asking for trouble.
pub fn confirm_then<F>(
    parent: &impl IsA<gtk::Window>,
    title: &str,
    message: &str,
    accept_label: &str,
    destructive: bool,
    on_accept: F,
) where
    F: Fn() + 'static,
{
    let dialog = build_dialog(parent, title);

    let label = Label::new(Some(message));
    label.set_wrap(true);
    label.set_max_width_chars(60);
    label.set_halign(gtk::Align::Start);
    dialog.content.append(&label);

    let cancel_btn = dialog_button(&dialog.button_row, "Cancel");
    let accept_btn = dialog_button(&dialog.button_row, accept_label);
    accept_btn.add_css_class(if destructive {
        "destructive-action"
    } else {
        "suggested-action"
    });

    {
        let window = dialog.window.clone();
        cancel_btn.connect_clicked(move |_| window.close());
    }

    {
        let window = dialog.window.clone();
        accept_btn.connect_clicked(move |_| {
            window.close();
            on_accept();
        });
    }

    // Enter shouldn't confirm a destructive action by accident, so only a
    // non-destructive dialog makes its confirm button the default.
    if !destructive {
        dialog.window.set_default_widget(Some(&accept_btn));
    }

    dialog.window.present();
}

pub fn show_error(parent: &impl IsA<gtk::Window>, message: &str) {
    let dialog = build_dialog(parent, "Error");

    let label = Label::new(Some(message));
    label.set_wrap(true);
    dialog.content.append(&label);

    let ok_btn = dialog_button(&dialog.button_row, "OK");

    let window = dialog.window.clone();
    ok_btn.connect_clicked(move |_| window.close());

    dialog.window.present();
}

pub fn show_info(parent: &ApplicationWindow, title: &str, message: &str) {
    let dialog = build_dialog(parent, title);

    let label = Label::new(Some(message));
    label.set_wrap(true);
    dialog.content.append(&label);

    let ok_btn = dialog_button(&dialog.button_row, "OK");

    let window = dialog.window.clone();
    ok_btn.connect_clicked(move |_| window.close());

    dialog.window.present();
}

pub fn choose_conflict_policy(
    parent: &ApplicationWindow,
    conflict_count: usize,
) -> Option<crate::operations::jobs::ConflictPolicy> {
    use crate::operations::jobs::ConflictPolicy;
    use gtk::glib;
    use std::cell::Cell;
    use std::rc::Rc;

    let dialog = build_dialog(parent, "File Conflict");

    let noun = if conflict_count == 1 { "file" } else { "files" };

    let label = Label::new(Some(&format!(
        "{conflict_count} {noun} already {} in the destination.\n\nWhat should MITOS Files do? This choice applies to all of them.",
        if conflict_count == 1 { "exists" } else { "exist" }
    )));
    label.set_wrap(true);
    dialog.content.append(&label);

    let cancel_btn = dialog_button(&dialog.button_row, "Cancel");
    let skip_btn = dialog_button(&dialog.button_row, "Skip Existing");
    let replace_btn = dialog_button(&dialog.button_row, "Replace");
    let keep_both_btn = dialog_button(&dialog.button_row, "Keep Both");
    keep_both_btn.add_css_class("suggested-action");

    let loop_ = glib::MainLoop::new(None, false);
    let result: Rc<Cell<Option<ConflictPolicy>>> = Rc::new(Cell::new(None));
    // Guards against responding twice: closing the window from inside a
    // button handler also fires `connect_close_request` below, which
    // would otherwise overwrite an already-chosen result with `None`.
    let responded = Rc::new(Cell::new(false));

    let respond: Rc<dyn Fn(Option<ConflictPolicy>)> = {
        let result = result.clone();
        let responded = responded.clone();
        let loop_ = loop_.clone();
        let window = dialog.window.clone();

        Rc::new(move |policy: Option<ConflictPolicy>| {
            if responded.replace(true) {
                return;
            }
            result.set(policy);
            loop_.quit();
            window.close();
        })
    };

    {
        let respond = respond.clone();
        cancel_btn.connect_clicked(move |_| respond(None));
    }
    {
        let respond = respond.clone();
        skip_btn.connect_clicked(move |_| respond(Some(ConflictPolicy::SkipExisting)));
    }
    {
        let respond = respond.clone();
        replace_btn.connect_clicked(move |_| respond(Some(ConflictPolicy::Replace)));
    }
    {
        let respond = respond.clone();
        keep_both_btn.connect_clicked(move |_| respond(Some(ConflictPolicy::KeepBoth)));
    }
    {
        let respond = respond.clone();
        dialog.window.connect_close_request(move |_| {
            respond(None);
            glib::Propagation::Proceed
        });
    }

    dialog.window.present();
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

    let dialog = build_dialog(parent, "Connect to Server");

    let label = Label::new(Some(
        "Enter a network address:\nsmb://server/share · sftp://user@host/path · ftp://host/path",
    ));
    label.set_wrap(true);
    label.set_halign(gtk::Align::Start);

    let entry = Entry::new();
    entry.set_placeholder_text(Some("smb://server/share"));
    entry.set_activates_default(true);

    dialog.content.append(&label);
    dialog.content.append(&entry);

    // Servers connected to before: one click fills the box.
    let recent_servers = crate::navigation::servers::load();

    if !recent_servers.is_empty() {
        let recent_label = Label::new(Some("Recent servers"));
        recent_label.set_halign(gtk::Align::Start);
        recent_label.add_css_class("dim-label");
        dialog.content.append(&recent_label);

        for uri in recent_servers {
            let button = gtk::Button::with_label(&uri);
            button.set_has_frame(false);
            button.set_halign(gtk::Align::Start);

            let entry = entry.clone();

            button.connect_clicked(move |clicked| {
                if let Some(label) = clicked.label() {
                    entry.set_text(&label);
                }
            });

            dialog.content.append(&button);
        }
    }

    let cancel_btn = dialog_button(&dialog.button_row, "Cancel");
    let connect_btn = dialog_button(&dialog.button_row, "Connect");
    connect_btn.add_css_class("suggested-action");
    dialog.window.set_default_widget(Some(&connect_btn));

    {
        let window = dialog.window.clone();
        cancel_btn.connect_clicked(move |_| window.close());
    }

    {
        let window = dialog.window.clone();
        let parent = parent.clone();

        connect_btn.connect_clicked(move |_| {
            let uri = entry.text().trim().to_string();
            window.close();

            if uri.is_empty() {
                return;
            }

            let uri_to_remember = uri.clone();
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
                    // Already being mounted (e.g. a second "Connect" while
                    // the first is still in flight) isn't a real failure
                    // -- fall through and try to resolve a local path
                    // regardless.
                    if let Err(err) = result {
                        if !err.matches(gio::IOErrorEnum::AlreadyMounted) {
                            show_error(&parent_for_error, &format!("Couldn't connect: {err}"));
                            return;
                        }
                    }

                    // It worked (or was already mounted): keep the address for
                    // next time, minus any password.
                    crate::navigation::servers::remember(&uri_to_remember);

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
    }

    dialog.window.present();
}
