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

    let label = Label::new(Some(
        "Some files already exist in the destination.\n\nWhat should MITOS Files do?",
    ));

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
